#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};
use std::{io::Write, path::Path};

use testcontainers::{
  ContainerAsync, GenericImage, ImageExt, TestcontainersError,
  core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
  runners::AsyncRunner,
};

mod common;

// Include the generated protobuf code (message types only)
pub mod hello {
  include!(concat!(env!("OUT_DIR"), "/helloworld.rs"));
}

async fn create_backend_grpc_container(
  network: &str,
  cert_dir: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
  let backend_grpc_image: GenericImage = self::common::build_backend_grpc_image().await?;
  backend_grpc_image
    .with_exposed_port(ContainerPort::Tcp(50051))
    .with_wait_for(WaitFor::seconds(3)) // Fixed wait duration, since the gRPC server is HTTP/2-only
    .with_network(network)
    .with_hostname("backend")
    .with_mount(Mount::bind_mount(cert_dir.to_string_lossy().to_string(), "/etc/certs"))
    .start()
    .await
}

async fn create_ferron_container(
  network: &str,
  config_file: &Path,
  cert_dir: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
  let ferron_image: GenericImage = self::common::build_ferron_image().await?;
  ferron_image
    .with_exposed_port(ContainerPort::Tcp(443))
    .with_wait_for(WaitFor::Http(Box::new(
      HttpWaitStrategy::new("/%")
        .with_port(ContainerPort::Tcp(443))
        .with_tls()
        .with_client(
          reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true)
            .build()
            .map_err(|_| TestcontainersError::other("failed to configure HTTP client"))?,
        )
        .with_response_matcher(|_| true),
    )))
    .with_network(network)
    .with_hostname("ferron")
    .with_mount(Mount::bind_mount(
      config_file.to_string_lossy().to_string(),
      "/etc/ferron.kdl",
    ))
    .with_mount(Mount::bind_mount(cert_dir.to_string_lossy().to_string(), "/etc/certs"))
    .start()
    .await
}

struct GRpcRProxyTestContext {
  _backend: ContainerAsync<GenericImage>,
  _ferron: ContainerAsync<GenericImage>,
  ferron_port: u16,
  _cert_dir: tempfile::TempDir,
  _config_file: tempfile::NamedTempFile,
}

impl GRpcRProxyTestContext {
  async fn new(test_name: &str, config_content: &[u8]) -> Self {
    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let cert_dir = tempfile::Builder::new()
      .permissions(Permissions::from_mode(0o777))
      .tempdir()
      .unwrap();
    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
      .permissions(Permissions::from_mode(0o666))
      .tempfile()
      .unwrap();

    #[cfg(not(unix))]
    let cert_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    // Generate certs for Ferron
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    std::fs::write(cert_dir.path().join("server.crt"), cert.cert.pem()).unwrap();
    std::fs::write(cert_dir.path().join("server.key"), cert.signing_key.serialize_pem()).unwrap();

    let network = format!("e2e-test-grpc-rproxy-{}", test_name);

    // Start backend
    let backend = create_backend_grpc_container(&network, cert_dir.path()).await.unwrap();

    // Write Ferron config
    config_file.as_file_mut().write_all(config_content).unwrap();

    // Start Ferron
    let ferron = create_ferron_container(&network, config_file.path(), cert_dir.path())
      .await
      .unwrap();

    let ferron_port = ferron.get_host_port_ipv4(ContainerPort::Tcp(443)).await.unwrap();

    Self {
      _backend: backend,
      _ferron: ferron,
      ferron_port,
      _cert_dir: cert_dir,
      _config_file: config_file,
    }
  }
}

async fn call_say_hello(host: String, port: u16, name: String) -> Result<String, Box<dyn std::error::Error>> {
  use prost::Message;

  // Create the request message
  let request_msg = hello::HelloRequest { name };
  let mut request_bytes = Vec::new();
  request_msg.encode(&mut request_bytes)?;

  // gRPC message format: compression flag (1 byte) + length (4 bytes) + data
  let mut grpc_message = Vec::with_capacity(5 + request_bytes.len());
  grpc_message.push(0); // no compression
  grpc_message.extend_from_slice(&(request_bytes.len() as u32).to_be_bytes());
  grpc_message.extend_from_slice(&request_bytes);

  // Create HTTP/2 request
  let response = reqwest::Client::builder()
    .danger_accept_invalid_certs(true)
    .danger_accept_invalid_hostnames(true)
    .build()?
    .post(format!("https://{}:{}/helloworld.Greeter/SayHello", host, port))
    .header("Content-Type", "application/grpc")
    .body(grpc_message)
    .send()
    .await?;

  let status = response.status();
  if !status.is_success() {
    return Err(format!("gRPC request failed with status: {}", status).into());
  }

  let body_bytes = response.bytes().await?;

  // Skip compression flag (1 byte) and length (4 bytes)
  if body_bytes.len() < 5 {
    return Err("Response too short".into());
  }
  let message_bytes = &body_bytes[5..];
  let response_msg = hello::HelloReply::decode(message_bytes)?;

  Ok(response_msg.message)
}

#[tokio::test]
async fn test_grpc_reverse_proxy_basic() {
  let _ = rustls::crypto::ring::default_provider().install_default();

  let config = br#"
globals {
  protocols "h1" "h2"
}

:443 {
  tls "/etc/certs/server.crt" "/etc/certs/server.key"
  proxy "http://backend:50051/"
  proxy_http2_only
}
"#;

  let ctx = GRpcRProxyTestContext::new("basic", config).await;

  let response = call_say_hello("localhost".to_string(), ctx.ferron_port, "Ferron".to_string())
    .await
    .expect("Failed to call SayHello");

  assert_eq!(response, "Hello Ferron");
}

// TODO: gRPC over TLS backend
/*
#[tokio::test]
async fn test_grpc_reverse_proxy_tls_backend() {
  let _ = rustls::crypto::ring::default_provider().install_default();

  let config = br#"
globals {
  protocols "h1" "h2"
}

:443 {
  tls "/etc/certs/server.crt" "/etc/certs/server.key"
  proxy "https://backend:50051/"
  proxy_http2_only
  proxy_no_verification
}
"#;

  let ctx = GRpcRProxyTestContext::new("tls-backend", config).await;

  let response = call_say_hello("localhost".to_string(), ctx.ferron_port, "Ferron TLS".to_string())
    .await
    .expect("Failed to call SayHello");

  assert_eq!(response, "Hello Ferron TLS");
}
*/
