//! The service implementations shared by the examples and the integration tests.

use crate::proto::demo::demo_server::Demo;
use crate::proto::demo::{
    AckRequest, AppFailureRequest, EchoRequest, PingReply, PingRequest, SleepRequest,
};
use crate::proto::helloworld::greeter_server::Greeter;
use crate::proto::helloworld::{HelloReply, HelloRequest};
use foxtive::prelude::AppMessage;
use foxtive_tonic::contracts::ResponseCodeContract;
use foxtive_tonic::enums::response_code::ResponseCode;
use foxtive_tonic::error::GrpcError;
use foxtive_tonic::grpc::envelope::{Envelope, EnvelopeResult};
use std::time::Duration;
use tonic::{Request, Response, Status};

/// `helloworld.Greeter`, the hello-world service most examples serve.
#[derive(Clone, Copy, Debug, Default)]
pub struct GreeterService;

#[tonic::async_trait]
impl Greeter for GreeterService {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let name = request.into_inner().name;
        Ok(Response::new(HelloReply {
            message: format!("Hello {name}!"),
        }))
    }
}

/// `foxtive.tonic.examples.v1.Demo`.
///
/// Each method exists so one integration test can express a scenario without a
/// dedicated proto; keep the behaviour minimal and predictable.
#[derive(Clone, Copy, Debug, Default)]
pub struct DemoService;

#[tonic::async_trait]
impl Demo for DemoService {
    async fn ping(&self, request: Request<PingRequest>) -> Result<Response<PingReply>, Status> {
        if request.into_inner().fail {
            // A bare `Status`, not a `GrpcError`: the metrics layer has to see
            // the failure either way.
            return Err(Status::not_found("nothing matches that name"));
        }

        Ok(Response::new(PingReply {
            message: "pong".to_string(),
        }))
    }

    async fn sleep(&self, request: Request<SleepRequest>) -> Result<Response<PingReply>, Status> {
        let millis = request.into_inner().millis;
        tokio::time::sleep(Duration::from_millis(millis)).await;
        Ok(Response::new(PingReply {
            message: format!("slept {millis}ms"),
        }))
    }

    async fn echo(&self, request: Request<EchoRequest>) -> EnvelopeResult {
        let EchoRequest { payload, code } = request.into_inner();

        let data: serde_json::Value = serde_json::from_str(&payload).map_err(|err| {
            GrpcError::AppError(AppMessage::Invalid(format!("payload is not JSON: {err}")))
        })?;

        // `from_code` falls back to `Internal` for a string it does not know, so
        // an arbitrary `code` from the client cannot forge a success flag.
        let response_code = ResponseCode::from_code(&code);

        Ok(Response::new(Envelope::with_code(&data, response_code)?))
    }

    async fn ack(&self, request: Request<AckRequest>) -> EnvelopeResult {
        Ok(Response::new(Envelope::message_only(
            request.into_inner().message,
            ResponseCode::Ok,
        )))
    }

    async fn app_failure(&self, request: Request<AppFailureRequest>) -> EnvelopeResult {
        // `GrpcError`'s conversion puts the JSON envelope in `Status::details`,
        // which is how a typed service reports a failure.
        let entity = request.into_inner().entity;
        Err(GrpcError::AppError(AppMessage::NotFound(entity)).into_status())
    }
}
