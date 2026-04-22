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
async fn test_http_auth_success() {
    let _ = rustls::crypto::ring::default_provider().install_default();

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

    let password_hash = password_auth::generate_hash("test");

    config_file
        .as_file_mut()
        .write_all(
            format!(
                r#"
*:80 {{
  basic_auth {{
    realm "HTTP authentication test"
    users {{
      test "{password_hash}"
    }}
  }}
  root "/var/www/ferron"
}}
"#
            )
            .as_bytes(),
        )
        .unwrap();

    self::common::write_file(webroot_dir.path().join("test.txt"), b"test content").unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let response = client
        .get(format!("http://localhost:{}/test.txt", port))
        .basic_auth("test", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "test content");

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_http_auth_failure() {
    let _ = rustls::crypto::ring::default_provider().install_default();

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

    let password_hash = "$argon2id$v=19$m=65536,t=3,p=1$c2VjcmV0c2FsdDEyMzQ1Njc4$R7dF5Q8QYJZQYJZQYJZQYJZQYJZQYJZQYJZQYJZQYJQ";

    config_file
        .as_file_mut()
        .write_all(
            format!(
                r#"
*:80 {{
  basic_auth {{
    realm "HTTP authentication test"
    users {{
      test "{password_hash}"
    }}
  }}
  root "/var/www/ferron"
}}
"#
            )
            .as_bytes(),
        )
        .unwrap();

    self::common::write_file(webroot_dir.path().join("test.txt"), b"test content").unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let response = client
        .get(format!("http://localhost:{}/test.txt", port))
        .basic_auth("test", Some("wrong_password"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

    container.stop().await.unwrap();
}
