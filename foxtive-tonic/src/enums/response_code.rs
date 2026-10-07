use crate::contracts::ResponseCodeContract;
use tonic::Code;

/// Standardized response codes mapping to gRPC status codes.
///
/// Each variant provides a string code (e.g. "000") and a corresponding gRPC
/// status code. All 17 canonical gRPC codes are represented, so a status never
/// degrades into `Internal` and loses its retryability semantics.
///
/// The HTTP status in each variant's comment is informational: gRPC carries the
/// status in `grpc-status`, not the HTTP status line. `Created` and `Accepted`
/// both map to `Code::Ok`, distinguished only by their string code.
#[derive(Clone)]
pub enum ResponseCode {
    /// Ok - operation succeeded.
    Ok,
    /// 201 Created - resource was created.
    Created,
    /// 202 Accepted - request accepted for processing.
    Accepted,
    /// 400 Bad Request - client sent an invalid request.
    BadRequest,
    /// 401 Unauthorized - authentication is required.
    Unauthenticated,
    /// 403 Forbidden - client lacks permission.
    PermissionDenied,
    /// 404 Not Found - resource does not exist.
    NotFound,
    /// 409 Conflict - request conflicts with current state.
    AlreadyExists,
    /// 429 Too Many Requests - rate limit exceeded.
    ResourceExhausted,
    /// 500 Internal Server Error - unexpected server failure.
    Internal,
    /// 501 Not Implemented - functionality not implemented.
    Unimplemented,
    /// 503 Service Unavailable - server is temporarily unavailable.
    Unavailable,
    /// 504 Deadline Exceeded - operation timed out.
    DeadlineExceeded,
    /// FAILED_PRECONDITION - retry after the system state changes (HTTP 400).
    FailedPrecondition,
    /// 499 Client Closed Request - cancelled by the caller (retryable).
    Cancelled,
    /// 409 Conflict - transaction aborted, retry after resolving the conflict.
    Aborted,
    /// 400 Bad Request - operation was performed on an out-of-range key.
    OutOfRange,
    /// 500 Internal Server Error - unrecoverable data loss or corruption.
    DataLoss,
    /// 500 Internal Server Error - unexpected state (`Code::Unknown`, not `Internal`).
    Unknown,
}

impl ResponseCodeContract for ResponseCode {
    fn code(&self) -> &str {
        match self {
            ResponseCode::Ok => "000",
            ResponseCode::Created => "001",
            ResponseCode::Accepted => "002",
            ResponseCode::BadRequest => "003",
            ResponseCode::Unauthenticated => "004",
            ResponseCode::PermissionDenied => "005",
            ResponseCode::NotFound => "006",
            ResponseCode::AlreadyExists => "007",
            ResponseCode::ResourceExhausted => "008",
            ResponseCode::Internal => "009",
            ResponseCode::Unimplemented => "010",
            ResponseCode::Unavailable => "011",
            ResponseCode::DeadlineExceeded => "012",
            ResponseCode::FailedPrecondition => "013",
            ResponseCode::Cancelled => "014",
            ResponseCode::Aborted => "015",
            ResponseCode::OutOfRange => "016",
            ResponseCode::DataLoss => "017",
            ResponseCode::Unknown => "018",
        }
    }

    fn status(&self) -> Code {
        match self {
            ResponseCode::Ok => Code::Ok,
            ResponseCode::Created => Code::Ok,
            ResponseCode::Accepted => Code::Ok,
            ResponseCode::BadRequest => Code::InvalidArgument,
            ResponseCode::Unauthenticated => Code::Unauthenticated,
            ResponseCode::PermissionDenied => Code::PermissionDenied,
            ResponseCode::NotFound => Code::NotFound,
            ResponseCode::AlreadyExists => Code::AlreadyExists,
            ResponseCode::ResourceExhausted => Code::ResourceExhausted,
            ResponseCode::Internal => Code::Internal,
            ResponseCode::Unimplemented => Code::Unimplemented,
            ResponseCode::Unavailable => Code::Unavailable,
            ResponseCode::DeadlineExceeded => Code::DeadlineExceeded,
            ResponseCode::FailedPrecondition => Code::FailedPrecondition,
            ResponseCode::Cancelled => Code::Cancelled,
            ResponseCode::Aborted => Code::Aborted,
            ResponseCode::OutOfRange => Code::OutOfRange,
            ResponseCode::DataLoss => Code::DataLoss,
            ResponseCode::Unknown => Code::Unknown,
        }
    }

    fn from_code(code: &str) -> Self {
        match code {
            "000" => ResponseCode::Ok,
            "001" => ResponseCode::Created,
            "002" => ResponseCode::Accepted,
            "003" => ResponseCode::BadRequest,
            "004" => ResponseCode::Unauthenticated,
            "005" => ResponseCode::PermissionDenied,
            "006" => ResponseCode::NotFound,
            "007" => ResponseCode::AlreadyExists,
            "008" => ResponseCode::ResourceExhausted,
            "009" => ResponseCode::Internal,
            "010" => ResponseCode::Unimplemented,
            "011" => ResponseCode::Unavailable,
            "012" => ResponseCode::DeadlineExceeded,
            "013" => ResponseCode::FailedPrecondition,
            "014" => ResponseCode::Cancelled,
            "015" => ResponseCode::Aborted,
            "016" => ResponseCode::OutOfRange,
            "017" => ResponseCode::DataLoss,
            "018" => ResponseCode::Unknown,
            _ => ResponseCode::Internal,
        }
    }

