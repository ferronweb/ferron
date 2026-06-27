//! WebSocket edge case tests for Ferron reverse proxy.
//!
//! These tests verify WebSocket proxying behavior beyond the basic echo test,
//! including large frames, binary messages, multiple sequential messages,
//! close handshakes, and ping/pong keepalive frames.
//!
//! Inspired by nginx-tests `proxy_websocket.t`.

use std::io::Write;

use futures_util::{SinkExt, StreamExt};
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

#[path = "common/mod.rs"]
mod common;

async fn create_backend_container(
    network: &str,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let backend_image = self::common::build_backend_image().await?;
    backend_image
        .with_exposed_port(ContainerPort::Tcp(3000))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(3000))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_hostname("backend")
        .start()
        .await
}

async fn create_ferron_container(
    network: &str,
    config_file: &std::path::Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/%")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_hostname("ferron")
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy(),
            "/etc/ferron.conf",
        ))
        .start()
        .await
}

struct WsTestContext {
    _backend: ContainerAsync<GenericImage>,
    _ferron: ContainerAsync<GenericImage>,
    ws_url: String,
    _config_file: tempfile::NamedTempFile,
}

impl WsTestContext {
    async fn new(test_name: &str, config: &[u8]) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();

        #[cfg(unix)]
        nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

        #[cfg(unix)]
        let mut config_file = self::common::create_temp_file();
        #[cfg(not(unix))]
        let mut config_file = tempfile::NamedTempFile::new().unwrap();

        let network = format!("e2e-test-ws-{}", test_name);

        let backend = create_backend_container(&network).await.unwrap();

        config_file.as_file_mut().write_all(config).unwrap();

        let ferron = create_ferron_container(&network, config_file.path())
            .await
            .unwrap();

        let port = ferron
            .get_host_port_ipv4(ContainerPort::Tcp(80))
            .await
            .unwrap();

        let ws_url = format!("ws://localhost:{}/echo", port);

        Self {
            _backend: backend,
            _ferron: ferron,
            ws_url,
            _config_file: config_file,
        }
    }
}

/// Test WebSocket with a large text frame (10KB).
///
/// Inspired by nginx-tests proxy_websocket.t — verifies that large WebSocket
/// frames are correctly proxied without truncation.
#[tokio::test]
async fn test_ws_large_text_frame() {
    let ctx = WsTestContext::new(
        "large-frame",
        br#"
*:80 {
  proxy "http://backend:3000"
}
"#,
    )
    .await;

    let (mut ws_stream, _) = connect_async(&ctx.ws_url)
        .await
        .expect("Failed to connect");

    // Send a 10KB text message
    let large_msg = "A".repeat(10240);
    ws_stream
        .send(Message::Text(large_msg.clone().into()))
        .await
        .expect("Failed to send large frame");

    let response = ws_stream
        .next()
        .await
        .expect("Stream ended")
        .expect("Failed to receive");

    match response {
        Message::Text(text) => {
            let received: &str = &text;
            assert_eq!(received, large_msg.as_str());
        }
        other => panic!("Expected Text message, got {:?}", other),
    }

    ws_stream.send(Message::Close(None)).await.ok();
}

/// Test WebSocket with binary messages.
///
/// Inspired by nginx-tests proxy_websocket.t — verifies that binary WebSocket
/// frames are correctly proxied without corruption.
#[tokio::test]
async fn test_ws_binary_message() {
    let ctx = WsTestContext::new(
        "binary",
        br#"
*:80 {
  proxy "http://backend:3000"
}
"#,
    )
    .await;

    let (mut ws_stream, _) = connect_async(&ctx.ws_url)
        .await
        .expect("Failed to connect");

    // Send binary data (1KB of alternating bytes)
    let binary_data: Vec<u8> = (0..1024).map(|i| (i % 256) as u8).collect();
    ws_stream
        .send(Message::Binary(binary_data.clone().into()))
        .await
        .expect("Failed to send binary message");

    let response = ws_stream
        .next()
        .await
        .expect("Stream ended")
        .expect("Failed to receive");

    match response {
        Message::Binary(data) => {
            let received: &[u8] = &data;
            assert_eq!(received, binary_data.as_slice());
        }
        other => panic!("Expected Binary message, got {:?}", other),
    }

    ws_stream.send(Message::Close(None)).await.ok();
}

