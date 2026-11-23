#[cfg(unix)]
use std::{io::Write, path::Path};

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

use crate::common;

async fn create_persist_container(
    webroot_dir: &Path,
    persist_dir: &Path,
    config_file: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
        .with_network("bridge")
        .with_mount(Mount::bind_mount(
            webroot_dir.to_string_lossy(),
            "/var/www/ferron",
        ))
        .with_mount(Mount::bind_mount(
            persist_dir.to_string_lossy(),
            "/var/cache/ferron",
        ))
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy(),
            "/etc/ferron.conf",
        ))
        .start()
        .await
}

/// Entries journaled to disk must survive a full container restart: after the
/// restart the same request is served from cache without touching the origin.
#[tokio::test]
async fn test_cache_persistence_survives_restart() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let persist_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            r#"
      {
        cache {
          max_entries 1024
          persist "/var/cache/ferron"
          persist_interval "1s"
        }
      }

      persist-test.example.com:80 {
        root "/var/www/ferron"
        file_cache_control "public, max-age=600"
        cache true
      }
  "#
            .as_bytes(),
        )
        .unwrap();

    let container =
        create_persist_container(webroot_dir.path(), persist_dir.path(), config_file.path())
            .await
            .unwrap();

    self::common::write_file(webroot_dir.path().join("test.txt"), "persist-v1".as_bytes()).unwrap();

    let client = reqwest::Client::new();
    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let url = format!("http://localhost:{}/test.txt", port);

    let response = client
        .get(&url)
        .header("Host", "persist-test.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        String::from_utf8_lossy(response.headers().get("Cache-Status").unwrap().as_bytes())
            .contains("miss")
    );
    assert_eq!(&*response.bytes().await.unwrap(), b"persist-v1");

    let response = client
        .get(&url)
        .header("Host", "persist-test.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        String::from_utf8_lossy(response.headers().get("Cache-Status").unwrap().as_bytes())
            .contains("hit")
    );
    drop(response);

    let journal = persist_dir.path().join("global").join("journal");
    let journal_written = async {
        for _ in 0..25 {
            if std::fs::metadata(&journal).is_ok_and(|meta| meta.len() > 0) {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        false
    }
    .await;
    assert!(
        journal_written,
        "expected a non-empty journal at {}",
        journal.display()
    );

    container.stop().await.unwrap();
    let container =
        create_persist_container(webroot_dir.path(), persist_dir.path(), config_file.path())
            .await
            .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let url = format!("http://localhost:{}/test.txt", port);

    // The entry must be restored from disk: a cache hit without touching
    // the origin file (the file is gone, so a miss would fail).
    std::fs::remove_file(webroot_dir.path().join("test.txt")).unwrap();

    let response = client
        .get(&url)
        .header("Host", "persist-test.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        String::from_utf8_lossy(response.headers().get("Cache-Status").unwrap().as_bytes())
            .contains("hit"),
        "restored entry must be served as a cache hit"
    );
    assert_eq!(&*response.bytes().await.unwrap(), b"persist-v1");

    container.stop().await.unwrap();
}
