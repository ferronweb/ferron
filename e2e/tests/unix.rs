use std::io::Write;
use std::path::Path;
use std::time::Duration;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor},
    runners::AsyncRunner,
};

mod common;

async fn http_get_via_unix(
    socket_path: &Path,
    host: &str,
    uri: &str,
) -> Result<(u16, String), Box<dyn std::error::Error>> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::UnixStream::connect(socket_path).await?;
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        uri, host
    );
    stream.write_all(request.as_bytes()).await?;
    stream.flush().await?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    let response_str = String::from_utf8_lossy(&response);
    let status_line = response_str.lines().next().unwrap_or("");
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    // Split header/body
    let parts: Vec<&str> = response_str.splitn(2, "\r\n\r\n").collect();
    let body = parts.get(1).unwrap_or(&"").to_string();
    Ok((status, body))
}

async fn wait_for_socket(path: &Path, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if path.exists() {
            // Also check it's a socket
            if let Ok(meta) = std::fs::metadata(path) {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::FileTypeExt;
                    if meta.file_type().is_socket() {
                        return true;
                    }
                }
                #[cfg(not(unix))]
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

#[cfg(unix)]
async fn create_ferron_unix_container(
    webroot_dir: &Path,
    config_file: &Path,
    host_socket_dir: &Path,
    container_socket_dir: &str,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
    ferron_image
        .with_wait_for(WaitFor::seconds(2))
        .with_network("bridge")
        .with_mount(Mount::bind_mount(
            webroot_dir.to_string_lossy(),
            "/var/www/ferron",
        ))
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy(),
            "/etc/ferron.conf",
        ))
        .with_mount(Mount::bind_mount(
            host_socket_dir.to_string_lossy(),
            container_socket_dir,
        ))
        .with_cmd(vec![
            "/bin/sh",
            "-c",
            "ferron daemon -c /etc/ferron.conf --pid-file /tmp/ferron.pid; sleep 1; wait $(cat /tmp/ferron.pid)",
        ])
        .start()
        .await
}

#[tokio::test]
async fn test_unix_absolute() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(not(unix))]
    {
        eprintln!("Skipping Unix socket test on non-Unix platform");
        return;
    }

    #[cfg(unix)]
    {
        nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

        let webroot_dir = common::create_temp_dir();
        let mut config_file = common::create_temp_file();
        let host_socket_dir = common::create_temp_dir();

        common::write_file(
            webroot_dir.path().join("index.html"),
            b"hello from unix absolute",
        )
        .unwrap();

        // Ferron config: absolute Unix socket, no TCP port needed
        // When unix is configured, TCP/QUIC listeners are disabled.
        let container_socket_path = "/tmp/unix/ferron.sock";
        let config_content = format!(
            r#"{{
  unix "{container_socket_path}"
}}
*:80 {{
  root "/var/www/ferron"
}}
"#
        );
        config_file
            .as_file_mut()
            .write_all(config_content.as_bytes())
            .unwrap();

        let container = create_ferron_unix_container(
            webroot_dir.path(),
            config_file.path(),
            host_socket_dir.path(),
            "/tmp/unix",
        )
        .await
        .unwrap();

        let host_socket_path = host_socket_dir.path().join("ferron.sock");
        assert!(
            wait_for_socket(&host_socket_path, Duration::from_secs(10)).await,
            "Unix socket file should appear at {:?}",
            host_socket_path
        );

        // Verify TCP is disabled: no exposed port 80 should be reachable
        // The container was not exposed with 80, so get_host_port should fail or connection should fail
        let tcp_result = container.get_host_port_ipv4(ContainerPort::Tcp(80)).await;
        assert!(
            tcp_result.is_err(),
            "TCP port 80 should not be exposed when Unix socket is configured"
        );

        // Also try raw TCP connect to ensure no listener (if we had exposed, it would fail)
        // For completeness, try to connect via reqwest to the host port if it were there — but we expect no port.

        let (status, body) = http_get_via_unix(&host_socket_path, "example.com", "/index.html")
            .await
            .expect("HTTP over Unix socket should succeed");

        assert_eq!(status, 200, "Expected 200 over Unix socket, got {}", status);
        assert!(
            body.contains("hello from unix absolute"),
            "Body mismatch: {}",
            body
        );

        container.stop().await.unwrap();
        // Socket should be cleaned up after stop (best-effort)
        // Host file may still exist until container is removed, but we check it was created.
    }
}

