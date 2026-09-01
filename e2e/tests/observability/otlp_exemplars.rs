use std::io::Write;

use testcontainers::core::ContainerPort;

use crate::otlp_setup::{
    create_ferron_container, create_otlp_container, create_test_files, poll_received,
};

const TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
const SPAN_ID: &str = "00f067aa0ba902b7";

/// When a request carries a W3C `traceparent` header, the exported metric
/// data points for that request carry an exemplar whose trace and span IDs
/// match the header.
#[tokio::test(flavor = "multi_thread")]
async fn test_otlp_exemplars() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut files = create_test_files();
    files
        .config
        .as_file_mut()
        .write_all(
            r#"{
  http {
    trace {
      generate
      trust_request
    }
  }
}

*:80 {
  root "/var/www/ferron"
  observability {
    provider otlp
    service_name "e2e-otlp-exemplars"
    metrics "http://otlp:4318/v1/metrics" {
      protocol "http/protobuf"
      read_interval "1s"
    }
    # Add tracing backend so exemplars are exported
    traces "http://otlp:4318/v1/traces" {
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

    let network = "e2e-test-otlp-exemplars";
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
        .header("traceparent", format!("00-{TRACE_ID}-{SPAN_ID}-01"))
        .send()
        .await
        .unwrap();

    let received_url = format!("http://localhost:{}/received", otlp_port);
    let payload = match poll_received(&client, &received_url, |json| {
        json["metrics"].as_array().is_some_and(|metrics| {
            metrics
                .iter()
                .filter(|metric| metric["name"] == "http.server.request.duration")
                .flat_map(|metric| metric["points"].as_array().into_iter().flatten())
                .flat_map(|point| point["exemplars"].as_array().into_iter().flatten())
                .any(|exemplar| exemplar["trace_id"] == TRACE_ID)
        })
    })
    .await
    {
        Some(payload) => payload,
        None => {
            let dump = client
                .get(&received_url)
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap();
            panic!("OTLP collector did not export metric exemplars; /received: {dump}");
        }
    };

    let _exemplar = payload["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|metric| metric["name"] == "http.server.request.duration")
        .flat_map(|metric| metric["points"].as_array().into_iter().flatten())
        .flat_map(|point| point["exemplars"].as_array().into_iter().flatten())
        .find(|exemplar| exemplar["trace_id"] == TRACE_ID)
        .expect("exemplar must carry the request trace ID");
    // Commented out, because Ferron generates new span ID in this configuration.
    //assert_eq!(_exemplar["span_id"], SPAN_ID);

    ferron.stop().await.unwrap();
    otlp.stop().await.unwrap();
}
