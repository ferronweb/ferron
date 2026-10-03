use std::io::Write;

use testcontainers::core::ContainerPort;

use crate::otlp_setup::{
    create_ferron_container, create_otlp_container, create_test_files, poll_received,
};

#[tokio::test]
async fn test_otlp_traces_exported() {
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
    service_name "e2e-otlp"
    traces "http://otlp:4318/v1/traces" {
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

    let network = "e2e-test-otlp-traces";

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

    // Trigger a request to produce a trace
    let _ = client
        .get(format!("http://localhost:{}/basic.txt", http_port))
        .send()
        .await
        .unwrap();

    // Assert on the decoded span, not just on payload counts: wait for the
    // request span itself to arrive so stage spans from earlier batches do
    // not end the poll early.
    let received_url = format!("http://localhost:{}/received", otlp_port);
    let payload = poll_received(&client, &received_url, |json| {
        json.get("spans")
            .and_then(|spans| spans.as_array())
            .is_some_and(|spans| {
                spans.iter().any(|span| {
                    span["name"] == "ferron.request"
                        && span["attributes"]
                            .get("url.full")
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|path| path.contains("basic.txt"))
                })
            })
    })
    .await
    .expect("OTLP collector did not receive traces");

    let spans = payload["spans"].as_array().unwrap();
    assert!(
        spans.iter().any(|span| span["name"] == "ferron.request"),
        "expected a decoded ferron.request span: {spans:?}"
    );
    // The decoded span carries the path as an attribute.
    assert!(
        spans.iter().any(|span| span["attributes"]
            .get("url.full")
            .is_some_and(|path| { path.as_str().is_some_and(|path| path.contains("basic.txt")) })),
        "expected a span with url.full=/basic.txt: {spans:?}"
    );

    ferron.stop().await.unwrap();
    otlp.stop().await.unwrap();
}

/// Count decoded spans that belong to `service_name`.
fn spans_for_service<'a>(
    payload: &'a serde_json::Value,
    service_name: &str,
) -> Vec<&'a serde_json::Value> {
    payload["spans"]
        .as_array()
        .map(|spans| {
            spans
                .iter()
                .filter(|span| {
                    span["resource"]
                        .get("service.name")
                        .and_then(|v| v.as_str())
                        == Some(service_name)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Test that `trace_sampling` applies per host.
///
/// Verifies that a host block with `trace_sampling always_off` exports no
/// spans, while another host on the same listener still exports them.
///
/// See: modules/http-server/src/config/trace.rs
#[tokio::test]
async fn test_per_host_trace_sampling_always_off() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut files = create_test_files();
    files
        .config
        .as_file_mut()
        .write_all(
            r#"sampled.example.com:80 {
  root "/var/www/ferron"
  observability {
    provider otlp
    service_name "e2e-otlp-sampled"
    traces "http://otlp:4318/v1/traces" {
      protocol "http/protobuf"
      export_interval "1s"
      export_batch_size 1
    }
  }
}

unsampled.example.com:80 {
  root "/var/www/ferron"
  http {
    trace_sampling always_off
  }
  observability {
    provider otlp
    service_name "e2e-otlp-unsampled"
    traces "http://otlp:4318/v1/traces" {
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

    let network = "e2e-test-otlp-traces-per-host";

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
    let received_url = format!("http://localhost:{}/received", otlp_port);

    // Baseline: the sampled host exports its request span.
    let _ = client
        .get(format!("http://localhost:{}/basic.txt", http_port))
        .header("Host", "sampled.example.com")
        .send()
        .await
        .unwrap();

    let payload = poll_received(&client, &received_url, |json| {
        !spans_for_service(json, "e2e-otlp-sampled").is_empty()
    })
    .await
    .expect("OTLP collector did not receive traces for the sampled host");

    let sampled_spans = spans_for_service(&payload, "e2e-otlp-sampled").len();
    assert!(
        sampled_spans > 0,
        "expected the sampled host to export spans: {payload:?}"
    );

    // The host with `trace_sampling always_off` must not export anything.
    for _ in 0..5 {
        let _ = client
            .get(format!("http://localhost:{}/basic.txt", http_port))
            .header("Host", "unsampled.example.com")
            .send()
            .await
            .unwrap();
    }

    // Wait past the export interval so a leaked span would have been flushed.
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;

    let after = client
        .get(&received_url)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();

    assert!(
        spans_for_service(&after, "e2e-otlp-unsampled").is_empty(),
        "expected no exported spans for the host with `trace_sampling always_off`: {:?}",
        spans_for_service(&after, "e2e-otlp-unsampled")
    );
    assert!(
        spans_for_service(&after, "e2e-otlp-sampled").len() >= sampled_spans,
        "expected the sampled host to keep exporting spans: {after:?}"
    );

    ferron.stop().await.unwrap();
    otlp.stop().await.unwrap();
}
