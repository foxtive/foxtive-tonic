use crate::contracts::ResponseCodeContract;
use crate::enums::response_code::ResponseCode;
use crate::error::GrpcError;
use crate::helpers::json_message::JsonMessage;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

/// Builder for standardized gRPC responses.
///
/// All methods return responses with a consistent JSON envelope containing
/// `code`, `success`, `timestamp`, `message`, and `data` fields.
pub struct Responder;

/// A standardized JSON response envelope for gRPC (serializable).
#[derive(Debug, Serialize, Deserialize)]
pub struct GrpcResponse<T: Serialize> {
    /// Application-specific response code.
    pub code: String,
    /// Whether the operation was successful.
    pub success: bool,
    /// Unix timestamp of the response.
    pub timestamp: u64,
    /// Optional human-readable message.
    pub message: Option<String>,
    /// The response payload.
    pub data: T,
}

impl<T: Serialize> Display for GrpcResponse<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        // A Display that returns Err would panic format!() in the caller, so on
        // failure log and emit the same minimal envelope every other path uses.
        match serde_json::to_string(self) {
            Ok(json) => f.write_str(&json),
            Err(err) => {
                tracing::error!("failed to serialize response envelope: {err}");
                f.write_str(FALLBACK_ENVELOPE)
            }
        }
    }
}

/// Minimal envelope emitted when serializing a real envelope fails.
///
/// Hand-written so the fallback cannot itself fail; key order matches
/// [`GrpcResponse`]'s fields, and `timestamp` is `0` (a constant cannot call
/// [`foxtive::helpers::time::current_timestamp`]).
pub(crate) const FALLBACK_ENVELOPE: &str =
    r#"{"code":"009","success":false,"timestamp":0,"message":"Internal Server Error","data":null}"#;

/// Serialize `value`, degrading to [`FALLBACK_ENVELOPE`] instead of an empty body.
///
/// An empty string is indistinguishable from a successful empty payload on the wire.
pub(crate) fn serialize_or_internal<T: Serialize>(value: &T, context: &str) -> String {
    match serde_json::to_string(value) {
        Ok(json) => json,
        Err(err) => {
            tracing::error!("{context}: failed to serialize response envelope: {err}");
            FALLBACK_ENVELOPE.to_owned()
        }
    }
}

/// Type alias for gRPC handler results.
pub type GrpcResult<T> = Result<T, GrpcError>;

#[allow(dead_code)]
impl Responder {
    /// Send data with a response code and message.
    pub fn send_msg<C, D>(data: D, code: C, msg: impl Into<String>) -> String
    where
        C: ResponseCodeContract,
        D: Serialize,
    {
        let envelope = JsonMessage::make(data, code.code(), code.success(), Some(msg.into()));
        serialize_or_internal(&envelope, "Responder::send_msg")
    }

    /// Send data with a response code (no message).
    pub fn send<C, D>(data: D, code: C) -> String
    where
        C: ResponseCodeContract,
        D: Serialize,
    {
        let envelope = JsonMessage::make(data, code.code(), code.success(), None);
        serialize_or_internal(&envelope, "Responder::send")
    }

    /// Send a success response with a message.
    pub fn ok_message(msg: &str) -> String {
        Self::message(msg, ResponseCode::Ok)
    }

    /// Send a 400 Bad Request response with a message.
    pub fn bad_req_message(msg: &str) -> String {
        Self::message(msg, ResponseCode::BadRequest)
    }

    /// Send a 404 Not Found response with a message.
    pub fn not_found_message(msg: &str) -> String {
        Self::message(msg, ResponseCode::NotFound)
    }

    /// Send a 404 Not Found response describing a missing entity.
    pub fn entity_not_found_message(entity: &str) -> String {
        let msg = format!("Such {entity} does not exists");
        Self::not_found_message(&msg)
    }

    /// Send a 500 Internal Server Error response with a message.
    pub fn internal_server_error_message(msg: &str) -> String {
        Self::message(msg, ResponseCode::Internal)
    }

    /// Send a 404 Not Found response with default message.
    pub fn not_found() -> String {
        Self::not_found_message("Not Found")
    }

    /// Send a 500 Internal Server Error response with default message.
    pub fn internal_server_error() -> String {
        Self::internal_server_error_message("Internal Server Error")
    }

    /// Send a response with a message and response code.
    pub fn message<C: ResponseCodeContract>(msg: &str, code: C) -> String {
        let envelope = JsonMessage::make((), code.code(), code.success(), Some(msg.to_owned()));
        serialize_or_internal(&envelope, "Responder::message")
    }

