use std::io::Write;

use testcontainers::core::ContainerPort;

use crate::otlp_setup::{
    create_ferron_container, create_otlp_container, create_test_files, find_metric, poll_received,
};

/// All three signals exported over gRPC (port 4317) arrive at the mock
/// collector's gRPC services and are decoded from the protobuf payloads.
#[tokio::test(flavor = "multi_thread")]
async fn test_otlp_grpc_exported() {
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
    service_name "e2e-otlp-grpc"
    traces "http://otlp:4317" {
      protocol "grpc"
      export_interval "1s"
      export_batch_size 1
    }
    metrics "http://otlp:4317" {
      protocol "grpc"
      read_interval "1s"
    }
    logs "http://otlp:4317" {
      protocol "grpc"
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

    let network = "e2e-test-otlp-grpc";
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

    // The 404 produces an error log; the healthy request produces a span,
    // metrics, and an access log.
    let _ = client
        .get(format!("http://localhost:{}/does-not-exist", http_port))
        .send()
        .await
        .unwrap();

    let received_url = format!("http://localhost:{}/received", otlp_port);
    let payload = poll_received(&client, &received_url, |json| {
        json.get("spans")
            .and_then(|spans| spans.as_array())
            .is_some_and(|spans| spans.iter().any(|span| span["name"] == "ferron.request"))
            && json
                .get("logs")
                .and_then(|logs| logs.as_array())
                .is_some_and(|logs| logs.iter().any(|log| log["scope"] == "ferron.access"))
            && find_metric(json, "ferron.http.server.request_count").is_some()
    })
    .await
    .expect("OTLP collector did not receive gRPC exports");

    let spans = payload["spans"].as_array().unwrap();
    assert!(
        spans.iter().any(|span| span["name"] == "ferron.request"),
        "expected a gRPC-transported ferron.request span: {spans:?}"
    );

    ferron.stop().await.unwrap();
    otlp.stop().await.unwrap();
}
