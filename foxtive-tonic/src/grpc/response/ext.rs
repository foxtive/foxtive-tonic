use crate::contracts::ResponseCodeContract;
use crate::error::GrpcError;

/// Extension trait for sending `Result` types as gRPC responses.
pub trait ResultResponseExt {
    /// Send the result data with the given response code.
    fn send_result<C: ResponseCodeContract>(self, code: C) -> Result<String, GrpcError>;

    /// Send the result data with the given response code and message.
    fn send_result_msg<C: ResponseCodeContract, M: Into<String>>(
        self,
        code: C,
        msg: M,
    ) -> Result<String, GrpcError>;
}

/// Extension trait for converting `AppMessage` into a gRPC response.
pub trait AppMessageExt {
    /// Convert the message into a gRPC response string.
    fn respond(self) -> Result<String, GrpcError>;
}

/// Extension trait for responding with serializable data and response codes.
pub trait ResponderExt {
    /// Respond with data, a message, and a response code.
    fn respond_code<C: ResponseCodeContract, M: Into<String>>(
        self,
        msg: M,
        code: C,
    ) -> Result<String, GrpcError>;

    /// Respond with data and a success message.
    fn respond_msg(self, suc: impl Into<String>) -> Result<String, GrpcError>;

    /// Respond with data using Ok.
    fn respond(self) -> Result<String, GrpcError>;
}

/// Extension trait for `AppResult<T>` with empty-check helpers.
pub trait OptionResultResponseExt<T> {
    /// Returns `true` if the result is a NotFound error.
    fn is_empty(&self) -> bool;

    /// Returns `true` if the result is an error.
    fn is_error(&self) -> bool;

    /// Returns `true` if the result is an error or a NotFound.
    fn is_error_or_empty(&self) -> bool;

    /// Send the result as a response with the given code and message.
    fn send_response<C: ResponseCodeContract, M: Into<String>>(
        self,
        code: C,
        msg: M,
    ) -> Result<String, GrpcError>;
}
