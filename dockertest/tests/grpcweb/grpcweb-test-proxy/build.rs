use std::path::PathBuf;

use tonic_prost_build::configure;

fn main() {
  let proto_directory = if std::fs::exists("../hello.proto").unwrap_or(false) {
    let mut pathbuf = PathBuf::new();
    pathbuf.push("..");
    pathbuf
  } else {
    PathBuf::new()
  };
  let proto_file = {
    let mut pathbuf = proto_directory.clone();
    pathbuf.push("hello.proto");
    pathbuf
  };
  configure().compile_protos(&[proto_file], &[proto_directory]).unwrap();
}
