//! Lossy conversions between `serde_json::Value` and `prost_types::Value`, used
//! by the typed [`Envelope`](crate::grpc::envelope::Envelope) payload. The
//! mapping follows protobuf's canonical JSON form, with these caveats:
//!
//! * `google.protobuf.Value` has only `double` for numbers, so integers above
//!   `2^53` lose precision in either direction (a `u64` id of
//!   `9_007_199_254_740_993` arrives as `9_007_199_254_740_992`). Payloads that
//!   need integer or decimal exactness belong in a typed proto message.
//! * JSON separates `42` from `42.0` and the wire type does not: a number with
//!   no fractional part is handed back as an integer, so `42.0` round-trips to
//!   `42`.
//! * `f64::NAN` and the infinities have no JSON representation; they convert to
//!   `Value::Null` rather than failing.
//!
//! Nulls, booleans, strings, arrays, objects and nesting round-trip unchanged.

use prost_types::value::Kind;
use prost_types::{ListValue, NullValue, Struct, Value};

/// The `null` member of the `Value` union, as its wire (`i32`) representation.
const PROTO_NULL: i32 = NullValue::NullValue as i32;

/// Convert a JSON value into its protobuf `Value` equivalent.
///
/// Numbers become [`Kind::NumberValue`] as `f64`; see the module docs for the
/// precision this costs above `2^53`.
pub fn json_to_proto_value(value: serde_json::Value) -> Value {
    let kind = match value {
        serde_json::Value::Null => Kind::NullValue(PROTO_NULL),
        serde_json::Value::Bool(flag) => Kind::BoolValue(flag),
        serde_json::Value::Number(number) => match number.as_f64() {
            // as_f64 is None only for arbitrary-precision integers, which are out
            // of range for the wire type, so they decode as null.
            Some(double) if double.is_finite() => Kind::NumberValue(double),
            _ => Kind::NullValue(PROTO_NULL),
        },
        serde_json::Value::String(text) => Kind::StringValue(text),
        serde_json::Value::Array(items) => Kind::ListValue(ListValue {
            values: items.into_iter().map(json_to_proto_value).collect(),
        }),
        serde_json::Value::Object(fields) => Kind::StructValue(Struct {
            fields: fields
                .into_iter()
                .map(|(key, value)| (key, json_to_proto_value(value)))
                .collect(),
        }),
    };

    Value { kind: Some(kind) }
}

/// Convert a protobuf `Value` back into JSON.
///
/// A `Value` with no `kind` set is a protocol error on the producer's side; it
/// decodes as `serde_json::Value::Null` so that reading a response never fails
/// because of a malformed upstream.
pub fn proto_to_json_value(value: &Value) -> serde_json::Value {
    match value.kind.as_ref() {
        None | Some(Kind::NullValue(_)) => serde_json::Value::Null,
        Some(Kind::BoolValue(flag)) => serde_json::Value::Bool(*flag),
        Some(Kind::NumberValue(double)) => number_to_json(*double),
        Some(Kind::StringValue(text)) => serde_json::Value::String(text.clone()),
        Some(Kind::ListValue(list)) => {
            serde_json::Value::Array(list.values.iter().map(proto_to_json_value).collect())
        }
        Some(Kind::StructValue(reference)) => serde_json::Value::Object(
            reference
                .fields
                .iter()
                .map(|(key, value)| (key.clone(), proto_to_json_value(value)))
                .collect(),
        ),
    }
}

