use crate::contracts::ResponseCodeContract;
use crate::error::GrpcError;
use crate::grpc::responder::Responder;
use crate::grpc::response::ext::{OptionResultResponseExt, ResultResponseExt};
use foxtive::prelude::{AppMessage, AppResult};
use serde::Serialize;

impl<T: Serialize> ResultResponseExt for AppResult<T> {
    fn send_result<C: ResponseCodeContract>(self, code: C) -> Result<String, GrpcError> {
        match self {
            Ok(data) => Ok(Responder::send(data, code)),
            Err(err) => Err(GrpcError::AppError(err)),
        }
    }

    fn send_result_msg<C: ResponseCodeContract, M: Into<String>>(
        self,
        code: C,
        msg: M,
    ) -> Result<String, GrpcError> {
        match self {
            Ok(data) => Ok(Responder::send_msg(data, code, msg)),
            Err(err) => Err(GrpcError::AppError(err)),
        }
    }
}

impl<T: Serialize> OptionResultResponseExt<T> for AppResult<T> {
    fn is_empty(&self) -> bool {
        match self {
            Ok(_) => false,
            Err(message) => matches!(message, AppMessage::NotFound(..)),
        }
    }

    fn is_error(&self) -> bool {
        self.as_ref().is_err()
    }

    fn is_error_or_empty(&self) -> bool {
        self.is_error() || self.is_empty()
    }

    fn send_response<C: ResponseCodeContract, M: Into<String>>(
        self,
        code: C,
        msg: M,
    ) -> Result<String, GrpcError> {
        let data = self.map_err(GrpcError::AppError)?;
        Ok(Responder::send_msg(data, code, msg))
    }
}
