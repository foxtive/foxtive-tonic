//! The typed `Envelope` path: a service whose methods answer with a real protobuf
//! message instead of a JSON string.
//!
//! Run it, then:
//!
//! ```text
//! grpcurl -plaintext -d '{"payload":"{\"id\":7,\"name\":\"Alice\"}","code":"001"}' \
//!     localhost:50051 foxtive.tonic.examples.v1.Demo/Echo
//! grpcurl -plaintext -d '{"message":"deleted"}' \
//!     localhost:50051 foxtive.tonic.examples.v1.Demo/Ack
//! grpcurl -plaintext -d '{"entity":"User"}' \
//!     localhost:50051 foxtive.tonic.examples.v1.Demo/AppFailure
//! ```
//!
//! The handler lives in `foxtive_tonic_examples::services::DemoService`, which
//! `tests/envelope.rs` also exercises over a real socket. For the other style (the
//! same envelope as a JSON `String` in a proto field), see `examples/error_handling.rs`.

use foxtive::App;
use foxtive::results::AppResult;
use foxtive::setup::trace::Tracing;
use foxtive_tonic::health;
use foxtive_tonic::health::HealthState;
use foxtive_tonic::server::Server;
use foxtive_tonic_examples::proto::demo::FILE_DESCRIPTOR_SET as DEMO_DESCRIPTOR_SET;
use foxtive_tonic_examples::proto::demo::demo_server::DemoServer;
use foxtive_tonic_examples::services::DemoService;
use std::time::Duration;
use tonic::service::Routes;
use tracing::info;

#[tokio::main]
async fn main() -> AppResult<()> {
    let app = App::builder("TypedEnvelope", "TYPED_ENVELOPE")
        .environment(foxtive::Environment::Local)
        .tracing(Tracing::default())
        .build()
        .await?;

    Server::new(app)
        .host("127.0.0.1")
        .port(50051)
        .health(HealthState::new())
        .has_started_bootstrap(true)
        .timeout(Duration::from_secs(10))
        .on_started(async {
            info!("Typed-envelope demo listening on 50051");
            info!("Try: grpcurl -plaintext -d '{{\"payload\":\"{{\\\"id\\\":7}}\",\"code\":\"001\"}}' localhost:50051 foxtive.tonic.examples.v1.Demo/Echo");
        })
        .configure(|| {
            let mut builder = Routes::builder();
            builder.add_service(DemoServer::new(DemoService));

            // The demo's descriptor set includes the imported envelope.proto, so
            // grpcurl can describe the envelope's fields rather than treat it as unknown.
            #[cfg(feature = "reflection")]
            {
                let reflection_service = tonic_reflection::server::Builder::configure()
                    .register_encoded_file_descriptor_set(DEMO_DESCRIPTOR_SET)
                    .register_encoded_file_descriptor_set(health::FILE_DESCRIPTOR_SET)
                    .build_v1()
                    .expect("reflection descriptors must be valid");

                builder.add_service(reflection_service);
            }

            builder.routes()
        })
        .run()
        .await
}
