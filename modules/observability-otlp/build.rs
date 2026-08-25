use std::path::PathBuf;

use protox::prost::Message;

fn main() {
    let crate_root_dir: PathBuf = std::env::var("CARGO_MANIFEST_DIR").unwrap().into();
    let opentelemetry_proto_dir = crate_root_dir.join("opentelemetry-proto");

    if !opentelemetry_proto_dir.join("opentelemetry/proto").exists() {
        // Execute a `git` command instead, because git2 depends indirectly on zlib (`libz-sys`),
        // which fails to build for some CPU architectures
        if let Some(git_path) = find_git::git_path() {
            std::process::Command::new(git_path)
                .arg("submodule")
                .arg("update")
                .arg("--init")
                .arg("--recursive")
                .arg("--")
                .arg("opentelemetry-proto")
                .current_dir(&crate_root_dir)
                .status()
                .expect("failed to update the opentelemetry-proto submodule");
        }
    }

    if !opentelemetry_proto_dir.join("opentelemetry/proto").exists() {
        panic!(
            "failed to obtain the opentelemetry-proto Git repository, perhaps you need \
            to run `git submodule update --init --recursive`?"
        );
    }

    // Re-run the build script when any protobuf definition changes.
    for entry in walkdir_proto_files(&opentelemetry_proto_dir) {
        println!("cargo:rerun-if-changed={}", entry.display());
    }

    // Use `protox`, which doesn't require installing `protoc` on the host machine,
    // along with `tonic-prost-build` to genereate Rust code from protobufs
    //
    // Use protobuf files from opentelemetry/proto/collector, similarly to what
    // `opentelemetry-otlp` crate does.
    let file_descriptors = protox::compile(
        [
            opentelemetry_proto_dir
                .join("opentelemetry/proto/collector/logs/v1/logs_service.proto"),
            opentelemetry_proto_dir
                .join("opentelemetry/proto/collector/metrics/v1/metrics_service.proto"),
            opentelemetry_proto_dir
                .join("opentelemetry/proto/collector/trace/v1/trace_service.proto"),
        ],
        [opentelemetry_proto_dir.clone()],
    )
    .expect("failed to compile opentelemetry protobufs");

    // This wouldn't be an OTLP observability backends, just an OTLP exporter...
    // Also, separate builds: client-only for production, client-and-server for tests
    tonic_prost_build::configure()
        .build_server(false)
        .build_client(true)
        .compile_fds(file_descriptors.clone())
        .expect("failed to compile opentelemetry protobufs");

    let test_out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("with_server");
    let _ = std::fs::create_dir_all(&test_out_dir);
    tonic_prost_build::configure()
        .out_dir(test_out_dir)
        .build_server(true)
        .build_client(true)
        .compile_fds(file_descriptors.clone())
        .expect("failed to compile opentelemetry protobufs");

    // Generate `serde::Serialize`/`serde::Deserialize` implementations (JSON
    // Protobuf encoding) for all `opentelemetry` packages. Enums are serialized
    // as integers, as required by the OTLP/HTTP JSON encoding.
    pbjson_build::Builder::new()
        .register_descriptors(&file_descriptors.encode_to_vec())
        .expect("failed to register opentelemetry protobufs for pbjson")
        .use_integers_for_enums()
        .ignore_unknown_fields()
        .build(&[".opentelemetry"])
        .expect("failed to generate opentelemetry protobuf JSON (de)serialization code");
}

/// List all `.proto` files under the `opentelemetry-proto` directory.
fn walkdir_proto_files(dir: &PathBuf) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(walkdir_proto_files(&path));
            } else if path.extension().is_some_and(|ext| ext == "proto") {
                files.push(path);
            }
        }
    }
    files
}
