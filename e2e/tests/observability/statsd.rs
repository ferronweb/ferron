use std::io::Write;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError, core::ContainerPort,
    runners::AsyncRunner,
};

use crate::otlp_setup::{create_ferron_container, create_test_files, poll_received};

/// Start the mock StatsD receiver on a shared network.
async fn create_statsd_container(
    network: &str,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let statsd_image = crate::common::build_statsd_image().await?;
    statsd_image
        .with_exposed_port(ContainerPort::Tcp(8080))
        // Short fixed wait; the test polls the /received endpoint for datagrams.
        .with_wait_for(testcontainers::core::WaitFor::seconds(2))
        .with_network(network)
        .with_hostname("statsd")
        .start()
        .await
}

/// Send a few HTTP requests to generate server metrics.
async fn generate_requests(client: &reqwest::Client, http_port: u16) {
    for _ in 0..3 {
        let _ = client
            .get(format!("http://localhost:{}/basic.txt", http_port))
            .send()
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
}

/// Poll the mock receiver until a datagram matching `needle` arrives.
async fn wait_for_datagram(
    client: &reqwest::Client,
    received_url: &str,
    needle: &str,
) -> Option<serde_json::Value> {
    poll_received(client, received_url, |json| {
        json["items"].as_array().is_some_and(|items| {
            items
                .iter()
                .any(|item| item.as_str().is_some_and(|s| s.contains(needle)))
        })
    })
    .await
}

/// Every request produces server metrics. With `datadog true`, the module
/// emits counters with the `c` type, histograms with the `h` type, and tags.
#[tokio::test(flavor = "multi_thread")]
async fn test_statsd_metrics_emitted_datadog() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut files = create_test_files();
    files
        .config
        .as_file_mut()
        .write_all(
            r#"*:80 {
  root "/var/www/ferron"
  observability {
    provider statsd
    host "statsd"
    port 8125
    prefix "e2e"
    datadog true
  }
}
"#
            .as_bytes(),
        )
        .unwrap();

    crate::common::write_file(files.webroot.path().join("basic.txt"), b"hello").unwrap();

    let network = "e2e-test-statsd-datadog";
    let statsd = create_statsd_container(network).await.unwrap();
    let ferron = create_ferron_container(network, files.webroot.path(), files.config.path())
        .await
        .unwrap();

    let http_port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let statsd_http_port = statsd
        .get_host_port_ipv4(ContainerPort::Tcp(8080))
        .await
        .unwrap();

    let client = reqwest::Client::new();
    generate_requests(&client, http_port).await;

    let received_url = format!("http://localhost:{}/received", statsd_http_port);

    // Counter with the configured prefix and `c` type.
    let payload = wait_for_datagram(
        &client,
        &received_url,
        "e2e.ferron.http.server.request_count:1|c",
    )
    .await
    .expect("StatsD mock did not receive the request counter");

    let items = payload["items"].as_array().unwrap();
    let joined = items
        .iter()
        .filter_map(|i| i.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    // Histogram with the DogStatsD `h` type.
    assert!(
        joined
            .lines()
            .any(|l| l.ends_with("|h") || l.contains("|h|#")),
        "no `h` histogram datagram received, got:\n{}",
        joined
    );

    // Tags are present in datadog mode.
    assert!(
        joined.contains("|#"),
        "no DogStatsD tags received, got:\n{}",
        joined
    );

    statsd.stop().await.unwrap();
    ferron.stop().await.unwrap();
}

/// Without `datadog`, histograms use the vanilla `ms` timer type and no tags
/// are attached.
#[tokio::test(flavor = "multi_thread")]
async fn test_statsd_vanilla_mode() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut files = create_test_files();
    files
        .config
        .as_file_mut()
        .write_all(
            r#"*:80 {
  root "/var/www/ferron"
  observability {
    provider statsd
    host "statsd"
    port 8125
  }
}
"#
            .as_bytes(),
        )
        .unwrap();

    crate::common::write_file(files.webroot.path().join("basic.txt"), b"hello").unwrap();

    let network = "e2e-test-statsd-vanilla";
    let statsd = create_statsd_container(network).await.unwrap();
    let ferron = create_ferron_container(network, files.webroot.path(), files.config.path())
        .await
        .unwrap();

    let http_port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let statsd_http_port = statsd
        .get_host_port_ipv4(ContainerPort::Tcp(8080))
        .await
        .unwrap();

    let client = reqwest::Client::new();
    generate_requests(&client, http_port).await;

    let received_url = format!("http://localhost:{}/received", statsd_http_port);

    let payload = wait_for_datagram(
        &client,
        &received_url,
        "ferron.http.server.request_count:1|c",
    )
    .await
    .expect("StatsD mock did not receive the request counter");

    let items = payload["items"].as_array().unwrap();
    let joined = items
        .iter()
        .filter_map(|i| i.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    // Histogram with the vanilla `ms` timer type.
    assert!(
        joined.lines().any(|l| l.ends_with("|ms")),
        "no `ms` timer datagram received, got:\n{}",
        joined
    );

    // No DogStatsD tags in vanilla mode.
    assert!(
        !joined.contains("|#"),
        "unexpected DogStatsD tags in vanilla mode, got:\n{}",
        joined
    );

    statsd.stop().await.unwrap();
    ferron.stop().await.unwrap();
}