/// Test WebSocket with multiple sequential messages.
///
/// Inspired by nginx-tests proxy_websocket.t — verifies that the WebSocket
/// proxy handles multiple messages in sequence without mixing them up.
#[tokio::test]
async fn test_ws_multiple_messages() {
    let ctx = WsTestContext::new(
        "multi-msg",
        br#"
*:80 {
  proxy "http://backend:3000"
}
"#,
    )
    .await;

    let (mut ws_stream, _) = connect_async(&ctx.ws_url)
        .await
        .expect("Failed to connect");

    // Send 10 messages and verify each is echoed correctly
    for i in 0..10 {
        let msg = format!("message-{}", i);
        ws_stream
            .send(Message::Text(msg.clone().into()))
            .await
            .expect("Failed to send");

        let response = ws_stream
            .next()
            .await
            .expect("Stream ended")
            .expect("Failed to receive");

        match response {
            Message::Text(text) => {
                let received: &str = &text;
                assert_eq!(received, msg.as_str());
            }
            other => panic!("Expected Text message for iteration {}, got {:?}", i, other),
        }
    }

    ws_stream.send(Message::Close(None)).await.ok();
}

/// Test WebSocket close handshake.
///
/// Inspired by nginx-tests proxy_websocket.t — verifies that the close
/// handshake is correctly proxied between client and backend.
#[tokio::test]
async fn test_ws_close_handshake() {
    let ctx = WsTestContext::new(
        "close",
        br#"
*:80 {
  proxy "http://backend:3000"
}
"#,
    )
    .await;

    let (mut ws_stream, _) = connect_async(&ctx.ws_url)
        .await
        .expect("Failed to connect");

    // Send a message first to verify connection is working
    ws_stream
        .send(Message::Text("before-close".into()))
        .await
        .expect("Failed to send");

    let response = ws_stream
        .next()
        .await
        .expect("Stream ended")
        .expect("Failed to receive");

    match response {
        Message::Text(text) => {
            let received: &str = &text;
            assert_eq!(received, "before-close");
        }
        other => panic!("Expected Text message, got {:?}", other),
    }

    // Initiate close with a close code
    ws_stream
        .send(Message::Close(Some(
            tokio_tungstenite::tungstenite::protocol::CloseFrame {
                code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Normal,
                reason: "test close".into(),
            },
        )))
        .await
        .expect("Failed to send close");

    // The backend should echo the close frame back
    // After close, the stream should end
    let mut got_close = false;
    for _ in 0..5 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), ws_stream.next()).await {
            Ok(Some(Ok(Message::Close(_)))) => {
                got_close = true;
                break;
            }
            Ok(Some(Ok(_))) => continue,
            _ => break,
        }
    }
    assert!(got_close, "Did not receive close frame from backend");
}

/// Test WebSocket with ping/pong keepalive frames.
///
/// Inspired by nginx-tests proxy_websocket.t — verifies that ping/pong
/// frames are correctly proxied for WebSocket keepalive.
#[tokio::test]
async fn test_ws_ping_pong() {
    let ctx = WsTestContext::new(
        "ping-pong",
        br#"
*:80 {
  proxy "http://backend:3000"
}
"#,
    )
    .await;

    let (mut ws_stream, _) = connect_async(&ctx.ws_url)
        .await
        .expect("Failed to connect");

    // Send a ping with custom payload
    let ping_data = b"ping-test";
    ws_stream
        .send(Message::Ping(ping_data.to_vec().into()))
        .await
        .expect("Failed to send ping");

    // Wait for pong response
    let mut got_pong = false;
    for _ in 0..5 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), ws_stream.next()).await {
            Ok(Some(Ok(Message::Pong(data)))) => {
                assert_eq!(&*data, ping_data);
                got_pong = true;
                break;
            }
            Ok(Some(Ok(_))) => continue,
            _ => break,
        }
    }
    assert!(got_pong, "Did not receive pong response from backend");

    ws_stream.send(Message::Close(None)).await.ok();
}
