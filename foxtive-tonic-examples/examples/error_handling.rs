use foxtive::App;
use foxtive::prelude::AppMessage;
use foxtive::results::AppResult;
use foxtive::setup::trace::Tracing;
use foxtive_tonic::enums::response_code::ResponseCode;
use foxtive_tonic::error::GrpcError;
use foxtive_tonic::grpc::responder::Responder;
use foxtive_tonic::grpc::response::ext::ResponderExt;
use foxtive_tonic::server::Server;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tonic::service::Routes;
use tonic::{Request, Response, Status};
use tracing::info;

use foxtive_tonic_examples::proto::helloworld::{
    HelloReply, HelloRequest, greeter_server::Greeter,
};

// Defines its own `Greeter` to show the response and error shapes; the shared
// `foxtive_tonic_examples::services` has the plain version the other examples use.
use foxtive_tonic_examples::proto::helloworld::greeter_server::GreeterServer;

#[derive(Debug, Serialize, Deserialize)]
struct UserProfile {
    id: String,
    name: String,
    email: String,
}

struct GreeterService;

#[tonic::async_trait]
impl Greeter for GreeterService {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let name = request.into_inner().name;

        if name.is_empty() {
            return Err(Status::invalid_argument("Name cannot be empty"));
        }

        let profile = UserProfile {
            id: "usr_123".to_string(),
            name: name.clone(),
            email: format!("{}@example.com", name.to_lowercase()),
        };

        let response_json =
            Responder::send_msg(&profile, ResponseCode::Ok, "User profile retrieved");

        info!("Responding with profile for: {}", name);

        Ok(Response::new(HelloReply {
            message: response_json,
        }))
    }
}

// Exercises the `GrpcError` and `Responder` conversion shapes.
fn demonstrate_error_conversions() {
    let app_error = AppMessage::NotFound("User".to_string());
    let grpc_error = GrpcError::from(app_error);
    let _status: Status = grpc_error.into();

    let status = Status::permission_denied("Access denied to resource");
    let _grpc_error = GrpcError::from(status);

    let _not_found = Responder::not_found();
    let _internal = Responder::internal_server_error();
    let _bad_req = Responder::bad_req_message("Invalid input format");
    let _entity = Responder::entity_not_found_message("Order");
    let _ok = Responder::ok_message("Operation completed");

    let result: AppResult<UserProfile> = Ok(UserProfile {
        id: "usr_456".to_string(),
        name: "Alice".to_string(),
        email: "alice@example.com".to_string(),
    });

    let _response: Result<String, GrpcError> = result.respond();

    let error_result: AppResult<UserProfile> =
        Err(AppMessage::Unauthorized("Token expired".to_string()));
    let _err_response: Result<String, GrpcError> = error_result.respond();
}

#[tokio::main]
async fn main() -> AppResult<()> {
    demonstrate_error_conversions();

    let app = App::builder("ErrorHandling", "ERROR_HANDLING")
        .environment(foxtive::Environment::Local)
        .tracing(Tracing::default())
        .build()
        .await?;

    Server::new(app)
        .host("127.0.0.1")
        .port(50051)
        .has_started_bootstrap(true)
        .timeout(Duration::from_secs(10))
        .on_started(async {
            info!("Error handling demo server started");
            info!("Try: grpcurl -plaintext -d '{{\"name\":\"Alice\"}}' localhost:50051 helloworld.Greeter/SayHello");
        })
        .configure(|| {
            let mut builder = Routes::builder();
            builder.add_service(GreeterServer::new(GreeterService));
            builder.routes()
        })
        .run()
        .await
}
