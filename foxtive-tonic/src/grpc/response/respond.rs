use crate::contracts::ResponseCodeContract;
use crate::enums::response_code::ResponseCode;
use crate::error::GrpcError;
use crate::grpc::responder::Responder;
use crate::grpc::response::ext::{AppMessageExt, ResponderExt, ResultResponseExt};
use foxtive::prelude::{AppMessage, AppResult};
use serde::Serialize;
use tokio::task::JoinError;

impl<T> ResponderExt for AppResult<T>
where
    T: Sized + Serialize,
{
    fn respond_code<C: ResponseCodeContract, M: Into<String>>(
        self,
        msg: M,
        code: C,
    ) -> Result<String, GrpcError> {
        self.send_result_msg(code, msg)
    }

    fn respond_msg(self, msg: impl Into<String>) -> Result<String, GrpcError> {
        self.send_result_msg(ResponseCode::Ok, msg)
    }

    fn respond(self) -> Result<String, GrpcError> {
        self.send_result(ResponseCode::Ok)
    }
}

impl<T> ResponderExt for Result<AppResult<T>, JoinError>
where
    T: Sized + Serialize,
{
    fn respond_code<C: ResponseCodeContract, M: Into<String>>(
        self,
        msg: M,
        code: C,
    ) -> Result<String, GrpcError> {
        match self {
            Ok(val) => val.send_result_msg(code, msg),
            Err(err) => Err(GrpcError::JoinError(err)),
        }
    }

    fn respond_msg(self, msg: impl Into<String>) -> Result<String, GrpcError> {
        match self {
            Ok(val) => val.send_result_msg(ResponseCode::Ok, msg),
            Err(err) => Err(GrpcError::JoinError(err)),
        }
    }

    fn respond(self) -> Result<String, GrpcError> {
        match self {
            Ok(val) => val.send_result(ResponseCode::Ok),
            Err(err) => Err(GrpcError::JoinError(err)),
        }
    }
}

impl AppMessageExt for AppMessage {
    fn respond(self) -> Result<String, GrpcError> {
        let grpc_code = crate::error::http_status_to_grpc_code(self.status_code().as_u16());
        let code = ResponseCode::from_status(grpc_code);
        match code.success() {
            true => Ok(Responder::message(&self.message(), code)),
            false => Err(GrpcError::AppError(self)),
        }
    }
}

impl AppMessageExt for AppResult<AppMessage> {
    fn respond(self) -> Result<String, GrpcError> {
        match self {
            Ok(msg) => msg.respond(),
            Err(err) => Err(GrpcError::AppError(err)),
        }
    }
}
