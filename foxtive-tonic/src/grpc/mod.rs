//! gRPC response builder, extension traits, and standardized envelopes.

/// Typed protobuf response envelope, wire-compatible with the JSON one.
pub mod envelope;
/// Standardized gRPC response builder and envelope types.
pub mod responder;
/// Extension traits for ergonomic gRPC responses.
pub mod response;
