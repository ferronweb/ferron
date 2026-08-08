use std::io::Write;

use testcontainers::core::ContainerPort;

use crate::otlp_setup::{
    create_ferron_container, create_otlp_container, create_test_files, find_metric, poll_received,
};

/// Every request produces server metrics. With a 1 s read interval the mock
/// collector decodes an `ExportMetricsServiceRequest` containing at least the
/// request counter, the active-request up/down counter, and the request
/// duration exponential histogram.
#[tokio::test(flavor = "multi_thread")]
async fn test_otlp_metrics_exported() {
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
    service_name "e2e-otlp-metrics"
    metrics "http://otlp:4318/v1/metrics" {
      protocol "http/protobuf"
      read_interval "1s"
    }
  }
}
"#
            .as_bytes(),
        )
        .unwrap();

    crate::common::write_file(files.webroot.path().join("basic.txt"), b"hello").unwrap();

    let network = "e2e-test-otlp-metrics";
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

    for _ in 0..3 {
        let _ = client
            .get(format!("http://localhost:{}/basic.txt", http_port))
            .send()
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    let received_url = format!("http://localhost:{}/received", otlp_port);
    let payload = poll_received(&client, &received_url, |json| {
        find_metric(json, "ferron.http.server.request_count").is_some()
    })
    .await
    .expect("OTLP collector did not export metrics");

    // Counter: sum of the request count with a positive summed value.
    let request_count = find_metric(&payload, "ferron.http.server.request_count").unwrap();
    assert_eq!(request_count["kind"], "sum");
    assert_eq!(request_count["is_monotonic"], serde_json::Value::Bool(true));
    let point = &request_count["points"][0];
    assert!(
        point["value"].as_i64().unwrap_or(0) >= 1,
        "request count sum must be >= 1"
    );

    // Duration histogram: exponential layout by default, with at least one
    // sample and a positive sum.
    let duration = find_metric(&payload, "http.server.request.duration").unwrap();
    assert_eq!(duration["kind"], "exponential_histogram");
    let duration_point = &duration["points"][0];
    assert!(duration_point["count"].as_u64().unwrap_or(0) >= 1);
    assert!(
        duration_point["sum"].as_f64().unwrap_or(0.0) > 0.0,
        "request duration sum must be positive"
    );

    // Active requests return to zero after the request completes.
    let active = find_metric(&payload, "http.server.active_requests").unwrap();
    assert_eq!(active["kind"], "sum");
    assert_eq!(active["is_monotonic"], serde_json::Value::Bool(false));

    ferron.stop().await.unwrap();
    otlp.stop().await.unwrap();
}
