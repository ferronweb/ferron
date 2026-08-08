use std::io::Write;

use testcontainers::core::ContainerPort;

use crate::otlp_setup::{
    create_ferron_container, create_otlp_container, create_test_files, find_metric, poll_received,
};

/// With `native_histograms false` the request duration histogram is exported
/// as a plain `Histogram` with the SDK default explicit bucket boundaries
/// instead of an exponential histogram.
#[tokio::test(flavor = "multi_thread")]
async fn test_otlp_native_histograms_explicit() {
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
    service_name "e2e-otlp-explicit"
    metrics "http://otlp:4318/v1/metrics" {
      protocol "http/protobuf"
      read_interval "1s"
      native_histograms false
    }
  }
}
"#
            .as_bytes(),
        )
        .unwrap();

    crate::common::write_file(files.webroot.path().join("basic.txt"), b"hello").unwrap();

    let network = "e2e-test-otlp-explicit";
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
        find_metric(json, "http.server.request.duration")
            .is_some_and(|metric| metric["kind"] == "histogram")
    })
    .await
    .expect("OTLP collector did not export an explicit histogram");

    let duration = find_metric(&payload, "http.server.request.duration").unwrap();
    assert_eq!(duration["kind"], "histogram");
    let point = &duration["points"][0];
    let bounds = point["explicit_bounds"].as_array().unwrap();
    // There may be different buckets for different histogram metrics...
    assert!(
        !bounds
            .iter()
            .map(|bound| bound.as_f64().unwrap())
            .collect::<Vec<f64>>()
            .is_empty()
    );
    let counts = point["bucket_counts"].as_array().unwrap();
    assert_eq!(counts.len(), bounds.len() + 1);
    assert!(point["count"].as_u64().unwrap_or(0) >= 1);
    assert_eq!(
        counts
            .iter()
            .map(|count| count.as_u64().unwrap())
            .sum::<u64>(),
        point["count"].as_u64().unwrap()
    );

    ferron.stop().await.unwrap();
    otlp.stop().await.unwrap();
}