    fn from_status(status: Code) -> Self {
        match status {
            Code::Ok => ResponseCode::Ok,
            Code::InvalidArgument => ResponseCode::BadRequest,
            Code::Unauthenticated => ResponseCode::Unauthenticated,
            Code::PermissionDenied => ResponseCode::PermissionDenied,
            Code::NotFound => ResponseCode::NotFound,
            Code::AlreadyExists => ResponseCode::AlreadyExists,
            Code::ResourceExhausted => ResponseCode::ResourceExhausted,
            Code::Internal => ResponseCode::Internal,
            Code::Unimplemented => ResponseCode::Unimplemented,
            Code::Unavailable => ResponseCode::Unavailable,
            Code::DeadlineExceeded => ResponseCode::DeadlineExceeded,
            Code::FailedPrecondition => ResponseCode::FailedPrecondition,
            Code::Cancelled => ResponseCode::Cancelled,
            Code::Aborted => ResponseCode::Aborted,
            Code::OutOfRange => ResponseCode::OutOfRange,
            Code::DataLoss => ResponseCode::DataLoss,
            Code::Unknown => ResponseCode::Unknown,
            // Forward compatibility: Code is exhaustive today, but a new tonic variant
            // must degrade to Internal rather than fail to compile.
            #[allow(unreachable_patterns)]
            _ => ResponseCode::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    /// Every variant, in declaration order.
    const ALL: [(ResponseCode, &str, Code, bool); 19] = [
        (ResponseCode::Ok, "000", Code::Ok, true),
        (ResponseCode::Created, "001", Code::Ok, true),
        (ResponseCode::Accepted, "002", Code::Ok, true),
        (
            ResponseCode::BadRequest,
            "003",
            Code::InvalidArgument,
            false,
        ),
        (
            ResponseCode::Unauthenticated,
            "004",
            Code::Unauthenticated,
            false,
        ),
        (
            ResponseCode::PermissionDenied,
            "005",
            Code::PermissionDenied,
            false,
        ),
        (ResponseCode::NotFound, "006", Code::NotFound, false),
        (
            ResponseCode::AlreadyExists,
            "007",
            Code::AlreadyExists,
            false,
        ),
        (
            ResponseCode::ResourceExhausted,
            "008",
            Code::ResourceExhausted,
            false,
        ),
        (ResponseCode::Internal, "009", Code::Internal, false),
        (
            ResponseCode::Unimplemented,
            "010",
            Code::Unimplemented,
            false,
        ),
        (ResponseCode::Unavailable, "011", Code::Unavailable, false),
        (
            ResponseCode::DeadlineExceeded,
            "012",
            Code::DeadlineExceeded,
            false,
        ),
        (
            ResponseCode::FailedPrecondition,
            "013",
            Code::FailedPrecondition,
            false,
        ),
        (ResponseCode::Cancelled, "014", Code::Cancelled, false),
        (ResponseCode::Aborted, "015", Code::Aborted, false),
        (ResponseCode::OutOfRange, "016", Code::OutOfRange, false),
        (ResponseCode::DataLoss, "017", Code::DataLoss, false),
        (ResponseCode::Unknown, "018", Code::Unknown, false),
    ];

    #[test]
    fn code_string_and_status_and_success_per_variant() {
        for (variant, code, status, success) in ALL {
            assert_eq!(variant.code(), code, "code() for {code}");
            assert_eq!(variant.status(), status, "status() for {code}");
            assert_eq!(variant.success(), success, "success() for {code}");
        }
    }

    #[test]
    fn code_round_trips_through_from_code() {
        for (variant, code, _, _) in ALL {
            assert_eq!(
                ResponseCode::from_code(code).status(),
                variant.status(),
                "from_code({code}) must resolve to the same gRPC status"
            );
        }
    }

    #[test]
    fn status_round_trips_through_from_status() {
        // Ok/Created/Accepted all share Code::Ok, so from_status can only recover
        // Ok; every other gRPC status must come back as its own variant.
        for (_variant, _code, status, _) in ALL {
            let recovered = ResponseCode::from_status(status);
            if status == Code::Ok {
                assert_eq!(recovered.code(), "000");
            } else {
                assert_eq!(recovered.status(), status, "from_status({status:?})");
            }
        }
    }

    #[test]
    fn no_canonical_grpc_code_collapses_to_internal() {
        // No canonical gRPC code may collapse to Internal and lose its
        // retryability semantics.
        let all_grpc_codes = [
            Code::Ok,
            Code::Cancelled,
            Code::Unknown,
            Code::InvalidArgument,
            Code::DeadlineExceeded,
            Code::NotFound,
            Code::AlreadyExists,
            Code::PermissionDenied,
            Code::ResourceExhausted,
            Code::FailedPrecondition,
            Code::Aborted,
            Code::OutOfRange,
            Code::Unimplemented,
            Code::Internal,
            Code::Unavailable,
            Code::DataLoss,
            Code::Unauthenticated,
        ];
        for code in all_grpc_codes {
            let recovered = ResponseCode::from_status(code);
            if code == Code::Internal {
                continue;
            }
            assert_ne!(
                recovered.status(),
                Code::Internal,
                "{code:?} must not collapse to Internal"
            );
            assert_eq!(recovered.status(), code, "{code:?} must round-trip");
        }
    }

    #[test]
    fn unknown_code_strings_fall_back_to_internal() {
        for bogus in ["", "999", "abc", "019"] {
            assert_eq!(ResponseCode::from_code(bogus).code(), "009", "{bogus:?}");
        }
    }
}
