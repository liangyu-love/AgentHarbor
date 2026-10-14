//! Generates the Cursor Agent protobuf bindings from the vendored schema.
use std::{env, path::PathBuf};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let proto_dir = manifest.join("proto");
    let agent = proto_dir.join("agent_v1.proto");
    env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path().expect("vendored protoc"));
    prost_build::Config::new()
        .compile_protos(
            &[&agent],
            &[proto_dir.clone(), protoc_bin_vendored::include_path().expect("vendored protobuf includes")],
        )
        .expect("compile Cursor protobuf schema");
    println!("cargo:rerun-if-changed={}", agent.display());
}
