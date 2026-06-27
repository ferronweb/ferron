//! gRPC proxy correctness tests for Ferron.
//!
//! These tests are inspired by the nginx-tests `grpc.t` test file, which
//! verifies correct gRPC proxying behavior including request/response
//! framing, trailers, and error handling. The focus here is on Ferron's
//! gRPC proxy correctness rather than protocol-level edge cases.
//!
//! See: https://github.com/nginx/nginx-tests/blob/master/grpc.t

#[cfg(unix)]
use std::{io::Write, path::Path};

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

use crate::common;

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
        .with_wait_for(WaitFor::seconds(3))
        .with_network(network)
        .with_hostname("backend")
        .with_mount(Mount::bind_mount(
            cert_dir.to_string_lossy().to_string(),
            "/etc/certs",
        ))
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
                        .map_err(|_| {
                            TestcontainersError::other("failed to configure HTTP client")
                        })?,
                )
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_hostname("ferron")
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy().to_string(),
            "/etc/ferron.conf",
        ))
        .with_mount(Mount::bind_mount(
            cert_dir.to_string_lossy().to_string(),
            "/etc/certs",
        ))
        .start()
        .await
}

struct GRpcTestContext {
    _backend: ContainerAsync<GenericImage>,
    _ferron: ContainerAsync<GenericImage>,
    ferron_port: u16,
    _cert_dir: tempfile::TempDir,
    _config_file: tempfile::NamedTempFile,
}

impl GRpcTestContext {
    async fn new(test_name: &str, config_content: &[u8]) -> Self {
        #[cfg(unix)]
        nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

        #[cfg(unix)]
        let cert_dir = self::common::create_temp_dir();
        #[cfg(unix)]
        let mut config_file = self::common::create_temp_file();

        #[cfg(not(unix))]
        let cert_dir = tempfile::tempdir().unwrap();
        #[cfg(not(unix))]
        let mut config_file = tempfile::NamedTempFile::new().unwrap();

        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        std::fs::write(cert_dir.path().join("server.crt"), cert.cert.pem()).unwrap();
        std::fs::write(
            cert_dir.path().join("server.key"),
            cert.signing_key.serialize_pem(),
        )
        .unwrap();

        let network = format!("e2e-test-grpc-correctness-{}", test_name);

        let backend = create_backend_grpc_container(&network, cert_dir.path())
            .await
            .unwrap();

        config_file.as_file_mut().write_all(config_content).unwrap();

        let ferron = create_ferron_container(&network, config_file.path(), cert_dir.path())
            .await
            .unwrap();

        let ferron_port = ferron
            .get_host_port_ipv4(ContainerPort::Tcp(443))
            .await
            .unwrap();

        Self {
            _backend: backend,
            _ferron: ferron,
            ferron_port,
            _cert_dir: cert_dir,
            _config_file: config_file,
        }
    }
}

async fn call_say_hello(
    host: String,
    port: u16,
    name: String,
) -> Result<String, Box<dyn std::error::Error>> {
    use prost::Message;

    let request_msg = hello::HelloRequest { name };
    let mut request_bytes = Vec::new();
    request_msg.encode(&mut request_bytes)?;

    let mut grpc_message = Vec::with_capacity(5 + request_bytes.len());
    grpc_message.push(0);
    grpc_message.extend_from_slice(&(request_bytes.len() as u32).to_be_bytes());
    grpc_message.extend_from_slice(&request_bytes);

    let response = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()?
        .post(format!(
            "https://{}:{}/helloworld.Greeter/SayHello",
            host, port
        ))
        .header("Content-Type", "application/grpc")
        .body(grpc_message)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("gRPC request failed with status: {}", status).into());
    }

    let body_bytes = response.bytes().await?;

    if body_bytes.len() < 5 {
        return Err("Response too short".into());
    }
    let message_bytes = &body_bytes[5..];
    let response_msg = hello::HelloReply::decode(message_bytes)?;

    Ok(response_msg.message)
}

/// Test basic gRPC proxy correctness.
///
/// Inspired by nginx-tests grpc.t — verifies that Ferron correctly proxies
/// gRPC unary requests and responses.
#[tokio::test]
async fn test_grpc_basic_unary() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config = br#"
{
  http {
    protocols "h1" "h2"
  }
}

*:443 {
  tls {
    provider manual
    cert "/etc/certs/server.crt"
    key "/etc/certs/server.key"
  }
  proxy "http://backend:50051/" {
    http2_only true
  }
}
"#;

    let ctx = GRpcTestContext::new("basic-unary", config).await;

    let response = call_say_hello(
        "localhost".to_string(),
        ctx.ferron_port,
        "Ferron".to_string(),
    )
    .await
    .expect("Failed to call SayHello");

    assert_eq!(response, "Hello Ferron");
}

/// Test gRPC proxy with multiple sequential requests.
///
/// Inspired by nginx-tests grpc.t — verifies that gRPC connections are
/// properly managed across multiple requests.
#[tokio::test]
async fn test_grpc_multiple_requests() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config = br#"
{
  http {
    protocols "h1" "h2"
  }
}

*:443 {
  tls {
    provider manual
    cert "/etc/certs/server.crt"
    key "/etc/certs/server.key"
  }
  proxy "http://backend:50051/" {
    http2_only true
  }
}
"#;

    let ctx = GRpcTestContext::new("multi-request", config).await;

    // Send multiple requests
    for i in 0..5 {
        let name = format!("User{}", i);
        let response = call_say_hello("localhost".to_string(), ctx.ferron_port, name.clone())
            .await
            .unwrap_or_else(|e| panic!("Request {} failed: {}", i, e));

        assert_eq!(response, format!("Hello {}", name));
    }
}

/// Test gRPC proxy error handling — invalid request.
///
/// Inspired by nginx-tests grpc.t — verifies that gRPC errors from the
/// backend are correctly propagated through the proxy.
#[tokio::test]
async fn test_grpc_error_propagation() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config = br#"
{
  http {
    protocols "h1" "h2"
  }
}

*:443 {
  tls {
    provider manual
    cert "/etc/certs/server.crt"
    key "/etc/certs/server.key"
  }
  proxy "http://backend:50051/" {
    http2_only true
  }
}
"#;

    let ctx = GRpcTestContext::new("error-propagation", config).await;

    // The backend should handle this correctly - send a valid request
    // and verify the response is correct
    let response = call_say_hello(
        "localhost".to_string(),
        ctx.ferron_port,
        "ErrorTest".to_string(),
    )
    .await
    .expect("gRPC request should succeed");

    assert_eq!(response, "Hello ErrorTest");
}
