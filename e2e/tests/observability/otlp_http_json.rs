use std::io::Write;

use testcontainers::core::ContainerPort;

use crate::otlp_setup::{
    create_ferron_container, create_otlp_container, create_test_files, poll_received,
};

/// With `protocol "http/json"` spans are transported as OTLP JSON
/// (pbjson + hex ID fields) and the mock collector decodes them from the
/// JSON body.
#[tokio::test(flavor = "multi_thread")]
async fn test_otlp_http_json_exported() {
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
    service_name "e2e-otlp-json"
    traces "http://otlp:4318/v1/traces" {
      protocol "http/json"
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

    let network = "e2e-test-otlp-json";
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

    let _ = client
        .get(format!("http://localhost:{}/basic.txt", http_port))
        .send()
        .await
        .unwrap();

    let received_url = format!("http://localhost:{}/received", otlp_port);
    let payload = poll_received(&client, &received_url, |json| {
        json.get("spans")
            .and_then(|spans| spans.as_array())
            .is_some_and(|spans| spans.iter().any(|span| span["name"] == "ferron.request"))
    })
    .await
    .expect("OTLP collector did not receive the JSON export");

    let spans = payload["spans"].as_array().unwrap();
    let span = spans
        .iter()
        .find(|span| span["name"] == "ferron.request")
        .expect("expected a JSON-transported ferron.request span");
    // The OTLP JSON encoding emits IDs as hex strings, not base64.
    let trace_id = span["trace_id"].as_str().unwrap();
    assert!(
        !trace_id.is_empty() && trace_id.chars().all(|c| c.is_ascii_hexdigit()),
        "trace_id must be a hex string: {trace_id}"
    );
    assert!(span["json"].as_bool().unwrap_or(false));

    ferron.stop().await.unwrap();
    otlp.stop().await.unwrap();
}
