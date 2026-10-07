fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::env::var("OUT_DIR")?;

    // The plain request/reply service, with a descriptor set for reflection.
    tonic_prost_build::configure()
        .file_descriptor_set_path(format!("{out_dir}/helloworld.bin"))
        .compile_protos(&["proto/helloworld.proto"], &["proto/"])?;

    // The typed-envelope service. It imports the library's reference
    // `foxtive/tonic/v1/envelope.proto`, and `extern_path` points that message at
    // the hand-written `foxtive_tonic::grpc::envelope::Envelope` instead of
    // generating a second one, so a renamed or retagged field breaks the build
    // rather than silently changing the wire format.
    tonic_prost_build::configure()
        .extern_path(
            ".foxtive.tonic.v1.Envelope",
            "::foxtive_tonic::grpc::envelope::Envelope",
        )
        .file_descriptor_set_path(format!("{out_dir}/demo.bin"))
        .compile_protos(
            &["proto/demo.proto"],
            // The second entry is where `foxtive/tonic/v1/envelope.proto` lives.
            &["proto/", "../foxtive-tonic/proto/"],
        )?;

    // Third pass: a shadow build of the same contract with no `extern_path`, so a
    // test can encode with the hand-written struct and decode with prost code
    // generated from the `.proto`, pinning field names as well as tags and types.
    let shadow_dir = format!("{out_dir}/envelope_shadow");
    std::fs::create_dir_all(&shadow_dir)?;

    tonic_prost_build::configure()
        .out_dir(&shadow_dir)
        .compile_protos(
            &["../foxtive-tonic/proto/foxtive/tonic/v1/envelope.proto"],
            &["../foxtive-tonic/proto/"],
        )?;

    // prost-build only watches the files handed to `compile_protos`, not the
    // include directories, so the imported contract is declared explicitly.
    println!("cargo:rerun-if-changed=../foxtive-tonic/proto/foxtive/tonic/v1/envelope.proto");
    println!("cargo:rerun-if-changed=proto");

    Ok(())
}
