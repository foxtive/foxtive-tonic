use crate::contracts::ResponseCodeContract;
use crate::enums::response_code::ResponseCode;
use crate::helpers::json_message::JsonMessage;
use foxtive::prelude::AppMessage;
use thiserror::Error;
use tonic::Code;
use tonic::Status;

/// gRPC error type that converts application errors into gRPC statuses.
///
/// Every variant carries the same JSON envelope in `Status::details` with
/// `success: false`, except a [`GrpcError::Status`] that already has a details
/// payload, which is preserved rather than rewrapped.
#[derive(Error, Debug)]
pub enum GrpcError {
    /// A boxed standard error (maps to Internal).
    #[error("{0}")]
    Std(Box<dyn std::error::Error + Send + Sync + 'static>),
    /// An application-level error message (maps to the message's status code).
    #[error("{0}")]
    AppError(#[from] AppMessage),
    /// A tonic status error.
    #[error("{0}")]
    Status(#[from] Status),
    /// A tokio task join error (maps to Internal).
    #[error("Join Error: {0}")]
    JoinError(#[from] tokio::task::JoinError),
}

impl GrpcError {
    /// Returns the gRPC status code for this error.
    ///
    /// Never returns [`Code::Ok`]: a success status in an error position
    /// becomes [`Code::Internal`].
    pub fn code(&self) -> Code {
        match self {
            GrpcError::AppError(m) => error_status_to_grpc_code(m.status_code().as_u16()),
            GrpcError::Status(s) => never_ok(s.code()),
            _ => Code::Internal,
        }
    }

    /// Convert into a tonic Status carrying this error's envelope in `details`.
    pub fn into_status(self) -> Status {
        match &self {
            GrpcError::AppError(m) => {
                m.log();
                let code = error_status_to_grpc_code(m.status_code().as_u16());
                envelope_status(code, &m.message(), &[])
            }
            GrpcError::Status(s) => {
                // Reuse existing details so a double-wrapped or foreign payload survives.
                let code = never_ok(s.code());
                envelope_status(code, s.message(), s.details())
            }
            GrpcError::Std(e) => {
                tracing::error!("Error: {e}");
                envelope_status(Code::Internal, "Internal Server Error", &[])
            }
            GrpcError::JoinError(e) => {
                tracing::error!("Error: {e}");
                envelope_status(Code::Internal, "Internal Server Error", &[])
            }
        }
    }
}

impl From<Box<dyn std::error::Error + Send + Sync>> for GrpcError {
    fn from(error: Box<dyn std::error::Error + Send + Sync>) -> Self {
        GrpcError::Std(error)
    }
}

impl From<GrpcError> for Status {
    fn from(err: GrpcError) -> Self {
        err.into_status()
    }
}

/// Map an HTTP status code onto its gRPC equivalent, treating 2xx/3xx as success.
pub(crate) fn http_status_to_grpc_code(status: u16) -> Code {
    match status {
        200..=299 => Code::Ok,
        400 | 422 => Code::InvalidArgument,
        401 => Code::Unauthenticated,
        403 => Code::PermissionDenied,
        404 => Code::NotFound,
        409 => Code::AlreadyExists,
        429 => Code::ResourceExhausted,
        501 => Code::Unimplemented,
        503 => Code::Unavailable,
        504 => Code::DeadlineExceeded,
        _ => Code::Internal,
    }
}

/// Map an HTTP status code onto its gRPC equivalent from an *error* position.
///
/// Like [`http_status_to_grpc_code`], but never yields [`Code::Ok`]: a 2xx/3xx
/// status in an error position maps to [`Code::Internal`].
pub(crate) fn error_status_to_grpc_code(status: u16) -> Code {
    match status {
        200..=399 => Code::Internal,
        _ => http_status_to_grpc_code(status),
    }
}

/// Replace a success status found in an error position with [`Code::Internal`].
fn never_ok(code: Code) -> Code {
    if code == Code::Ok {
        Code::Internal
    } else {
        code
    }
}

/// Build a [`Status`] whose details are the standard Foxtive envelope.
///
/// `existing_details` is reused verbatim when non-empty; when empty (as with any
/// `Status::xxx` constructor) a fresh envelope is generated for `code`/`message`.
fn envelope_status(code: Code, message: &str, existing_details: &[u8]) -> Status {
    let details = if existing_details.is_empty() {
        let response_code = ResponseCode::from_status(code);
        let detail = JsonMessage::make((), response_code.code(), false, Some(message.to_string()));
        crate::grpc::responder::serialize_or_internal(&detail, "error envelope").into_bytes()
    } else {
        existing_details.to_vec()
    };

    Status::with_details(code, message, tonic::codegen::Bytes::from(details))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    #[test]
    fn http_status_mapping_table() {
        let cases = [
            (200, Code::Ok),
            (201, Code::Ok),
            (299, Code::Ok),
            (400, Code::InvalidArgument),
            (401, Code::Unauthenticated),
            (403, Code::PermissionDenied),
            (404, Code::NotFound),
            (409, Code::AlreadyExists),
            (422, Code::InvalidArgument),
            (429, Code::ResourceExhausted),
            (501, Code::Unimplemented),
            (503, Code::Unavailable),
            (504, Code::DeadlineExceeded),
            (418, Code::Internal),
            (405, Code::Internal),
            (599, Code::Internal),
        ];
        for (status, expected) in cases {
            assert_eq!(
                http_status_to_grpc_code(status),
                expected,
                "HTTP {status} must map to {expected:?}"
            );
        }
    }

    #[test]
    fn error_status_mapping_never_returns_ok() {
        // A success status in an error position is a server bug, not a success.
        for status in [200u16, 201, 204, 299, 301, 302, 399] {
            assert_eq!(
                error_status_to_grpc_code(status),
                Code::Internal,
                "HTTP {status} in an error position must map to Internal"
            );
        }
        // Everything else keeps the ordinary mapping.
        assert_eq!(error_status_to_grpc_code(404), Code::NotFound);
        assert_eq!(error_status_to_grpc_code(422), Code::InvalidArgument);
        assert_eq!(error_status_to_grpc_code(503), Code::Unavailable);
    }

    #[test]
    fn app_error_with_success_status_does_not_report_ok() {
        let err = GrpcError::AppError(AppMessage::Success("done".into()));
        assert_eq!(err.code(), Code::Internal);
        assert_eq!(err.into_status().code(), Code::Internal);
    }

    #[test]
    fn status_variant_with_ok_code_is_rewritten() {
        let err = GrpcError::Status(Status::ok("surprise"));
        assert_eq!(err.code(), Code::Internal);
    }

    #[test]
    fn every_variant_produces_the_same_envelope_shape() {
        let statuses = [
            GrpcError::AppError(AppMessage::NotFound("User".into())).into_status(),
            GrpcError::Std(Box::<dyn std::error::Error + Send + Sync>::from("boom")).into_status(),
            GrpcError::Status(Status::not_found("gone")).into_status(),
        ];
        for status in statuses {
            let value: serde_json::Value = serde_json::from_slice(status.details())
                .unwrap_or_else(|e| panic!("details must be JSON envelope: {e}"));
            assert_eq!(value["success"], serde_json::Value::Bool(false));
            assert!(
                value["code"].as_str().is_some_and(|c| c.len() == 3),
                "code must be a 3-char string, got {value}"
            );
            assert!(value["message"].is_string());
            assert!(value["timestamp"].is_number());
            assert!(value.get("data").is_some());
        }
    }

    #[test]
    fn envelope_key_order_is_pinned() {
        let status = GrpcError::AppError(AppMessage::NotFound("User".into())).into_status();
        let text = String::from_utf8(status.details().to_vec()).unwrap();
        assert!(
            text.starts_with(r#"{"code":"006","success":false,"timestamp":"#),
            "unexpected envelope prefix: {text}"
        );
        assert!(
            text.ends_with(r#","message":"User","data":null}"#),
            "unexpected envelope suffix: {text}"
        );
    }

    #[test]
    fn status_details_that_are_already_an_envelope_are_preserved() {
        let original = GrpcError::AppError(AppMessage::NotFound("Order".into())).into_status();
        let details = original.details().to_vec();
        let again = GrpcError::Status(Status::with_details(
            Code::NotFound,
            "Order",
            tonic::codegen::Bytes::from(details.clone()),
        ))
        .into_status();
        assert_eq!(again.details(), details.as_slice());
    }

    #[test]
    fn foreign_status_details_are_left_alone() {
        // A non-envelope payload (e.g. a protobuf ErrorInfo) must survive untouched.
        let payload = b"\x0a\x12some-protobuf-bytes";
        let status = GrpcError::Status(Status::with_details(
            Code::InvalidArgument,
            "bad",
            tonic::codegen::Bytes::from(payload.to_vec()),
        ))
        .into_status();
        assert_eq!(status.details(), payload.as_slice());
        assert_eq!(status.code(), Code::InvalidArgument);
    }

    #[test]
    fn bare_status_gets_an_envelope() {
        // Status::not_found(..) carries no details, so it must gain the envelope.
        let status = GrpcError::Status(Status::not_found("gone")).into_status();
        let value: serde_json::Value = serde_json::from_slice(status.details()).unwrap();
        assert_eq!(value["code"], "006");
        assert_eq!(value["message"], "gone");
    }
}
