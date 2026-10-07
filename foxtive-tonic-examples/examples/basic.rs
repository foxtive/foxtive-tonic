use foxtive::App;
use foxtive::results::AppResult;
use foxtive::setup::trace::Tracing;
use foxtive_tonic::metrics::{Metrics, MetricsLayer};
use foxtive_tonic::server::Server;
use std::time::Duration;
use tonic::service::Routes;
use tracing::info;

#[tokio::main]
async fn main() -> AppResult<()> {
    let app = App::builder("BasicTonic", "BASIC_TONIC")
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
        .concurrency_limit(100)
        .tcp_keepalive(Duration::from_secs(60))
        .tcp_keepalive_interval(Duration::from_secs(10))
        .tcp_keepalive_retries(3)
        .http2_keepalive_interval(Some(Duration::from_secs(30)))
        .http2_keepalive_timeout(Some(Duration::from_secs(20)))
        .max_concurrent_streams(1000)
        .max_connection_age(Duration::from_secs(3600))
        .max_connection_age_grace(Duration::from_secs(30))
        .load_shed(true)
        .layer(MetricsLayer::new(metrics.clone()))
        .bootstrap(|app| async move {
            info!("Bootstrapping gRPC application: {}", app.app_name());
            Ok(())
        })
        .on_started(async {
            info!("gRPC server started successfully");
        })
        .configure(|| {
            let builder = Routes::builder();
            // A real service is registered here.
            builder.routes()
        })
        .run()
        .await
}
