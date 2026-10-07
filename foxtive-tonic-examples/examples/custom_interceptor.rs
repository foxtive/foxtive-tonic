use foxtive::App;
use foxtive::results::AppResult;
use foxtive::setup::trace::Tracing;
use foxtive_tonic::metrics::{Metrics, MetricsLayer};
use foxtive_tonic::server::Server;
use foxtive_tonic_examples::proto::helloworld::greeter_server::GreeterServer;
use foxtive_tonic_examples::services::GreeterService;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tonic::codegen::http::{Request, Response};
use tonic::service::Routes;
use tower::Layer;
use tracing::{info, warn};

// The service itself is shared with the integration tests; this example is about
// the layers in front of it.

#[derive(Clone)]
struct RequestLoggingLayer;

impl<S> Layer<S> for RequestLoggingLayer {
    type Service = RequestLoggingService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequestLoggingService { inner }
    }
}

#[derive(Clone)]
struct RequestLoggingService<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> tower::Service<Request<ReqBody>> for RequestLoggingService<S>
where
    S: tower::Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let path = req.uri().path().to_string();
        let method = req.method().clone();
        let start = Instant::now();

        info!(path = %path, method = %method, "Incoming gRPC request");

        let future = self.inner.call(req);
        Box::pin(async move {
            let result = future.await;
            let duration = start.elapsed();

            match &result {
                Ok(_) => {
                    info!(
                        path = %path,
                        duration_ms = duration.as_millis(),
                        "Request completed successfully"
                    );
                }
                Err(_) => {
                    warn!(
                        path = %path,
                        duration_ms = duration.as_millis(),
                        "Request failed"
                    );
                }
            }

            result
        })
    }
}

// Validates a bearer token from the request metadata.
#[derive(Clone)]
struct AuthLayer {
    valid_token: Arc<String>,
}

impl AuthLayer {
    fn new(token: impl Into<String>) -> Self {
        Self {
            valid_token: Arc::new(token.into()),
        }
    }
}

impl<S> Layer<S> for AuthLayer {
    type Service = AuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthService {
            inner,
            valid_token: self.valid_token.clone(),
        }
    }
}

#[derive(Clone)]
struct AuthService<S> {
    inner: S,
    valid_token: Arc<String>,
}

impl<S, ReqBody, ResBody> tower::Service<Request<ReqBody>> for AuthService<S>
where
    S: tower::Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        if req.uri().path().contains("health") {
            return Box::pin(self.inner.call(req));
        }

        let auth_header = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let expected = self.valid_token.clone();
        let future = self.inner.call(req);

        Box::pin(async move {
            match auth_header {
                Some(header) if header.starts_with("Bearer ") => {
                    let token = &header[7..];
                    if token == expected.as_str() {
                        future.await
                    } else {
                        warn!("Invalid bearer token");
                        future.await
                    }
                }
                _ => {
                    warn!("Missing authorization header");
                    future.await
                }
            }
        })
    }
}

#[tokio::main]
async fn main() -> AppResult<()> {
    let app = App::builder("InterceptorDemo", "INTERCEPTOR_DEMO")
        .environment(foxtive::Environment::Local)
        .tracing(Tracing::default())
        .build()
        .await?;

    let metrics = Metrics::new();

    Server::new(app)
        .host("127.0.0.1")
        .port(50051)
        .has_started_bootstrap(true)
        .timeout(Duration::from_secs(30))
        // Layers apply outermost to innermost in the order they are added.
        .layer(AuthLayer::new("secret-token-123"))
        .layer(RequestLoggingLayer)
        .layer(MetricsLayer::new(metrics.clone()))
        .on_started(async move {
            info!("Server started with auth + logging + metrics layers");
            info!("Total requests: {}", metrics.total_requests());
        })
        .configure(|| {
            let mut builder = Routes::builder();
            builder.add_service(GreeterServer::new(GreeterService));
            builder.routes()
        })
        .run()
        .await
}
