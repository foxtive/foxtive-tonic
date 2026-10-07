mod config;

pub use config::{BoxedService, Server, ShutdownSignalHandler};

use foxtive::prelude::AppMessage;
use foxtive::results::AppResult;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use tokio::signal;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tonic::body::Body;
use tonic::codegen::http::Request;
use tonic::transport::Server as TonicServer;
use tonic::transport::server::TcpIncoming;
use tower::util::BoxCloneService;
use tracing::{info, warn};

/// A handle to a server that is already listening.
///
/// Returned by [`Server::spawn`]. The socket is bound before this is handed back,
/// so [`ServerHandle::local_addr`] is meaningful immediately, including for a
/// server started with [`.port(0)`](Server::port).
///
/// Dropping the handle does *not* stop the server: the serving task is detached
/// and keeps running until a shutdown signal fires. Use
/// [`shutdown`](ServerHandle::shutdown) then [`stopped`](ServerHandle::stopped)
/// for a graceful stop.
#[derive(Debug)]
pub struct ServerHandle {
    local_addr: SocketAddr,
    shutdown_tx: watch::Sender<bool>,
    task: JoinHandle<AppResult<()>>,
}

impl ServerHandle {
    /// The address the server is actually listening on.
    ///
    /// Differs from the configured one when the port was `0`.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Request a graceful shutdown and return immediately.
    ///
    /// The listener stops accepting, in-flight requests are allowed to finish, and
    /// the [`on_shutdown`](Server::on_shutdown) handler plus the application
    /// shutdown hooks then run. Await [`stopped`](ServerHandle::stopped) to wait
    /// for that to complete.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Wait for the server to finish draining and stop.
    ///
    /// This does not itself request shutdown; call [`shutdown`](ServerHandle::shutdown)
    /// (or deliver `Ctrl+C`/`SIGTERM`) first, or await it from a task that will.
    pub async fn stopped(self) -> AppResult<()> {
        let Self {
            local_addr: _,
            shutdown_tx,
            task,
        } = self;

        let result = task.await.map_err(|err| AppMessage::Infrastructure {
            message: "Server task panicked or was cancelled".to_string(),
            source: Some(Box::new(err)),
        });

        // Held across the await so consuming the handle is not read as a shutdown request.
        drop(shutdown_tx);

        result?
    }
}

/// Bind, wire up, and serve from a background task.
pub(crate) async fn spawn(mut config: Server) -> AppResult<ServerHandle> {
    init_tracing(&mut config);

    if config.panic_hook {
        foxtive::setup::panic::install(config.app.env());
    }

    let app = config.app.clone();

    if let Some(bootstrap) = config.bootstrap.take() {
        bootstrap(app.clone()).await?;
    }

    // Startup hooks run before the bind, so a failing hook stops the server accepting traffic.
    app.run_startup_hooks().await?;

    let service_config =
        config
            .service_config
            .take()
            .ok_or_else(|| AppMessage::Infrastructure {
                message:
                    "No services configured. Call .configure() to add gRPC services to the server."
                        .to_string(),
                source: None,
            })?;

    let mut routes = service_config();
    if let Some(health) = config.health.take() {
        routes = routes.add_service(health.health_server());
    }
    let routes = routes.prepare();

    let addr = format!("{}:{}", config.host, config.port)
        .parse::<SocketAddr>()
        .map_err(|err| AppMessage::Infrastructure {
            message: format!("Invalid server address: {}:{}", config.host, config.port),
            source: Some(Box::new(err)),
        })?;

    // Bound here, not inside `serve_*`, so address failures (port taken, no permission,
    // malformed host) surface before `on_started` runs and before a `ServerHandle` exists.
    let incoming = TcpIncoming::bind(addr)
        .map(|incoming| {
            // serve_with_incoming* ignores the builder's TCP knobs, so re-apply all four here.
            incoming
                .with_nodelay(Some(config.tcp_nodelay))
                .with_keepalive(config.tcp_keepalive)
                .with_keepalive_interval(config.tcp_keepalive_interval)
                .with_keepalive_retries(config.tcp_keepalive_retries)
        })
        .and_then(|incoming| {
            let local_addr = incoming.local_addr()?;
            Ok((incoming, local_addr))
        })
        .map_err(|err| AppMessage::Infrastructure {
            message: format!("Failed to bind gRPC server to {addr}"),
            source: Some(Box::new(err)),
        })?;

    let (incoming, local_addr) = incoming;
    info!("Starting gRPC server at {local_addr} ...");

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let shutdown = shutdown_future(
        config.shutdown_signal.take(),
        config.on_shutdown.take(),
        shutdown_rx,
    );

    let builder = build_transport(&mut config)?;

    // Boxed either way, so one serve call covers the layered and unlayered paths.
    let routes_svc: BoxedService = box_routes(routes);
    let svc = match config.layer_fn.take() {
        Some(layer_fn) => layer_fn(routes_svc),
        None => routes_svc,
    };

    let task = tokio::spawn(async move {
        let served = builder
            .serve_with_incoming_shutdown(svc, incoming, shutdown)
            .await
            .map_err(|err| AppMessage::Infrastructure {
                message: format!("Server error while serving {local_addr}"),
                source: Some(Box::new(err)),
            });

        // Shutdown hooks run after the drain, so they observe a quiesced process.
        app.run_shutdown_hooks().await;

        served
    });

    if let Some(on_started) = config.on_started.take() {
        tokio::spawn(on_started);
    }

    Ok(ServerHandle {
        local_addr,
        shutdown_tx,
        task,
    })
}

