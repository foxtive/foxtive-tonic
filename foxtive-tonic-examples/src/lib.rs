//! Shared protos and services for the `foxtive-tonic` examples and integration
//! tests.
//!
//! This crate is not published, and every `.proto` is compiled here so the
//! library can ship without a build script (`cargo build -p foxtive-tonic` needs
//! no `protoc`). Examples and tests share these types so a scenario is written
//! once.

/// Generated code, one module per `.proto`.
pub mod proto {
    /// `helloworld.Greeter`, the plain request/reply service most examples serve.
    pub mod helloworld {
        tonic::include_proto!("helloworld");

        /// Encoded `helloworld.proto`, for a reflection service.
        pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("helloworld");
    }

    /// `foxtive.tonic.examples.v1.Demo`, typed envelopes and lifecycle scenarios.
    ///
    /// Its `Envelope` is not generated: `build.rs` maps the contract onto
    /// [`foxtive_tonic::grpc::envelope::Envelope`] with `extern_path`.
    pub mod demo {
        tonic::include_proto!("foxtive.tonic.examples.v1");

        /// Encoded `demo.proto` plus its imports (so also
        /// `foxtive/tonic/v1/envelope.proto`), the descriptor the envelope tests read.
        pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("demo");
    }
}

/// A second, independent build of `foxtive/tonic/v1/envelope.proto`.
///
/// Unlike [`proto::demo`], this has no `extern_path`, so prost generates its own
/// `Envelope`. `tests/envelope.rs` encodes with the library's hand-written struct
/// and compares the bytes with this type's, proving the two describe the same
/// message: names, tags, and wire types alike.
pub mod envelope_shadow {
    include!(concat!(
        env!("OUT_DIR"),
        "/envelope_shadow/foxtive.tonic.v1.rs"
    ));
}

pub mod services;
pub mod test_support;
