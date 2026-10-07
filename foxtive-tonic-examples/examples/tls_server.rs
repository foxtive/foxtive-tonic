use foxtive::App;
use foxtive::results::AppResult;
use foxtive::setup::trace::Tracing;
use foxtive_tonic::server::Server;
use foxtive_tonic_examples::proto::helloworld::greeter_server::GreeterServer;
use foxtive_tonic_examples::services::GreeterService;
use std::time::Duration;
use tonic::service::Routes;
use tonic::transport::{Identity, ServerTlsConfig};
use tracing::info;

#[tokio::main]
async fn main() -> AppResult<()> {
    let app = App::builder("TlsServer", "TLS_SERVER")
        .environment(foxtive::Environment::Production)
        .tracing(Tracing::default())
        .build()
        .await?;

    let cert = std::fs::read("certs/server.pem")?;
    let key = std::fs::read("certs/server.key")?;

    // For mTLS, also call `ServerTlsConfig::client_ca_root` with the CA certificate.
    let tls_config = ServerTlsConfig::new().identity(Identity::from_pem(cert, key));

    Server::new(app)
        .host("0.0.0.0")
        .port(50051)
        .has_started_bootstrap(true)
        .tls(tls_config)
        .timeout(Duration::from_secs(30))
        .http2_keepalive_interval(Some(Duration::from_secs(30)))
        .http2_keepalive_timeout(Some(Duration::from_secs(20)))
        .max_concurrent_streams(1000)
        .bootstrap(|app| async move {
            info!("Bootstrapping TLS server: {}", app.app_name());
            Ok(())
        })
        .on_started(async {
            info!("TLS gRPC server started on port 50051");
        })
        .configure(|| {
            let mut builder = Routes::builder();
            builder.add_service(GreeterServer::new(GreeterService));
            builder.routes()
        })
        .run()
        .await
}
