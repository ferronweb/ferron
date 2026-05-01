#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};
use std::{io::Write, path::Path};

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

mod common;

async fn create_ferron_container(
    webroot_dir: &Path,
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
            config_file.to_string_lossy(),
            "/etc/ferron.conf",
        ))
        .start()
        .await
}

#[tokio::test]
async fn test_config() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Set umask to 000 to ensure that the webroot directory is accessible to the container.
    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let basic_content = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Maecenas id dignissim leo, ac imperdiet tellus. Orci varius natoque penatibus et magnis dis parturient montes, nascetur ridiculus mus. Maecenas id erat finibus, auctor odio eu, efficitur libero. Aenean aliquet vehicula nisi ac tincidunt. Donec non vulputate dolor. Sed faucibus pulvinar augue eget viverra. Donec ornare lacus non mi mollis lacinia. Nulla suscipit vestibulum maximus. Nulla sit amet ex quis purus imperdiet vestibulum eget quis ex. Nullam accumsan nibh massa, vitae rhoncus sapien ultricies vel.\n";

    config_file
        .as_file_mut()
        .write_all(
            r#"
      snippet WORDPRESS_SCAN {
        status 403
      }

      match WORDPRESS_SCAN {
        request.uri ~ "(?i)^/wp-(?:login\.php|admin/?)(?:$|[?#])"
      }

      match SOMESCANNER_SCAN {
        request.header.user-agent ~ "^somescanner(/|$)"
      }

      aunrel:80 {
        status 403
      }

      ferron:80 {
        root "/var/www/ferron"

        location /phpmyadmin {
          status 403
        }

        if WORDPRESS_SCAN {
          use WORDPRESS_SCAN
        }

        if SOMESCANNER_SCAN {
          status 403
        }

        handle_error 404 {
          status 302 {
            regex "^/(?!basic\.txt(?:$|[?#]))"
            location "/basic.txt"
          }
        }
      }
  "#
            .as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();
    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    self::common::write_file(
        webroot_dir.path().join("basic.txt"),
        basic_content.as_bytes(),
    )
    .unwrap();

    // Test 1: Host configuration smoke test
    let response = client
        .get(format!("http://localhost:{}/basic.txt", port))
        .header("Host", "ferron")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), basic_content);

    // Test 2: Access denial with location (exact URL)
    let response = client
        .get(format!("http://localhost:{}/phpmyadmin", port))
        .header("Host", "ferron")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 3: Access denial with location (subdirectory)
    let response = client
        .get(format!("http://localhost:{}/phpmyadmin/index.php", port))
        .header("Host", "ferron")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 4: Access denial with regex conditional and snippet
    let response = client
        .get(format!("http://localhost:{}/wp-login.php", port))
        .header("Host", "ferron")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 5: Access denial with Rego-based conditional
    let response = client
        .get(format!("http://localhost:{}/", port))
        .header("Host", "ferron")
        .header("User-Agent", "somescanner/0.0.0")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 6: HTTP error interception
    let response = client
        .get(format!("http://localhost:{}/nonexistent.txt", port))
        .header("Host", "ferron")
        .send()
        .await
        .unwrap();

    assert!(
        matches!(
            response.status(),
            reqwest::StatusCode::OK | reqwest::StatusCode::FOUND
        ),
        "Expected status code OK or FOUND, got {}",
        response.status()
    );

    container.stop().await.unwrap();
}
