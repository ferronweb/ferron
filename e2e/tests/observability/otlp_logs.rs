use std::io::Write;

use testcontainers::core::ContainerPort;

use crate::otlp_setup::{
    create_ferron_container, create_otlp_container, create_test_files, poll_received,
};

/// Error-path requests emit log records. A malformed request produces a
/// "Request error" log; every request produces an access log. The mock
/// collector decodes both from `ExportLogsServiceRequest`.
#[tokio::test(flavor = "multi_thread")]
async fn test_otlp_logs_exported() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut files = create_test_files();
    files
        .config
        .as_file_mut()
        .write_all(
            r#"*:80 {
  root "/var/www/ferron"
  observability {
    provider otlp
    service_name "e2e-otlp-logs"
    logs "http://otlp:4318/v1/logs" {
      protocol "http/protobuf"
      export_interval "1s"
      export_batch_size 1
    }
  }
}
"#
            .as_bytes(),
        )
        .unwrap();

    crate::common::write_file(files.webroot.path().join("basic.txt"), b"hello").unwrap();

    let network = "e2e-test-otlp-logs";
    let otlp = create_otlp_container(network).await.unwrap();
    let ferron = create_ferron_container(network, files.webroot.path(), files.config.path())
        .await
        .unwrap();

    let http_port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let otlp_port = otlp
        .get_host_port_ipv4(ContainerPort::Tcp(4318))
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // A 200 response produces an access log; a malformed HTTP request
    // produces a "Request error" log through the pre-handler error path.
    let _ = client
        .get(format!("http://localhost:{}/basic.txt", http_port))
        .send()
        .await
        .unwrap();
    let mut sock = std::net::TcpStream::connect(format!("localhost:{http_port}")).unwrap();
    sock.write_all(b"GARBAGE\r\n\r\n").unwrap();
    drop(sock);

    let received_url = format!("http://localhost:{}/received", otlp_port);
    let payload = poll_received(&client, &received_url, |json| {
        json.get("logs")
            .and_then(|logs| logs.as_array())
            .is_some_and(|logs| {
                logs.iter().any(|log| {
                    log["severity_text"] == "ERROR"
                        && log["body"]
                            .as_str()
                            .is_some_and(|body| body.contains("Request error"))
                })
            })
    })
    .await
    .expect("OTLP collector did not export log records");

    let logs = payload["logs"].as_array().unwrap();

    let has_error = logs.iter().any(|log| {
        log["severity_text"] == "ERROR"
            && log["body"]
                .as_str()
                .is_some_and(|body| body.contains("Request error"))
    });
    assert!(
        has_error,
        "expected a decoded 'Request error' log record: {logs:?}"
    );

    let has_access = logs.iter().any(|log| {
        log["scope"]
            .as_str()
            .is_some_and(|scope| scope == "ferron.access")
    });
    assert!(
        has_access,
        "expected a decoded ferron.access log record: {logs:?}"
    );

    ferron.stop().await.unwrap();
    otlp.stop().await.unwrap();
}