/// Turn a wire `double` back into the closest JSON number.
///
/// `serde_json::Number` keeps integers and floats apart and compares them by
/// variant, so an integral `f64` that is exact is narrowed back to an integer;
/// without this, `42` would come back as `42.0`.
fn number_to_json(double: f64) -> serde_json::Value {
    /// Every integer in this range has exactly one `f64`, so the cast loses nothing.
    const EXACT_INTEGERS: f64 = 9_007_199_254_740_992.0; // 2^53

    if double.fract() == 0.0 && double.abs() <= EXACT_INTEGERS {
        return serde_json::Value::Number(serde_json::Number::from(double as i64));
    }

    serde_json::Number::from_f64(double)
        .map(serde_json::Value::Number)
        .unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value as Json, json};

    fn round_trip(input: Json) -> Json {
        proto_to_json_value(&json_to_proto_value(input.clone()))
    }

    #[test]
    fn scalars_round_trip() {
        assert_eq!(round_trip(Json::Null), Json::Null);
        assert_eq!(round_trip(json!(true)), json!(true));
        assert_eq!(round_trip(json!(false)), json!(false));
        assert_eq!(round_trip(json!("hello")), json!("hello"));
        assert_eq!(round_trip(json!("")), json!(""));
        assert_eq!(round_trip(json!(0)), json!(0));
        assert_eq!(round_trip(json!(-7)), json!(-7));
        assert_eq!(round_trip(json!(42)), json!(42));
    }

    #[test]
    fn floats_round_trip() {
        assert_eq!(round_trip(json!(0.5)), json!(0.5));
        assert_eq!(round_trip(json!(-0.125)), json!(-0.125));
        assert_eq!(round_trip(json!(1e30)), json!(1e30));
        // The wire type has one number kind, so an integral float comes back
        // as an integer.
        assert_eq!(round_trip(json!(3.0)), json!(3));
    }

    #[test]
    fn empty_containers_round_trip() {
        assert_eq!(round_trip(json!({})), json!({}));
        assert_eq!(round_trip(json!([])), json!([]));
        assert_eq!(
            round_trip(json!({ "items": [], "meta": {} })),
            json!({ "items": [], "meta": {} })
        );
    }

    #[test]
    fn nesting_round_trip_including_empty_containers() {
        let input = json!({
            "users": [
                { "id": 1, "tags": [], "profile": { "nick": "a", "active": true } },
                { "id": 2, "tags": ["x", "y"], "profile": null },
            ],
            "page": { "size": 10, "cursor": null },
            "deep": [[[["leaf"]]]],
        });
        assert_eq!(round_trip(input.clone()), input);
    }

    #[test]
    fn integers_above_two_pow_53_lose_precision() {
        // Documented limitation of `number_value: f64`.
        let exact: u64 = 9_007_199_254_740_993; // 2^53 + 1
        let back = round_trip(json!(exact));
        assert_eq!(back, json!(9_007_199_254_740_992u64));

        // Inside the safe range nothing is lost.
        assert_eq!(
            round_trip(json!(4_503_599_627_370_496u64)),
            json!(4_503_599_627_370_496u64)
        );
    }

    #[test]
    fn non_finite_floats_become_null_instead_of_failing() {
        // `serde_json` cannot hold a non-finite number, so the only way one reaches a
        // decoder is a producer that wrote the double straight onto the wire.
        for double in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let decoded = proto_to_json_value(&Value {
                kind: Some(Kind::NumberValue(double)),
            });
            assert_eq!(decoded, Json::Null, "{double} must degrade to null");
        }

        // Encoding a JSON null still produces a well-formed union member.
        assert_eq!(
            json_to_proto_value(Json::Null).kind,
            Some(Kind::NullValue(0))
        );
    }

    #[test]
    fn a_value_without_a_kind_decodes_as_null() {
        assert_eq!(proto_to_json_value(&Value { kind: None }), Json::Null);
    }

    #[test]
    fn proto_strings_and_structs_survive_in_both_directions() {
        let proto = Value {
            kind: Some(Kind::StructValue(Struct {
                fields: [
                    ("a".to_owned(), json_to_proto_value(json!(1))),
                    ("b".to_owned(), json_to_proto_value(json!("two"))),
                ]
                .into_iter()
                .collect(),
            })),
        };
        assert_eq!(proto_to_json_value(&proto), json!({ "a": 1, "b": "two" }));
        // Re-encoding the decoded form reaches the same value, modulo map ordering.
        assert_eq!(json_to_proto_value(proto_to_json_value(&proto)), proto);
    }
}
