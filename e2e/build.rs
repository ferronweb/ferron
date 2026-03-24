use std::env;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
  // Compile the hello.proto file for gRPC tests
  let proto_path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("images/backend-grpc/hello.proto");

  prost_build::compile_protos(&[&proto_path], &[env::var("CARGO_MANIFEST_DIR").unwrap()])?;

  println!("cargo:rerun-if-changed=images/backend-grpc/hello.proto");

  Ok(())
}
