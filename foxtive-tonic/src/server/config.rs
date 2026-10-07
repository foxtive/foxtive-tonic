use crate::health::HealthState;
use crate::server::ServerHandle;
use foxtive::App;
use foxtive::results::AppResult;
use foxtive::setup::trace::Tracing;
use futures::future::BoxFuture;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tonic::body::Body;
use tonic::codegen::http::{Request, Response};
use tonic::service::Routes;
use tower::util::BoxCloneService;

#[cfg(feature = "tls")]
use tonic::transport::ServerTlsConfig;

/// Type alias for a boxed future that resolves when the server should shut down.
pub type ShutdownSignalHandler = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// Type alias for the bootstrap function called before the server starts.
pub type BootstrapFn = Box<dyn FnOnce(Arc<App>) -> BoxFuture<'static, AppResult<()>> + Send>;

/// Type alias for the service configuration closure.
type ServiceConfigFn = Box<dyn FnOnce() -> Routes + Send>;

/// Type alias for a boxed service that can be layered.
///
/// The error type is a boxed error rather than the `Infallible` that [`Routes`]
/// reports, because tower's own layers (`LoadShedLayer`, for example) are
/// written against boxed errors. [`Server::layer`] widens `Infallible` into
/// `BoxError`.
pub type BoxedService =
    BoxCloneService<Request<Body>, Response<Body>, Box<dyn std::error::Error + Send + Sync>>;

/// Type alias for a layer function that transforms a boxed service.
type LayerFn = Box<dyn FnOnce(BoxedService) -> BoxedService + Send>;

/// The gRPC server configuration.
///
/// Use the builder pattern to configure host, port, services, interceptors,
/// and lifecycle hooks, then call [`Server::run`] to start serving.
///
/// # Example
/// ```rust,no_run
/// use foxtive_tonic::server::Server;
/// use tonic::service::Routes;
/// use foxtive::App;
/// use std::sync::Arc;
///
/// # async fn example(app: Arc<App>) {
/// Server::new(app)
///     .host("127.0.0.1")
///     .port(50051)
///     .configure(|| {
///         Routes::builder()
///             // .add_service(&my_grpc_service)
///             .routes()
///     })
///     .run()
///     .await;
/// # }
/// ```
pub struct Server {
    /// The foxtive application instance (DI container).
    pub(crate) app: Arc<App>,

    /// Optional bootstrap function called before the server starts.
    pub(crate) bootstrap: Option<BootstrapFn>,

    /// Optional future executed after the server has started.
    pub(crate) on_started: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,

    /// Optional future executed during graceful shutdown.
    pub(crate) on_shutdown: Option<ShutdownSignalHandler>,

    /// Optional custom shutdown signal (overrides default Ctrl+C / SIGTERM).
    pub(crate) shutdown_signal: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,

    /// The host address to bind to (default: "0.0.0.0").
    pub(crate) host: String,
    /// The port to listen on (default: 50051).
    pub(crate) port: u16,

    /// Optional tracing configuration.
    pub(crate) tracing_config: Option<Tracing>,

    /// Whether the app bootstrap has already been started externally.
    pub(crate) has_started_bootstrap: bool,

    /// Health state to serve as `grpc.health.v1.Health`, if any.
    pub(crate) health: Option<HealthState>,

    /// Closure that builds the gRPC routes.
    pub(crate) service_config: Option<ServiceConfigFn>,

    /// Per-request timeout.
    pub(crate) timeout: Option<Duration>,

    /// Per-connection concurrency limit.
    pub(crate) concurrency_limit: Option<usize>,

    /// TCP keepalive duration.
    pub(crate) tcp_keepalive: Option<Duration>,

    /// TCP keepalive interval.
    pub(crate) tcp_keepalive_interval: Option<Duration>,

    /// TCP keepalive retries.
    pub(crate) tcp_keepalive_retries: Option<u32>,

    /// TCP_NODELAY setting (default: true).
    pub(crate) tcp_nodelay: bool,

    /// HTTP/2 keepalive interval.
    pub(crate) http2_keepalive_interval: Option<Duration>,

    /// HTTP/2 keepalive timeout.
    pub(crate) http2_keepalive_timeout: Option<Duration>,