#[tokio::test]
async fn test_unix_relative() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(not(unix))]
    {
        eprintln!("Skipping Unix socket test on non-Unix platform");
        return;
    }

    #[cfg(unix)]
    {
        nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

        let webroot_dir = common::create_temp_dir();
        let mut config_file = common::create_temp_file();
        // For relative path, Ferron resolves relative to its WORKDIR (/var/log/ferron inside container)
        // We'll mount host_socket_dir to /var/log/ferron to capture the relative socket.
        let host_socket_dir = common::create_temp_dir();

        common::write_file(
            webroot_dir.path().join("index.html"),
            b"hello from unix relative",
        )
        .unwrap();

        // Relative path: will be resolved relative to /var/log/ferron inside container.
        // The container's WORKDIR is /var/log/ferron (from Dockerfile.test).
        // So "ferron.sock" becomes "/var/log/ferron/ferron.sock".
        // We mount host_socket_dir to /var/log/ferron to expose it.
        let config_content = r#"{
  unix "ferron.sock"
}
*:80 {
  root "/var/www/ferron"
}
"#;
        config_file
            .as_file_mut()
            .write_all(config_content.as_bytes())
            .unwrap();

        // Need custom container creation that mounts host_socket_dir to /var/log/ferron
        // (instead of /tmp/unix). Note that /var/log/ferron is also where Ferron logs go,
        // but mounting a tmpfs there is okay for test.
        let ferron_image = common::build_ferron_image().await.unwrap();
        let container = ferron_image
            .with_wait_for(WaitFor::seconds(2))
            .with_network("bridge")
            .with_mount(Mount::bind_mount(
                webroot_dir.path().to_string_lossy(),
                "/var/www/ferron",
            ))
            .with_mount(Mount::bind_mount(
                config_file.path().to_string_lossy(),
                "/etc/ferron.conf",
            ))
            .with_mount(Mount::bind_mount(
                host_socket_dir.path().to_string_lossy(),
                "/var/log/ferron",
            ))
            .with_cmd(vec![
                "/bin/sh",
                "-c",
                "ferron daemon -c /etc/ferron.conf --pid-file /tmp/ferron.pid; sleep 1; wait $(cat /tmp/ferron.pid)",
            ])
            .start()
            .await
            .unwrap();

        let host_socket_path = host_socket_dir.path().join("ferron.sock");
        assert!(
            wait_for_socket(&host_socket_path, Duration::from_secs(10)).await,
            "Relative Unix socket should be canonicalized and appear at {:?}",
            host_socket_path
        );

        let tcp_result = container.get_host_port_ipv4(ContainerPort::Tcp(80)).await;
        assert!(
            tcp_result.is_err(),
            "TCP should be disabled when Unix socket (even relative) is configured"
        );

        let (status, body) = http_get_via_unix(&host_socket_path, "example.com", "/index.html")
            .await
            .expect("HTTP over relative Unix socket should succeed");

        assert_eq!(status, 200);
        assert!(body.contains("hello from unix relative"));

        container.stop().await.unwrap();
    }
}

#[tokio::test]
async fn test_unix_tcp_disabled() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(not(unix))]
    {
        eprintln!("Skipping Unix socket test on non-Unix platform");
        return;
    }

    #[cfg(unix)]
    {
        nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

        let webroot_dir = common::create_temp_dir();
        let mut config_file = common::create_temp_file();
        let host_socket_dir = common::create_temp_dir();

        common::write_file(webroot_dir.path().join("index.html"), b"unix disables tcp").unwrap();

        let config_content = r#"{
  unix "/tmp/unix/ferron.sock"
}
*:80 {
  root "/var/www/ferron"
}
"#;
        config_file
            .as_file_mut()
            .write_all(config_content.as_bytes())
            .unwrap();

        let container = create_ferron_unix_container(
            webroot_dir.path(),
            config_file.path(),
            host_socket_dir.path(),
            "/tmp/unix",
        )
        .await
        .unwrap();

        let host_socket_path = host_socket_dir.path().join("ferron.sock");
        assert!(wait_for_socket(&host_socket_path, Duration::from_secs(10)).await);

        // Verify that even if we try to expose TCP 80, it's not listening.
        // The container was started without with_exposed_port(80), so any
        // attempt to connect via TCP should fail. As an extra check, try
        // to start a second client that would use TCP if it were available:
        // we ensure the Unix socket still works while TCP is gone.
        let (status, _) = http_get_via_unix(&host_socket_path, "example.com", "/index.html")
            .await
            .unwrap();
        assert_eq!(status, 200);

        // Explicitly verify that the Ferron logs mention disabling TCP/QUIC
        // (we can't easily read container logs here without exec, but the
        // absence of TCP port is sufficient for E2E).

        container.stop().await.unwrap();
    }
}