/// Install the global tracing subscriber unless one is already set.
///
/// `init()` panics if a global default exists, so an existing subscriber is kept
/// and the ignored configuration is logged.
fn init_tracing(config: &mut Server) {
    // has_been_set checks without triggering the panicking install.
    if tracing::dispatcher::has_been_set() {
        if config.tracing_config.is_some() {
            warn!(
                "Tracing is already initialised; the configuration passed to Server::tracing was ignored"
            );
        }
        return;
    }

    // has_started_bootstrap skips the default subscriber; explicit tracing is still honoured.
    if config.has_started_bootstrap && config.tracing_config.is_none() {
        return;
    }

    let tracing_config = config.tracing_config.take().unwrap_or_default();
    if let Err(err) = foxtive::setup::trace::init_tracing(tracing_config) {
        // Best-effort: a logging failure must not stop the server from serving.
        warn!("Failed to initialise tracing: {err}");
    }
}

/// Widen `Routes`' `Infallible` error to the boxed error [`BoxedService`] uses.
fn box_routes(routes: tonic::service::Routes) -> BoxedService {
    let mapped = tower::ServiceExt::<Request<Body>>::map_err(routes, Into::into);
    BoxCloneService::new(mapped)
}

/// Apply the transport-level knobs to a tonic server builder.
fn build_transport(config: &mut Server) -> AppResult<TonicServer> {
    let mut builder = TonicServer::builder();

    if let Some(timeout) = config.timeout {
        builder = builder.timeout(timeout);
    }

    if let Some(limit) = config.concurrency_limit {
        builder = builder.concurrency_limit_per_connection(limit);
    }

    if let Some(keepalive) = config.tcp_keepalive {
        builder = builder.tcp_keepalive(Some(keepalive));
    }

    if let Some(interval) = config.tcp_keepalive_interval {
        builder = builder.tcp_keepalive_interval(Some(interval));
    }

    if let Some(retries) = config.tcp_keepalive_retries {
        builder = builder.tcp_keepalive_retries(Some(retries));
    }

    builder = builder.tcp_nodelay(config.tcp_nodelay);

    if let Some(interval) = config.http2_keepalive_interval {
        builder = builder.http2_keepalive_interval(Some(interval));
    }

    if let Some(timeout) = config.http2_keepalive_timeout {
        builder = builder.http2_keepalive_timeout(Some(timeout));
    }

    if let Some(enabled) = config.http2_adaptive_window {
        builder = builder.http2_adaptive_window(Some(enabled));
    }

    if let Some(max) = config.max_concurrent_streams {
        builder = builder.max_concurrent_streams(max);
    }

    if let Some(size) = config.initial_stream_window_size {
        builder = builder.initial_stream_window_size(size);
    }

    if let Some(size) = config.initial_connection_window_size {
        builder = builder.initial_connection_window_size(size);
    }

    if let Some(age) = config.max_connection_age {
        builder = builder.max_connection_age(age);
    }

    if let Some(grace) = config.max_connection_age_grace {
        builder = builder.max_connection_age_grace(grace);
    }

    builder = builder.load_shed(config.load_shed);

    #[cfg(feature = "tls")]
    if let Some(tls_config) = config.tls_config.take() {
        // Tonic validates the material here; bad TLS is an error, not a plaintext fallback.
        builder = builder
            .tls_config(tls_config)
            .map_err(|err| AppMessage::Infrastructure {
                message: "Failed to configure TLS".to_string(),
                source: Some(Box::new(err)),
            })?;
    }

    Ok(builder)
}

/// The future that tells tonic to stop accepting and start draining.
///
/// Fires on [`ServerHandle::shutdown`], a custom
/// [`shutdown_signal`](Server::shutdown_signal), or `Ctrl+C`/`SIGTERM`, whichever
/// comes first, then runs the [`on_shutdown`](Server::on_shutdown) handler.
fn shutdown_future(
    custom: Option<ShutdownSignalHandler>,
    on_shutdown: Option<ShutdownSignalHandler>,
    shutdown_rx: watch::Receiver<bool>,
) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
    let requested = async move {
        tokio::select! {
            _ = wait_for_handle_shutdown(shutdown_rx) => {
                info!("Shutdown requested via ServerHandle");
            }
            _ = wait_for_signal(custom) => {}
        }
    };

    Box::pin(async move {
        requested.await;
        warn!("Shutdown requested. Draining connections...");

        if let Some(handler) = on_shutdown {
            handler.await;
        }
    })
}

/// The configured signal, or the process signals when there is none.
async fn wait_for_signal(custom: Option<ShutdownSignalHandler>) {
    match custom {
        Some(signal) => signal.await,
        None => default_shutdown_signal().await,
    }
}

/// Wait for [`ServerHandle::shutdown`].
async fn wait_for_handle_shutdown(mut rx: watch::Receiver<bool>) {
    while !*rx.borrow_and_update() {
        if rx.changed().await.is_err() {
            // All handles dropped is not a shutdown request; keep waiting for signals.
            std::future::pending::<()>().await;
        }
    }
}

/// The default shutdown signal: `Ctrl+C`, plus `SIGTERM` on Unix.
///
/// A handler that cannot be installed is logged and ignored, so a restricted
/// environment degrades to the remaining signal rather than aborting.
async fn default_shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = signal::ctrl_c().await {
            warn!("Could not install the Ctrl+C handler: {err}");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match signal::unix::signal(signal::unix::SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(err) => {
                warn!("Could not install the SIGTERM handler: {err}");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