    /// HTTP/2 adaptive window.
    pub(crate) http2_adaptive_window: Option<bool>,

    /// Max concurrent streams per connection.
    pub(crate) max_concurrent_streams: Option<u32>,

    /// Initial stream window size.
    pub(crate) initial_stream_window_size: Option<u32>,

    /// Initial connection window size.
    pub(crate) initial_connection_window_size: Option<u32>,

    /// Max connection age.
    pub(crate) max_connection_age: Option<Duration>,

    /// Max connection age grace period.
    pub(crate) max_connection_age_grace: Option<Duration>,

    /// Enable load shedding.
    pub(crate) load_shed: bool,

    /// Layer function for middleware.
    pub(crate) layer_fn: Option<LayerFn>,

    /// TLS configuration.
    #[cfg(feature = "tls")]
    pub(crate) tls_config: Option<ServerTlsConfig>,

    /// Whether to install a structured panic hook with backtrace logging.
    pub(crate) panic_hook: bool,
}

impl Server {
    /// Create a new server with the given application instance.
    pub fn new(app: Arc<App>) -> Server {
        Server {
            port: 50051,
            host: "0.0.0.0".to_string(),
            app,
            has_started_bootstrap: false,
            tracing_config: None,
            health: None,
            on_started: None,
            on_shutdown: None,
            bootstrap: None,
            shutdown_signal: None,
            service_config: None,
            timeout: None,
            concurrency_limit: None,
            tcp_keepalive: None,
            tcp_keepalive_interval: None,
            tcp_keepalive_retries: None,
            tcp_nodelay: true,
            http2_keepalive_interval: None,
            http2_keepalive_timeout: None,
            http2_adaptive_window: None,
            max_concurrent_streams: None,
            initial_stream_window_size: None,
            initial_connection_window_size: None,
            max_connection_age: None,
            max_connection_age_grace: None,
            load_shed: false,
            layer_fn: None,
            #[cfg(feature = "tls")]
            tls_config: None,
            panic_hook: false,
        }
    }

