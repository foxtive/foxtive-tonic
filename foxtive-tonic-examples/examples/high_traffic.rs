use foxtive::App;
use foxtive::results::AppResult;
use foxtive::setup::trace::Tracing;
use foxtive_tonic::health::HealthState;
use foxtive_tonic::server::Server;
use foxtive_tonic_examples::proto::helloworld::greeter_server::GreeterServer;
use foxtive_tonic_examples::services::GreeterService;
use std::time::Duration;
use tonic::service::Routes;
use tracing::info;

#[tokio::main]
async fn main() -> AppResult<()> {
    let app = App::builder("HighTraffic", "HIGH_TRAFFIC")
        .environment(foxtive::Environment::Production)
        .tracing(Tracing::default())
        .build()
        .await?;

    // Readiness gate: probes fail until the listener is actually bound.
    let health = HealthState::new();
    health.set_not_serving().await;

    Server::new(app)
        .host("0.0.0.0")
        .port(50051)
        .has_started_bootstrap(true)
        .health(health.clone())
        .tcp_nodelay(true)
        .tcp_keepalive(Duration::from_secs(60))
        .tcp_keepalive_interval(Duration::from_secs(10))
        .tcp_keepalive_retries(5)
        .initial_stream_window_size(2 * 1024 * 1024)
        .initial_connection_window_size(4 * 1024 * 1024)
        .http2_adaptive_window(Some(true))
        .http2_keepalive_interval(Some(Duration::from_secs(15)))
        .http2_keepalive_timeout(Some(Duration::from_secs(10)))
        .max_concurrent_streams(5000)
        .concurrency_limit(200)
        .timeout(Duration::from_secs(15))
        .max_connection_age(Duration::from_secs(1800))
        .max_connection_age_grace(Duration::from_secs(60))
        // Reject requests when overloaded instead of buffering.
        .load_shed(true)
        .bootstrap(|app| async move {
            info!("Bootstrapping high-traffic server: {}", app.app_name());
            Ok(())
        })
        .on_started(async move {
            health.set_serving().await;
            info!("High-traffic gRPC server started");
        })
        .configure(|| {
            let mut builder = Routes::builder();
            builder.add_service(GreeterServer::new(GreeterService));
            builder.routes()
        })
        .run()
        .await
}
