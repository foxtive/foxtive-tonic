//! The envelope contract, checked against a listening server and against the
//! `.proto` the library ships.
//!
//! `foxtive-tonic` has no build script, so its `Envelope` is hand-written prost,
//! and nothing in the normal build proves it matches
//! `proto/foxtive/tonic/v1/envelope.proto`. Three independent checks close that gap.
//!
//! * a value the library encodes must decode as the message generated from the
//!   `.proto`, and re-encode to the same bytes;
//! * the other way round, so a field added to the `.proto` alone is caught;
//! * the names, tags, types and presence of every field, read out of the encoded
//!   descriptor `demo.proto` carries.

use foxtive_tonic::contracts::ResponseCodeContract;
use foxtive_tonic::enums::response_code::ResponseCode;
use foxtive_tonic::grpc::envelope::Envelope;
use foxtive_tonic::helpers::proto_json::json_to_proto_value;
use foxtive_tonic_examples::envelope_shadow;
use foxtive_tonic_examples::proto::demo::demo_server::DemoServer;
use foxtive_tonic_examples::proto::demo::{
    AckRequest, AppFailureRequest, EchoRequest, FILE_DESCRIPTOR_SET,
};
use foxtive_tonic_examples::services::DemoService;
use foxtive_tonic_examples::test_support::{TestResult, app, demo_client, server};
use prost::Message as _;
use prost_types::field_descriptor_proto::Type;
use prost_types::{FileDescriptorSet, value};
use serde_json::{Value, json};
use tonic::Code;
use tonic::service::Routes;

/// A handler's `GrpcError::AppError` reaches the client as a `Status` whose
/// details are the same JSON envelope every other failure path produces.
#[tokio::test(flavor = "multi_thread")]
async fn an_app_failure_reports_the_json_envelope_in_its_status() -> TestResult {
    let handle = server(&app("envelope-failure").await)
        .configure(|| Routes::new(DemoServer::new(DemoService)))
        .spawn()
        .await?;

    let mut client = demo_client(handle.local_addr()).await;
    let status = client
        .app_failure(AppFailureRequest {
            entity: "User".to_string(),
        })
        .await
        .expect_err("the handler always fails");

    assert_eq!(status.code(), Code::NotFound);

    let details: Value = serde_json::from_slice(status.details())?;
    assert_eq!(details["code"], "006");
    assert_eq!(details["success"], Value::Bool(false));
    assert_eq!(details["message"], "User");
    assert!(
        details["data"].is_null(),
        "an error envelope carries no payload: {details}"
    );

    // The timestamp is part of the contract, so a client can tell a cached error
    // from a fresh one.
    assert!(
        details["timestamp"].as_u64().is_some_and(|stamp| stamp > 0),
        "{details}"
    );

    handle.shutdown();
    Ok(handle.stopped().await?)
}

