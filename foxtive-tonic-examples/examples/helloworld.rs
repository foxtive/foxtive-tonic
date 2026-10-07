use foxtive::App;
use foxtive::results::AppResult;
use foxtive::setup::trace::Tracing;
use foxtive_tonic::health::HealthState;
use foxtive_tonic::metrics::{Metrics, MetricsLayer};
use foxtive_tonic::server::Server;
use foxtive_tonic_examples::proto::helloworld::{
    FILE_DESCRIPTOR_SET, greeter_server::GreeterServer,
};
use foxtive_tonic_examples::services::GreeterService;
use std::time::Duration;
use tonic::service::Routes;
use tracing::info;

#[tokio::main]
async fn main() -> AppResult<()> {
    let app = App::builder("HelloWorld", "HELLO_WORLD")
        .environment(foxtive::Environment::Local)
        .tracing(Tracing::default())
        .build()
        .await?;

    let health_state = HealthState::new();
    let metrics = Metrics::new();

    Server::new(app)
        .host("127.0.0.1")
        .port(50051)
        .health(health_state.clone())
        .has_started_bootstrap(true)
        .timeout(Duration::from_secs(30))
        .concurrency_limit(100)
        .tcp_keepalive(Duration::from_secs(60))
        .http2_keepalive_interval(Some(Duration::from_secs(30)))
        .max_concurrent_streams(1000)
        .load_shed(true)
        .layer(MetricsLayer::new(metrics.clone()))
        .bootstrap(|app| async move {
            info!("Bootstrapping gRPC application: {}", app.app_name());
            Ok(())
        })
        .on_started(async move {
            info!("gRPC server started successfully");
            info!("Test with: grpcurl -plaintext -d '{{\"name\":\"Alice\"}}' localhost:50051 helloworld.Greeter/SayHello");
            health_state.set_serving().await;
        })
        .configure(|| {
            let mut builder = Routes::builder();

            builder.add_service(GreeterServer::new(GreeterService));

            #[cfg(feature = "reflection")]
            {
                let reflection_service = tonic_reflection::server::Builder::configure()
                    .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
                    // Also advertise grpc.health.v1.Health, which Server::health serves.
                    .register_encoded_file_descriptor_set(
                        foxtive_tonic::health::FILE_DESCRIPTOR_SET,
                    )
                    .build_v1()
                    .expect("Failed to build reflection service");

                builder.add_service(reflection_service);
            }

            builder.routes()
        })
        .run()
        .await
}
