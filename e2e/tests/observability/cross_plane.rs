use std::io::Write;
use std::path::Path;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor},
    runners::AsyncRunner,
};

async fn create_ferron_container(
    network: &str,
    webroot_dir: &Path,
    config_file: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = crate::common::build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_wait_for(WaitFor::Http(Box::new(
            testcontainers::core::wait::HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_mount(Mount::bind_mount(
            webroot_dir.to_string_lossy(),
            "/var/www/ferron",
        ))
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy(),
            "/etc/ferron.conf",
        ))
        .start()
        .await
}

async fn create_otlp_container(
    network: &str,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    use std::time::Duration;

    let mut attempts = 0;
    loop {
        attempts += 1;
        let otlp_image = crate::common::build_otlp_image().await?;
        let start_res = otlp_image
            .with_exposed_port(ContainerPort::Tcp(4318))
            .with_wait_for(WaitFor::seconds(2))
            .with_network(network)
            .with_hostname("otlp")
            .start()
            .await;

        match start_res {
            Ok(container) => return Ok(container),
            Err(err) => {
                if attempts >= 3 {
                    return Err(err);
                }
                eprintln!(
                    "otlp container start attempt {} failed: {:?}, retrying...",
                    attempts, err
                );
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

/*
fn write_config(path: &mut std::path::PathBuf, content: &str) {
    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(content.as_bytes()).unwrap();
}
*/

#[tokio::test]
async fn test_control_plane_global_metadata_in_traces() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = crate::common::create_temp_dir();
    #[cfg(unix)]
    let mut config_file = crate::common::create_temp_file();

    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"{
  control_plane {
    metadata {
      org_id "12345"
      team "platform"
    }
  }
}

*:80 {
  root "/var/www/ferron"
  observability {
    provider otlp
    service_name "e2e-cross-plane"
    traces "http://otlp:4318/v1/traces" {
      protocol "http/protobuf"
    }
  }
}
"#,
        )
        .unwrap();

    crate::common::write_file(webroot_dir.path().join("index.html"), b"<h1>hello</h1>").unwrap();

    let network = "e2e-test-cross-plane";
    let otlp = create_otlp_container(network).await.unwrap();
    let ferron = create_ferron_container(network, webroot_dir.path(), config_file.path())
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
        .get(format!("http://localhost:{}/index.html", http_port))
        .send()
        .await
        .unwrap();

    // Poll the OTLP mock collector until it reports decoded spans with control plane metadata
    let received_url = format!("http://localhost:{}/received", otlp_port);
    let mut found = false;
    for _ in 0..60 {
        if let Ok(resp) = client.get(&received_url).send().await
            && resp.status().is_success()
            && let Ok(json) = resp.json::<serde_json::Value>().await
        {
            if let Some(spans) = json.get("spans").and_then(|v| v.as_array()) {
                for span in spans {
                    if let Some(attrs) = span.get("attributes").and_then(|v| v.as_object()) {
                        if attrs
                            .get("ferron.control_plane.org_id")
                            .and_then(|v| v.as_str())
                            == Some("12345")
                            && attrs
                                .get("ferron.control_plane.team")
                                .and_then(|v| v.as_str())
                                == Some("platform")
                        {
                            found = true;
                            break;
                        }
                    }
                }
            }
        }
        if found {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    assert!(
        found,
        "Control plane metadata not found in OTLP trace spans"
    );

    ferron.stop().await.unwrap();
    otlp.stop().await.unwrap();
}

#[tokio::test]
async fn test_control_plane_per_host_metadata_precedence() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = crate::common::create_temp_dir();
    #[cfg(unix)]
    let mut config_file = crate::common::create_temp_file();

    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    // Global sets org_id to "global", host example.com overrides to "host-specific"
    config_file
        .as_file_mut()
        .write_all(
            br#"{
  control_plane {
    metadata {
      org_id "global"
    }
  }
}

*:80 {
  root "/var/www/ferron"
  observability {
    provider otlp
    service_name "e2e-cross-plane"
    traces "http://otlp:4318/v1/traces" {
      protocol "http/protobuf"
    }
  }
}

example.com:80 {
  control_plane {
    metadata {
      org_id "host-specific"
    }
  }
}
"#,
        )
        .unwrap();

    crate::common::write_file(webroot_dir.path().join("index.html"), b"<h1>hello</h1>").unwrap();

    let network = "e2e-test-cross-plane-host";
    let otlp = create_otlp_container(network).await.unwrap();
    let ferron = create_ferron_container(network, webroot_dir.path(), config_file.path())
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

    // Request with Host: example.com to trigger per-host metadata
    let _ = client
        .get(format!("http://localhost:{}/index.html", http_port))
        .header("Host", "example.com")
        .send()
        .await
        .unwrap();

    let received_url = format!("http://localhost:{}/received", otlp_port);
    let mut found = false;
    for _ in 0..60 {
        if let Ok(resp) = client.get(&received_url).send().await
            && resp.status().is_success()
            && let Ok(json) = resp.json::<serde_json::Value>().await
        {
            if let Some(spans) = json.get("spans").and_then(|v| v.as_array()) {
                for span in spans {
                    if let Some(attrs) = span.get("attributes").and_then(|v| v.as_object()) {
                        if attrs
                            .get("ferron.control_plane.org_id")
                            .and_then(|v| v.as_str())
                            == Some("host-specific")
                        {
                            found = true;
                            break;
                        }
                    }
                }
            }
        }
        if found {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    assert!(
        found,
        "Per-host control plane metadata should override global"
    );

    ferron.stop().await.unwrap();
    otlp.stop().await.unwrap();
}

#[tokio::test]
async fn test_control_plane_span_links_in_traces() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = crate::common::create_temp_dir();
    #[cfg(unix)]
    let mut config_file = crate::common::create_temp_file();

    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"{
  control_plane {
    span_links {
      trace_id "0af7651916cd43dd8448eb211c80319c"
      span_id "00f067aa0ba902b7"
      sampled true
    }
  }
}

*:80 {
  root "/var/www/ferron"
  observability {
    provider otlp
    service_name "e2e-cross-plane"
    traces "http://otlp:4318/v1/traces" {
      protocol "http/protobuf"
    }
  }
}
"#,
        )
        .unwrap();

    crate::common::write_file(webroot_dir.path().join("index.html"), b"<h1>hello</h1>").unwrap();

    let network = "e2e-test-cross-plane-links";
    let otlp = create_otlp_container(network).await.unwrap();
    let ferron = create_ferron_container(network, webroot_dir.path(), config_file.path())
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
        .get(format!("http://localhost:{}/index.html", http_port))
        .send()
        .await
        .unwrap();

    // Just verify the trace was produced (span link verification requires the mock
    // collector to expose link data, which is not yet implemented in the mock)
    let received_url = format!("http://localhost:{}/received", otlp_port);
    let mut found = false;
    for _ in 0..60 {
        if let Ok(resp) = client.get(&received_url).send().await
            && resp.status().is_success()
            && let Ok(json) = resp.json::<serde_json::Value>().await
        {
            if let Some(spans) = json.get("spans").and_then(|v| v.as_array()) {
                if !spans.is_empty() {
                    found = true;
                    break;
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    assert!(
        found,
        "OTLP collector did not receive spans with span links config"
    );

    ferron.stop().await.unwrap();
    otlp.stop().await.unwrap();
}
