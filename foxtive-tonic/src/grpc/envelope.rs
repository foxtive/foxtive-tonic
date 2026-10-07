//! The typed counterpart of the JSON envelope produced by
//! [`Responder`](crate::grpc::responder::Responder).
//!
//! `Responder` answers with a `String` holding JSON. `Envelope` carries the same
//! five fields as a real protobuf message, so a gRPC-only consumer gets a schema
//! instead of a string it has to parse:
//!
//! ```text
//! Envelope { code: "000", success: true, timestamp: 1763000000, message, data }
//! ```
//!
//! Field numbers match `proto/foxtive/tonic/v1/envelope.proto`.

use crate::contracts::ResponseCodeContract;
use crate::enums::response_code::ResponseCode;
use crate::error::GrpcError;
use crate::grpc::responder::GrpcResponse;
use crate::helpers::json_message::JsonMessage;
use crate::helpers::proto_json::{json_to_proto_value, proto_to_json_value};
use serde::Serialize;

/// A standardized gRPC response envelope, as a protobuf message.
///
/// Wire-compatible with a `google.protobuf.Value` payload; the `data` field is
/// filled through [`json_to_proto_value`], so the numeric caveats documented on
/// [`helpers::proto_json`](crate::helpers::proto_json) apply to it.
///
/// Build one with [`Envelope::ok`], [`Envelope::with_code`],
/// [`Envelope::with_msg`] or [`Envelope::message_only`], and name the handler return
/// type [`EnvelopeResult`].
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Envelope {
    /// Application-specific response code, matching tag 1 of `envelope.proto`.
    #[prost(string, tag = "1")]
    pub code: String,
    /// Whether the call succeeded; mirrors [`ResponseCodeContract::success`].
    #[prost(bool, tag = "2")]
    pub success: bool,
    /// Unix seconds at which the response was produced.
    #[prost(uint64, tag = "3")]
    pub timestamp: u64,
    /// Optional human-readable message.
    #[prost(string, optional, tag = "4")]
    pub message: Option<String>,
    /// The response payload, or absent for a message-only envelope.
    #[prost(message, optional, tag = "5")]
    pub data: Option<prost_types::Value>,
}

/// Result alias for a service method that answers with an [`Envelope`].
///
/// A handler ends with `Ok(tonic::Response::new(Envelope::ok(&payload)?))`; a
/// [`GrpcError`] converts into a [`tonic::Status`] through its `From` impl.
pub type EnvelopeResult = Result<tonic::Response<Envelope>, tonic::Status>;

impl Envelope {
    /// Success envelope (`ResponseCode::Ok`) carrying `data`.
    pub fn ok<T: Serialize>(data: &T) -> Result<Self, GrpcError> {
        Self::with_code(data, ResponseCode::Ok)
    }

    /// Envelope carrying `data` under an explicit response code.
    pub fn with_code<T: Serialize, C: ResponseCodeContract>(
        data: &T,
        code: C,
    ) -> Result<Self, GrpcError> {
        Self::build(data, code, None)
    }

    /// Envelope carrying `data`, an explicit code, and a message.
    pub fn with_msg<T: Serialize, C: ResponseCodeContract>(
        data: &T,
        code: C,
        msg: impl Into<String>,
    ) -> Result<Self, GrpcError> {
        Self::build(data, code, Some(msg.into()))
    }

    /// Envelope with a message and no payload, for acknowledgements.
    ///
    /// `data` is left absent rather than set to `null`: on the wire "no payload"
    /// and "a null payload" are distinguishable, and this one is the former.
    pub fn message_only<C: ResponseCodeContract>(msg: impl Into<String>, code: C) -> Self {
        Self {
            code: code.code().to_owned(),
            success: code.success(),
            timestamp: foxtive::helpers::time::current_timestamp(),
            message: Some(msg.into()),
            data: None,
        }
    }

    /// The failure a [`GrpcError`] reports, as a [`tonic::Status`].
    ///
    /// Identical to [`GrpcError::into_status`]: the JSON envelope goes into
    /// `details`, so both the string and the typed API report failures the same way.
    pub fn error(err: GrpcError) -> tonic::Status {
        err.into_status()
    }

    /// The payload as JSON, or `null` when the envelope carries none.
    pub fn data_json(&self) -> serde_json::Value {
        self.data
            .as_ref()
            .map(proto_to_json_value)
            .unwrap_or(serde_json::Value::Null)
    }

    /// Delegates to [`TryFrom<&GrpcResponse<T>>`] so both envelope styles stay
    /// consistent.
    fn build<T: Serialize, C: ResponseCodeContract>(
        data: &T,
        code: C,
        message: Option<String>,
    ) -> Result<Self, GrpcError> {
        let envelope = JsonMessage::make(data, code.code(), code.success(), message);
        Self::try_from(&envelope)
    }
}

/// Encode `data` for the [`Envelope::data`] field.
///
/// A value serde cannot represent as JSON (non-string map keys, `f64::NAN`, a
/// failing `Serialize` impl) is reported as an error instead of downgraded to a
/// different payload.
fn proto_data<T: Serialize>(data: &T) -> Result<prost_types::Value, GrpcError> {
    let json = serde_json::to_value(data).map_err(|err| GrpcError::Std(Box::new(err)))?;
    Ok(json_to_proto_value(json))
}