    /// Set the server host address.
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = host.into();
        self
    }

    /// Set the server port.
    ///
    /// `0` asks the OS for a free port; read the assigned one back from
    /// [`ServerHandle::local_addr`].
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Serve `grpc.health.v1.Health` backed by `state`.
    ///
    /// The health service is added to the server's routes automatically, so
    /// `configure` does not need to register it. Probe it with `grpcurl`,
    /// `grpc_health_probe` or a Kubernetes `grpc` readiness probe.
    ///
    /// # Example
    /// ```rust,no_run
    /// use foxtive_tonic::health::HealthState;
    /// use foxtive_tonic::server::Server;
    /// # use std::sync::Arc;
    /// # use foxtive::App;
    /// # async fn example(app: Arc<App>) {
    /// let health = HealthState::new();
    ///
    /// Server::new(app)
    ///     .health(health.clone())
    ///     .on_shutdown(async move {
    ///         // Start failing probes while in-flight requests drain.
    ///         health.set_not_serving().await;
    ///     })
    ///     .configure(|| tonic::service::Routes::builder().routes())
    ///     .run()
    ///     .await;
    /// # }
    /// ```
    pub fn health(mut self, state: HealthState) -> Self {
        self.health = Some(state);
        self
    }

    /// Set the tracing configuration.
    pub fn tracing(mut self, config: Tracing) -> Self {
        self.tracing_config = Some(config);
        self
    }

    /// Configure gRPC services by building routes.
    ///
    /// The closure should return a `tonic::service::Routes` with all services added.
    /// Use `Routes::builder()` or `Routes::new()` to construct routes.
    ///
    /// # Example
    /// ```rust,no_run
    /// use foxtive_tonic::server::Server;
    /// use tonic::service::Routes;
    /// # use std::sync::Arc;
    /// # use foxtive::App;
    /// # async fn example(app: Arc<App>) {
    /// Server::new(app)
    ///     .configure(|| {
    ///         let mut builder = Routes::builder();
    ///         // builder.add_service(&MyServiceServer::new(my_impl));
    ///         builder.routes()
    ///     });
    /// # }
    /// ```
    pub fn configure<F>(mut self, f: F) -> Self
    where
        F: FnOnce() -> Routes + Send + 'static,
    {
        self.service_config = Some(Box::new(f));
        self
    }

    /// Provide a future to run once the listener is bound.
    ///
    /// It is **spawned** on the runtime, not awaited, so a long-running or
    /// never-returning hook cannot delay `run()` or deadlock startup. The socket
    /// is already accepting connections by the time the hook runs, so the hook
    /// may make calls against this server.
    pub fn on_started<F>(mut self, handler: F) -> Self
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.on_started = Some(Box::pin(handler));
        self
    }

    /// Set a handler to run once the shutdown signal fires, before the listener
    /// stops accepting and while in-flight requests are still draining.
    ///
    /// This runs whether the signal is the default `Ctrl+C`/`SIGTERM` one or a
    /// custom [`Server::shutdown_signal`].
    pub fn on_shutdown<F>(mut self, func: F) -> Self
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.on_shutdown = Some(Box::pin(func));
        self
    }

    /// Replace the default shutdown signal (`Ctrl+C` or `SIGTERM`).
    ///
    /// The [`Server::on_shutdown`] handler still runs afterwards, so this only
    /// decides *when* draining starts, not what happens during it.
    pub fn shutdown_signal<F>(mut self, func: F) -> Self
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.shutdown_signal = Some(Box::pin(func));
        self
    }

    /// Provide a function to execute before the server starts.
    pub fn bootstrap<F, Fut>(mut self, func: F) -> Self
    where
        F: FnOnce(Arc<App>) -> Fut + Send + 'static,
        Fut: Future<Output = AppResult<()>> + Send + 'static,
    {
        self.bootstrap = Some(Box::new(|app| Box::pin(func(app))));
        self
    }

    /// Declare that the application already performed its own start-up setup.
    ///
    /// `Server::spawn` otherwise installs a default tracing subscriber; this
    /// suppresses that. It affects tracing only ([`Server::bootstrap`] and the
    /// lifecycle hooks always run).
    pub fn has_started_bootstrap(mut self, has_started_bootstrap: bool) -> Self {
        self.has_started_bootstrap = has_started_bootstrap;
        self
    }

    /// Set a timeout for all request handlers.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set the concurrency limit per connection.
    pub fn concurrency_limit(mut self, limit: usize) -> Self {
        self.concurrency_limit = Some(limit);
        self
    }

    /// Set TCP keepalive duration.
    pub fn tcp_keepalive(mut self, keepalive: Duration) -> Self {
        self.tcp_keepalive = Some(keepalive);
        self
    }

    /// Set TCP keepalive interval.
    pub fn tcp_keepalive_interval(mut self, interval: Duration) -> Self {
        self.tcp_keepalive_interval = Some(interval);
        self
    }

    /// Set TCP keepalive retries.
    pub fn tcp_keepalive_retries(mut self, retries: u32) -> Self {
        self.tcp_keepalive_retries = Some(retries);
        self
    }

    /// Set TCP_NODELAY (default: true).
    pub fn tcp_nodelay(mut self, enabled: bool) -> Self {
        self.tcp_nodelay = enabled;
        self
    }

    /// Set HTTP/2 keepalive interval.
    pub fn http2_keepalive_interval(mut self, interval: Option<Duration>) -> Self {
        self.http2_keepalive_interval = interval;
        self
    }

    /// Set HTTP/2 keepalive timeout.
    pub fn http2_keepalive_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.http2_keepalive_timeout = timeout;
        self
    }

    /// Enable HTTP/2 adaptive window.
    pub fn http2_adaptive_window(mut self, enabled: Option<bool>) -> Self {
        self.http2_adaptive_window = enabled;
        self
    }

    /// Set max concurrent streams per connection.
    pub fn max_concurrent_streams(mut self, max: impl Into<Option<u32>>) -> Self {
        self.max_concurrent_streams = max.into();
        self
    }

    /// Set initial stream window size.
    pub fn initial_stream_window_size(mut self, size: impl Into<Option<u32>>) -> Self {
        self.initial_stream_window_size = size.into();
        self
    }

    /// Set initial connection window size.
    pub fn initial_connection_window_size(mut self, size: impl Into<Option<u32>>) -> Self {
        self.initial_connection_window_size = size.into();
        self
    }

    /// Set max connection age.
    pub fn max_connection_age(mut self, age: Duration) -> Self {
        self.max_connection_age = Some(age);
        self
    }

    /// Set max connection age grace period.
    pub fn max_connection_age_grace(mut self, grace: Duration) -> Self {
        self.max_connection_age_grace = Some(grace);
        self
    }

    /// Enable or disable load shedding.
    pub fn load_shed(mut self, enabled: bool) -> Self {
        self.load_shed = enabled;
        self
    }

    /// Add a tower layer for middleware.
    ///
    /// Layers run in the order they are added: `.layer(a).layer(b)` puts `a`
    /// outside `b`, so a request passes `a` then `b` and a response back through
    /// `b` then `a`. Same rule as `tower::ServiceBuilder` and
    /// `tonic::transport::Server::layer`.
    ///
    /// # Example
    /// ```rust,no_run
    /// use foxtive_tonic::server::Server;
    /// use tower::load_shed::LoadShedLayer;
    /// use std::sync::Arc;
    /// use foxtive::App;
    ///
    /// # async fn example(app: Arc<App>) {
    /// Server::new(app)
    ///     .layer(LoadShedLayer::new())
    ///     .run()
    ///     .await;
    /// # }
    /// ```
    pub fn layer<L>(mut self, layer: L) -> Self
    where
        L: tower::Layer<BoxedService> + Clone + Send + 'static,
        L::Service: tower::Service<
                Request<Body>,
                Response = Response<Body>,
                Error = Box<dyn std::error::Error + Send + Sync>,
            > + Clone
            + Send
            + 'static,
        <L::Service as tower::Service<Request<Body>>>::Future: Send + 'static,
    {
        let new_fn = move |svc: BoxedService| {
            let layered = layer.layer(svc);
            BoxCloneService::new(layered)
        };

        self.layer_fn = Some(match self.layer_fn {
            // Wrap the existing chain's inside, so the first-added layer stays outermost.
            Some(prev) => Box::new(move |svc: BoxedService| {
                let inner = new_fn(svc);
                prev(inner)
            }),
            None => Box::new(new_fn),
        });
        self
    }

    /// Configure TLS.
    #[cfg(feature = "tls")]
    pub fn tls(mut self, config: ServerTlsConfig) -> Self {
        self.tls_config = Some(config);
        self
    }

    /// Enable structured panic logging with backtrace in debug mode.
    ///
    /// Logged information includes the panic message, location, and a backtrace
    /// (in debug builds).
    ///
    /// # Example
    /// ```rust,no_run
    /// use foxtive_tonic::server::Server;
    /// use foxtive::App;
    /// use std::sync::Arc;
    ///
    /// # async fn example(app: Arc<App>) {
    /// Server::new(app)
    ///     .panic_hook(true)
    ///     .run()
    ///     .await;
    /// # }
    /// ```
    pub fn panic_hook(mut self, enable: bool) -> Self {
        self.panic_hook = enable;
        self
    }

    /// Start the gRPC server and begin accepting connections.
    ///
    /// Returns once the shutdown signal has been handled and the server has
    /// stopped. Use [`Server::spawn`] to serve in the background instead.
    pub async fn run(self) -> AppResult<()> {
        self.spawn().await?.stopped().await
    }

    /// Bind the socket and serve in a background task, returning a handle.
    ///
    /// Binding and `bootstrap` failures are reported by this call, before the
    /// caller assumes the server is up. The returned [`ServerHandle`] exposes the
    /// real bound address, requests shutdown, and awaits the drain.
    ///
    /// # Example
    /// ```rust,no_run
    /// use foxtive_tonic::server::Server;
    /// # use std::sync::Arc;
    /// # use foxtive::App;
    /// # async fn example(app: Arc<App>) -> foxtive::results::AppResult<()> {
    /// let handle = Server::new(app)
    ///     .port(0)
    ///     .configure(|| tonic::service::Routes::builder().routes())
    ///     .spawn()
    ///     .await?;
    ///
    /// println!("listening on {}", handle.local_addr());
    /// // ... drive the workload ...
    /// handle.shutdown();
    /// handle.stopped().await
    /// # }
    /// ```
    pub async fn spawn(self) -> AppResult<ServerHandle> {
        crate::server::spawn(self).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tower::{Layer, Service as _};

    /// A minimal container for builder tests.
    async fn test_app() -> Arc<App> {
        foxtive::App::builder("test-app", "TEST")
            .environment(foxtive::Environment::Local)
            .build()
            .await
            .expect("a bare App with no infrastructure must build")
    }

    /// A layer that appends its tag to `log` before passing the request inwards.
    #[derive(Clone)]
    struct Marker {
        tag: &'static str,
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    impl Layer<BoxedService> for Marker {
        type Service = BoxedService;

        fn layer(&self, inner: BoxedService) -> BoxedService {
            let tag = self.tag;
            let log = self.log.clone();

            BoxCloneService::new(tower::service_fn(move |req: Request<Body>| {
                let log = log.clone();
                let mut inner = inner.clone();
                async move {
                    log.lock().unwrap().push(tag);
                    inner.call(req).await
                }
            }))
        }
    }

    fn recorded(log: Arc<Mutex<Vec<&'static str>>>) -> BoxedService {
        BoxCloneService::new(tower::service_fn(move |_req: Request<Body>| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push("inner");
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Response::new(Body::empty()))
            }
        }))
    }

    #[tokio::test]
    async fn new_uses_the_documented_defaults() {
        let server = Server::new(test_app().await);

        assert_eq!(server.port, 50051);
        assert_eq!(server.host, "0.0.0.0");
        assert!(server.tcp_nodelay, "gRPC wants low latency by default");
        assert!(!server.load_shed);
        assert!(!server.panic_hook);
        assert!(!server.has_started_bootstrap);
        assert!(server.timeout.is_none());
        assert!(server.concurrency_limit.is_none());
        assert!(server.max_connection_age.is_none());
        assert!(server.health.is_none());
        assert!(server.service_config.is_none());
        assert!(server.layer_fn.is_none());
    }

    #[tokio::test]
    async fn setters_replace_the_defaults() {
        let health = HealthState::new();
        let server = Server::new(test_app().await)
            .host("127.0.0.1")
            .port(0)
            .timeout(Duration::from_secs(5))
            .concurrency_limit(16)
            .tcp_nodelay(false)
            .max_connection_age(Duration::from_secs(300))
            .max_connection_age_grace(Duration::from_secs(30))
            .load_shed(true)
            .panic_hook(true)
            .has_started_bootstrap(true)
            .health(health)
            .configure(|| Routes::builder().routes());

        assert_eq!(server.port, 0);
        assert_eq!(server.host, "127.0.0.1");
        assert_eq!(server.timeout, Some(Duration::from_secs(5)));
        assert_eq!(server.concurrency_limit, Some(16));
        assert!(!server.tcp_nodelay);
        assert_eq!(server.max_connection_age, Some(Duration::from_secs(300)));
        assert_eq!(
            server.max_connection_age_grace,
            Some(Duration::from_secs(30))
        );
        assert!(server.load_shed);
        assert!(server.panic_hook);
        assert!(server.has_started_bootstrap);
        assert!(server.health.is_some());
        assert!(server.service_config.is_some());
    }

    #[tokio::test]
    async fn layers_run_in_the_order_they_are_added() {
        // `.layer(a).layer(b)` must nest a outside b, like `ServiceBuilder`.
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut server = Server::new(test_app().await)
            .layer(Marker {
                tag: "a",
                log: log.clone(),
            })
            .layer(Marker {
                tag: "b",
                log: log.clone(),
            });

        let layer_fn = server.layer_fn.take().expect("two layers were added");
        let mut svc = layer_fn(recorded(log.clone()));
        svc.call(Request::new(Body::empty())).await.unwrap();

        assert_eq!(
            *log.lock().unwrap(),
            vec!["a", "b", "inner"],
            "the first added layer has to observe the request first"
        );
    }

    #[tokio::test]
    async fn a_single_layer_wraps_the_service() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut server = Server::new(test_app().await).layer(Marker {
            tag: "only",
            log: log.clone(),
        });

        let layer_fn = server.layer_fn.take().expect("one layer was added");
        let mut svc = layer_fn(recorded(log.clone()));
        svc.call(Request::new(Body::empty())).await.unwrap();

        assert_eq!(*log.lock().unwrap(), vec!["only", "inner"]);
    }
}
