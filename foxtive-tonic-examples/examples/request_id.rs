use foxtive::App;
use foxtive::results::AppResult;
use foxtive::setup::trace::Tracing;
use foxtive_tonic::server::Server;
use foxtive_tonic_examples::proto::helloworld::greeter_server::GreeterServer;
use foxtive_tonic_examples::services::GreeterService;
use std::time::Duration;
use tonic::codegen::http::{Request, Response};
use tonic::service::Routes;
use tower::Layer;
use tracing::info;

// Extracts x-request-id from gRPC metadata and adds it to the tracing span.
#[derive(Clone)]
struct RequestIdLayer;

impl<S> Layer<S> for RequestIdLayer {
    type Service = RequestIdService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequestIdService { inner }
    }
}

#[derive(Clone)]
struct RequestIdService<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> tower::Service<Request<ReqBody>> for RequestIdService<S>
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
        let request_id = req
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string();

        let path = req.uri().path().to_string();

        let span = tracing::info_span!(
            "grpc_request",
            request_id = %request_id,
            path = %path,
        );

        let _enter = span.enter();
        let future = self.inner.call(req);
        drop(_enter);

        Box::pin(async move {
            let _span_guard = span.enter();
            let result = future.await;

            if let Ok(_response) = &result {
                info!(
                    request_id = %request_id,
                    "Request completed"
                );
            }

            result
        })
    }
}

#[tokio::main]
async fn main() -> AppResult<()> {
    let app = App::builder("RequestIdDemo", "REQUEST_ID_DEMO")
        .environment(foxtive::Environment::Local)
        .tracing(Tracing::default())
        .build()
        .await?;

    Server::new(app)
        .host("127.0.0.1")
        .port(50051)
        .has_started_bootstrap(true)
        .timeout(Duration::from_secs(30))
        .layer(RequestIdLayer)
        .on_started(async {
            info!("Request ID demo server started");
            info!("Test with: grpcurl -plaintext -H 'x-request-id: req-abc-123' -d '{{\"name\":\"Alice\"}}' localhost:50051 helloworld.Greeter/SayHello");
        })
        .configure(|| {
            let mut builder = Routes::builder();
            builder.add_service(GreeterServer::new(GreeterService));
            builder.routes()
        })
        .run()
        .await
}