impl<T: Serialize> TryFrom<&GrpcResponse<T>> for Envelope {
    type Error = GrpcError;

    fn try_from(envelope: &GrpcResponse<T>) -> Result<Self, Self::Error> {
        Ok(Self {
            code: envelope.code.clone(),
            success: envelope.success,
            timestamp: envelope.timestamp,
            message: envelope.message.clone(),
            data: Some(proto_data(&envelope.data)?),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enums::response_code::ResponseCode;
    use serde_json::json;

    #[derive(Serialize, PartialEq, Debug)]
    struct Payload {
        id: u32,
        name: String,
    }

    fn payload() -> Payload {
        Payload {
            id: 7,
            name: "thing".into(),
        }
    }

    #[test]
    fn ok_carries_the_payload_under_code_000() {
        let envelope = Envelope::ok(&payload()).unwrap();

        assert_eq!(envelope.code, "000");
        assert!(envelope.success);
        assert!(envelope.timestamp > 0);
        assert_eq!(envelope.message, None);
        assert_eq!(envelope.data_json(), json!({ "id": 7, "name": "thing" }));
    }

    #[test]
    fn the_code_decides_the_success_flag() {
        for (code, expected) in [
            (ResponseCode::Created, true),
            (ResponseCode::Accepted, true),
            (ResponseCode::NotFound, false),
            (ResponseCode::Internal, false),
            (ResponseCode::Cancelled, false),
        ] {
            let expected_code = code.code().to_owned();
            let envelope = Envelope::with_code(&payload(), code).unwrap();
            assert_eq!(envelope.code, expected_code);
            assert_eq!(envelope.success, expected, "{expected_code}");
        }
    }

    #[test]
    fn with_msg_keeps_code_data_and_message_apart() {
        let envelope =
            Envelope::with_msg(&payload(), ResponseCode::Created, "widget created").unwrap();

        assert_eq!(envelope.code, "001");
        assert_eq!(envelope.message.as_deref(), Some("widget created"));
        assert_eq!(envelope.data_json(), json!({ "id": 7, "name": "thing" }));
    }

    #[test]
    fn a_message_only_envelope_leaves_data_absent() {
        let envelope = Envelope::message_only("deleted", ResponseCode::Ok);

        assert_eq!(envelope.code, "000");
        assert!(envelope.success);
        assert_eq!(envelope.message.as_deref(), Some("deleted"));
        assert!(
            envelope.data.is_none(),
            "no payload must not be encoded as a null payload"
        );
        assert_eq!(envelope.data_json(), serde_json::Value::Null);
    }

    #[test]
    fn matches_the_string_envelope_field_for_field() {
        let data = payload();
        let typed = Envelope::with_msg(&data, ResponseCode::Accepted, "queued").unwrap();
        let text: serde_json::Value = serde_json::from_str(
            &crate::grpc::responder::Responder::send_msg(&data, ResponseCode::Accepted, "queued"),
        )
        .unwrap();

        assert_eq!(typed.code, text["code"].as_str().unwrap());
        assert_eq!(typed.success, text["success"].as_bool().unwrap());
        assert_eq!(typed.message.as_deref(), text["message"].as_str());
        assert_eq!(typed.data_json(), text["data"]);
    }

    #[test]
    fn try_from_a_grpc_response_reuses_its_timestamp() {
        let response = JsonMessage::make(
            payload(),
            ResponseCode::Ok.code(),
            true,
            Some("hi".to_owned()),
        );
        let envelope = Envelope::try_from(&response).unwrap();

        assert_eq!(envelope.timestamp, response.timestamp);
        assert_eq!(envelope.code, response.code);
        assert_eq!(envelope.message, response.message);
    }

    #[test]
    fn an_unserializable_payload_is_an_error_not_a_silent_null() {
        // Maps with non-string keys have no JSON object form.
        let bad: std::collections::BTreeMap<(u32, u32), &str> =
            [((1, 2), "x")].into_iter().collect();

        let err = Envelope::ok(&bad).expect_err("a map with tuple keys has no JSON form");
        let status = Envelope::error(err);
        assert_eq!(status.code(), tonic::Code::Internal);
    }

    #[test]
    fn encodes_and_survives_a_prost_round_trip() {
        use prost::Message as _;

        let envelope =
            Envelope::with_msg(&json!({ "nested": [1, 2, 3] }), ResponseCode::Ok, "n").unwrap();
        let mut buffer = Vec::new();
        envelope.encode(&mut buffer).unwrap();

        let decoded = Envelope::decode(buffer.as_slice()).unwrap();
        assert_eq!(decoded, envelope);
        assert_eq!(decoded.data_json(), json!({ "nested": [1, 2, 3] }));
    }

    #[test]
    fn error_reports_the_json_envelope_like_every_other_path() {
        let status = Envelope::error(GrpcError::AppError(foxtive::prelude::AppMessage::NotFound(
            "User".into(),
        )));

        assert_eq!(status.code(), tonic::Code::NotFound);
        let details: serde_json::Value = serde_json::from_slice(status.details()).unwrap();
        assert_eq!(details["code"], "006");
        assert_eq!(details["success"], serde_json::Value::Bool(false));
    }
}
