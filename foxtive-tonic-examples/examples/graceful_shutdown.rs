use foxtive::App;
use foxtive::results::AppResult;
use foxtive::setup::trace::Tracing;
use foxtive_tonic::health::HealthState;
use foxtive_tonic::server::Server;
use foxtive_tonic_examples::proto::helloworld::greeter_server::GreeterServer;
use foxtive_tonic_examples::services::GreeterService;
use std::time::Duration;
use tokio::sync::watch;
use tonic::service::Routes;
use tracing::info;

#[tokio::main]
async fn main() -> AppResult<()> {
    let app = App::builder("GracefulShutdown", "GRACEFUL_SHUTDOWN")
        .environment(foxtive::Environment::Local)
        .tracing(Tracing::default())
        .build()
        .await?;

    let health_state = HealthState::new();
    let health_for_shutdown = health_state.clone();

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let shutdown_trigger = shutdown_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        info!("Triggering shutdown via custom signal");
        let _ = shutdown_trigger.send(true);
    });

    Server::new(app)
        .host("127.0.0.1")
        .port(50051)
        .has_started_bootstrap(true)
        // Serves grpc.health.v1.Health so probes start failing the moment we drain.
        .health(health_state.clone())
        // Connection age ensures connections are recycled
        .max_connection_age(Duration::from_secs(300))
        // Grace period allows in-flight requests to complete
        .max_connection_age_grace(Duration::from_secs(30))
        .timeout(Duration::from_secs(10))
        // Use custom shutdown signal instead of default Ctrl+C
        .shutdown_signal(async move {
            let mut rx = shutdown_rx;
            while !*rx.borrow_and_update() {
                if rx.changed().await.is_err() {
                    break;
                }
            }
            info!("Custom shutdown signal received");
        })
        .bootstrap(|app| async move {
            info!("Bootstrapping: {}", app.app_name());
            Ok(())
        })
        .on_started(async move {
            info!("Server started, marking as healthy");
            health_state.set_serving().await;
        })
        .on_shutdown(async move {
            info!("Shutdown initiated, marking as not serving");
            health_for_shutdown.set_not_serving().await;

            info!("Cleaning up resources...");
            tokio::time::sleep(Duration::from_secs(1)).await;
            info!("Cleanup complete");
        })
        .configure(|| {
            let mut builder = Routes::builder();
            builder.add_service(GreeterServer::new(GreeterService));
            builder.routes()
        })
        .run()
        .await
}