    /// Serialize data to a JSON string for embedding in gRPC responses.
    pub fn respond<T: Serialize>(data: T) -> String {
        serialize_or_internal(&data, "Responder::respond")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enums::response_code::ResponseCode;
    use serde_json::Value;

    #[derive(Serialize)]
    struct Payload {
        id: u32,
        name: String,
    }

    /// `(constructor description, produced envelope, code used)`.
    fn envelopes() -> Vec<(String, String, String)> {
        let payload = Payload {
            id: 7,
            name: "thing".into(),
        };
        [
            (
                "send".to_owned(),
                Responder::send(&payload, ResponseCode::Ok),
                ResponseCode::Ok.code().to_owned(),
            ),
            (
                "send_msg".to_owned(),
                Responder::send_msg(&payload, ResponseCode::Created, "created"),
                ResponseCode::Created.code().to_owned(),
            ),
            (
                "ok_message".to_owned(),
                Responder::ok_message("fine"),
                ResponseCode::Ok.code().to_owned(),
            ),
            (
                "bad_req_message".to_owned(),
                Responder::bad_req_message("nope"),
                ResponseCode::BadRequest.code().to_owned(),
            ),
            (
                "not_found_message".to_owned(),
                Responder::not_found_message("missing"),
                ResponseCode::NotFound.code().to_owned(),
            ),
            (
                "entity_not_found_message".to_owned(),
                Responder::entity_not_found_message("user"),
                ResponseCode::NotFound.code().to_owned(),
            ),
            (
                "internal_server_error_message".to_owned(),
                Responder::internal_server_error_message("melted"),
                ResponseCode::Internal.code().to_owned(),
            ),
            (
                "not_found".to_owned(),
                Responder::not_found(),
                ResponseCode::NotFound.code().to_owned(),
            ),
            (
                "internal_server_error".to_owned(),
                Responder::internal_server_error(),
                ResponseCode::Internal.code().to_owned(),
            ),
            (
                "message".to_owned(),
                Responder::message("hi", ResponseCode::Accepted),
                ResponseCode::Accepted.code().to_owned(),
            ),
        ]
        .to_vec()
    }

    #[test]
    fn success_flag_matches_the_codes_success_contract() {
        for (name, envelope, code) in envelopes() {
            let value: Value = serde_json::from_str(&envelope)
                .unwrap_or_else(|e| panic!("{name} must produce JSON: {e}"));
            let expected = ResponseCode::from_code(&code).success();
            assert_eq!(
                value["success"].as_bool(),
                Some(expected),
                "{name} with code {code}"
            );
            assert_eq!(value["code"].as_str(), Some(code.as_str()), "{name}");
            assert!(
                value["timestamp"].is_number(),
                "{name} must carry a timestamp"
            );
        }
    }

    #[test]
    fn display_equals_serialized_form() {
        let envelope: GrpcResponse<Payload> = JsonMessage::make(
            Payload {
                id: 1,
                name: "a".into(),
            },
            ResponseCode::Ok.code(),
            true,
            None,
        );
        assert_eq!(
            envelope.to_string(),
            serde_json::to_string(&envelope).unwrap()
        );
    }

    #[test]
    fn envelope_field_order_is_pinned() {
        // Consumers parse this shape; a reordering is a wire-contract change.
        let envelope = Responder::send_msg(
            Payload {
                id: 1,
                name: "a".into(),
            },
            ResponseCode::Ok,
            "m",
        );
        assert!(
            envelope.starts_with(r#"{"code":"000","success":true,"timestamp":"#),
            "unexpected envelope prefix: {envelope}"
        );
        assert!(
            envelope.contains(r#","message":"m","data":{"id":1,"name":"a"}}"#),
            "unexpected envelope tail: {envelope}"
        );
    }

    #[test]
    fn fallback_envelope_is_valid_and_marked_as_internal_error() {
        let value: Value = serde_json::from_str(FALLBACK_ENVELOPE).unwrap();
        assert_eq!(value["code"], "009");
        assert_eq!(value["success"], Value::Bool(false));
        assert!(value["message"].is_string());
        assert!(value["data"].is_null());
    }

    #[test]
    fn respond_returns_the_payload_without_an_envelope() {
        let text = Responder::respond(Payload {
            id: 3,
            name: "x".into(),
        });
        assert_eq!(text, r#"{"id":3,"name":"x"}"#);
    }
}
