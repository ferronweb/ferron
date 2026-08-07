use std::path::PathBuf;

fn main() {
    let crate_root_dir: PathBuf = std::env::var("CARGO_MANIFEST_DIR").unwrap().into();
    let opentelemetry_proto_dir = crate_root_dir.join("opentelemetry-proto");

    if !opentelemetry_proto_dir.join("opentelemetry/proto").exists() {
        let repo = git2::Repository::open(&crate_root_dir)
            .expect("failed to open the ferron3 Git repository");
        let mut submodule = repo
            .find_submodule("opentelemetry-proto")
            .expect("failed to find the opentelemetry-proto submodule");
        submodule
            .update(true, None)
            .expect("failed to update the opentelemetry-proto submodule");
    }

    if !opentelemetry_proto_dir.join("opentelemetry/proto").exists() {
        panic!(
            "failed to obtain the opentelemetry-proto Git repository, perhaps you need \
            to run `git submodule update --init --recursive`?"
        );
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
    tonic_prost_build::configure()
        .build_server(false)
        .build_client(true)
        .compile_fds(file_descriptors)
        .expect("failed to compile opentelemetry protobufs");
}