/// `Envelope::ok(&payload)` survives a real round trip, in both directions, as
/// the typed message rather than as a string to be parsed.
#[tokio::test(flavor = "multi_thread")]
async fn typed_envelope_round_trips_over_the_wire() -> TestResult {
    // Shapes `helpers::proto_json` must cover, kept to values exact as IEEE-754
    // doubles: an integer above 2^53 does not come back intact, a documented
    // `google.protobuf.Value` limitation that `proto_json`'s unit tests pin.
    let payload = json!({
        "id": 42,
        "name": "widget",
        "ratio": 0.25,
        "active": true,
        "missing": null,
        "tags": ["a", "b"],
        "nested": { "deep": { "leaf": [1, 2, 3] } },
        "empty_object": {},
        "empty_array": [],
    });

    let handle = server(&app("envelope-wire").await)
        .configure(|| Routes::new(DemoServer::new(DemoService)))
        .spawn()
        .await?;

    let mut client = demo_client(handle.local_addr()).await;
    let response = client
        .echo(EchoRequest {
            payload: payload.to_string(),
            code: ResponseCode::Ok.code().to_string(),
        })
        .await?
        .into_inner();

    // `extern_path` in `build.rs` makes this the library's own `Envelope`, so the
    // client never handles JSON bytes: `code`, `success` and `data` are fields.
    assert_eq!(response.code, "000");
    assert!(response.success);
    assert_eq!(response.message, None);
    assert_eq!(
        response.data_json(),
        payload,
        "the payload came back intact"
    );

    // A failing code over the same transport must flip `success`, so the typed path
    // cannot report a failure as a success.
    let response = client
        .echo(EchoRequest {
            payload: json!({"reason": "gone"}).to_string(),
            code: ResponseCode::NotFound.code().to_string(),
        })
        .await?
        .into_inner();
    assert_eq!(response.code, "006");
    assert!(!response.success);
    assert_eq!(response.data_json(), json!({"reason": "gone"}));

    // A payload that is not JSON is rejected before an envelope is built.
    let status = client
        .echo(EchoRequest {
            payload: "not json".to_string(),
            code: "000".to_string(),
        })
        .await
        .expect_err("the handler rejects a malformed payload");
    assert_eq!(status.code(), Code::InvalidArgument);

    handle.shutdown();
    Ok(handle.stopped().await?)
}

/// An acknowledgement carries a message and no payload, and "no payload" has to
/// stay distinguishable from "a payload that is JSON null" across the socket.
#[tokio::test(flavor = "multi_thread")]
async fn an_absent_payload_is_not_a_null_payload() -> TestResult {
    let handle = server(&app("envelope-ack").await)
        .configure(|| Routes::new(DemoServer::new(DemoService)))
        .spawn()
        .await?;

    let mut client = demo_client(handle.local_addr()).await;

    let ack = client
        .ack(AckRequest {
            message: "deleted".to_string(),
        })
        .await?
        .into_inner();
    assert_eq!(ack.message.as_deref(), Some("deleted"));
    assert!(
        ack.data.is_none(),
        "an ack has no payload, so none should have been encoded"
    );

    let nulls = client
        .echo(EchoRequest {
            payload: "null".to_string(),
            code: "000".to_string(),
        })
        .await?
        .into_inner();
    assert_eq!(
        nulls.data.as_ref().and_then(|value| value.kind.as_ref()),
        Some(&value::Kind::NullValue(0)),
        "a null payload is present on the wire"
    );
    assert_eq!(nulls.data_json(), Value::Null);

    handle.shutdown();
    Ok(handle.stopped().await?)
}

/// Envelopes that cover each field's interesting values, built only through the
/// library's public constructors.
fn drift_cases() -> Vec<(&'static str, Envelope)> {
    let data = json!({ "id": 7, "items": [1, 2, 3] });

    vec![
        (
            "success with a payload",
            Envelope::ok(&data).expect("the payload is serializable"),
        ),
        (
            "failure with a message",
            Envelope::with_msg(&data, ResponseCode::NotFound, "User not found")
                .expect("the payload is serializable"),
        ),
        (
            "no message, no payload",
            Envelope::ok(&()).expect("the unit value is serializable"),
        ),
        (
            "an empty message, which is not no message",
            Envelope::with_msg(&data, ResponseCode::Accepted, "").expect("serializable"),
        ),
        (
            "a message-only acknowledgement",
            Envelope::message_only("deleted", ResponseCode::Ok),
        ),
        (
            "a code whose gRPC status is not Ok",
            Envelope::with_code(&data, ResponseCode::ResourceExhausted)
                .expect("the payload is serializable"),
        ),
    ]
}

/// The library's hand-written struct must produce bytes the message generated from
/// `envelope.proto` reads back as the same five fields.
#[test]
fn the_library_encodes_what_the_proto_decodes() -> TestResult {
    for (what, ours) in drift_cases() {
        let bytes = ours.encode_to_vec();
        let theirs = envelope_shadow::Envelope::decode(bytes.as_slice()).unwrap_or_else(|err| {
            panic!("{what}: the generated Envelope cannot read the library's bytes: {err}")
        });

        assert_eq!(theirs.code, ours.code, "{what}: code");
        assert_eq!(theirs.success, ours.success, "{what}: success");
        assert_eq!(theirs.timestamp, ours.timestamp, "{what}: timestamp");
        assert_eq!(theirs.message, ours.message, "{what}: message");
        assert_eq!(theirs.data, ours.data, "{what}: data");

        // Byte stability is the sharper half: prost omits a scalar still holding its
        // default, so if the two types disagreed about presence (say, `optional
        // string message` becoming plain `string`) the values would match while the
        // bytes came out different.
        assert_eq!(
            theirs.encode_to_vec(),
            bytes,
            "{what}: the two encodings are not interchangeable"
        );
    }

    Ok(())
}

/// And the other direction, which is the one that catches a field added to the
/// `.proto` but never to `src/grpc/envelope.rs`: our `decode` would drop it and our
/// `encode` would not put it back.
#[test]
fn the_proto_encodes_only_what_the_library_carries() -> TestResult {
    let theirs = envelope_shadow::Envelope {
        code: "006".to_string(),
        success: false,
        timestamp: 1_763_000_000,
        message: Some("User not found".to_string()),
        data: Some(json_to_proto_value(json!({ "id": 7 }))),
    };

    let bytes = theirs.encode_to_vec();
    let ours = Envelope::decode(bytes.as_slice())
        .expect("the library must be able to read what the proto writes");

    assert_eq!(ours.code, theirs.code);
    assert_eq!(ours.success, theirs.success);
    assert_eq!(ours.timestamp, theirs.timestamp);
    assert_eq!(ours.message, theirs.message);
    assert_eq!(ours.data_json(), json!({ "id": 7 }));
    assert_eq!(
        ours.encode_to_vec(),
        bytes,
        "the library lost a field the proto has, or reads it from a different tag"
    );

    Ok(())
}

/// The two checks above compare wire bytes, so they cannot see a *renamed* field:
/// `code` moved to tag 2 would encode identically in isolation. The encoded
/// descriptor `demo.proto` imports carries the names, and prost puts them in
/// declaration order, so reading them back pins the contract in prose too.
#[test]
fn the_shipped_proto_declares_the_documented_fields() -> TestResult {
    let set = FileDescriptorSet::decode(FILE_DESCRIPTOR_SET)?;
    let file = set
        .file
        .iter()
        .find(|file| file.name.as_deref() == Some("foxtive/tonic/v1/envelope.proto"))
        .expect("demo.proto imports envelope.proto, so its descriptor set must carry it");

    assert_eq!(file.package.as_deref(), Some("foxtive.tonic.v1"));
    assert!(
        file.dependency
            .iter()
            .any(|name| name == "google/protobuf/struct.proto"),
        "the payload is a google.protobuf.Value: {:?}",
        file.dependency
    );

    let envelope = file
        .message_type
        .iter()
        .find(|message| message.name.as_deref() == Some("Envelope"))
        .expect("the contract message must be named Envelope");

    let fields: Vec<(&str, i32, i32, bool)> = envelope
        .field
        .iter()
        .map(|field| {
            (
                field.name.as_deref().unwrap_or_default(),
                field.number.unwrap_or_default(),
                field.r#type.unwrap_or_default(),
                field.proto3_optional.unwrap_or(false),
            )
        })
        .collect();

    assert_eq!(
        fields,
        vec![
            ("code", 1, Type::String as i32, false),
            ("success", 2, Type::Bool as i32, false),
            ("timestamp", 3, Type::Uint64 as i32, false),
            ("message", 4, Type::String as i32, true),
            ("data", 5, Type::Message as i32, false),
        ],
        "name, tag, type and proto3 presence of every field of Envelope"
    );

    // `data` has to be a `Value`, not merely *a* message: the type name is the part
    // `extern_path` and `Envelope::data` both depend on.
    assert_eq!(
        envelope.field[4].type_name.as_deref(),
        Some(".google.protobuf.Value")
    );

    Ok(())
}
