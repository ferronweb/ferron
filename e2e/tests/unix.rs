use std::io::Write;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt,
    core::{ContainerPort, Mount, WaitFor},
    runners::AsyncRunner,
};

mod common;

async fn exec_curl_unix(
    container: &ContainerAsync<GenericImage>,
    socket_path: &str,
    host: &str,
    uri: &str,
) -> Result<(i64, String, String), Box<dyn std::error::Error>> {
    use testcontainers::core::ExecCommand;

    let url = format!("http://{host}{uri}");
    let exec = ExecCommand::new(vec![
        "curl",
        "-s",
        "-S",
        "-w",
        "\n%{http_code}",
        "--unix-socket",
        socket_path,
        &url,
    ]);

    let mut result = container.exec(exec).await?;
    let stdout = result.stdout_to_vec().await?;
    let stderr = result.stderr_to_vec().await?;
    let _exit_code = result.exit_code().await?.unwrap_or(-1);

    let stdout_str = String::from_utf8_lossy(&stdout).to_string();
    let stderr_str = String::from_utf8_lossy(&stderr).to_string();

    // curl -w prints http_code on last line
    let mut lines: Vec<&str> = stdout_str.lines().collect();
    let code_str = lines.pop().unwrap_or("0");
    let code: i64 = code_str.parse().unwrap_or(0);
    let body = lines.join("\n");

    // Also consider exit_code, but http_code is more reliable
    Ok((code, body, stderr_str))
}

async fn exec_test_socket_exists(
    container: &ContainerAsync<GenericImage>,
    socket_path: &str,
) -> bool {
    use testcontainers::core::ExecCommand;
    let exec = ExecCommand::new(vec!["test", "-S", socket_path]);
    match container.exec(exec).await {
        Ok(r) => r.exit_code().await.unwrap_or(Some(-1)).unwrap_or(-1) == 0,
        Err(_) => false,
    }
}

async fn wait_for_socket_in_container(
    container: &ContainerAsync<GenericImage>,
    socket_path: &str,
    timeout: std::time::Duration,
) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if exec_test_socket_exists(container, socket_path).await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    false
}

async fn exec_curl_tcp_should_fail(
    container: &ContainerAsync<GenericImage>,
    port: u16,
) -> bool {
    use testcontainers::core::ExecCommand;
    let url = format!("http://127.0.0.1:{}/", port);
    let exec = ExecCommand::new(vec![
        "curl",
        "-s",
        "--connect-timeout",
        "2",
        &url,
    ]);
    match container.exec(exec).await {
        Ok(r) => {
            let code = r.exit_code().await.unwrap_or(Some(-1)).unwrap_or(-1);
            // curl exit 7 = Failed to connect, 28 = timeout, 0 = success
            // If TCP is disabled, we expect non-zero.
            code != 0
        }
        Err(_) => true,
    }
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

        common::write_file(
            webroot_dir.path().join("index.html"),
            b"hello from unix absolute",
        )
        .unwrap();

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
            .with_cmd(vec![
                "/bin/sh",
                "-c",
                "ferron daemon -c /etc/ferron.conf --pid-file /tmp/ferron.pid; sleep 1; wait $(cat /tmp/ferron.pid)",
            ])
            .start()
            .await
            .unwrap();

        assert!(
            wait_for_socket_in_container(&container, container_socket_path, std::time::Duration::from_secs(10)).await,
            "Unix socket should appear at {} inside container",
            container_socket_path
        );

        // Verify TCP 80 is disabled: container should not expose port 80,
        // and curl to 127.0.0.1:80 inside container should fail
        let tcp_exposed = container
            .get_host_port_ipv4(ContainerPort::Tcp(80))
            .await
            .is_ok();
        assert!(
            !tcp_exposed,
            "TCP port 80 should not be exposed when Unix socket is configured"
        );
        assert!(
            exec_curl_tcp_should_fail(&container, 80).await,
            "curl to 127.0.0.1:80 inside container should fail when Unix is enabled"
        );

        // HTTP over Unix socket
        let (code, body, stderr) =
            exec_curl_unix(&container, container_socket_path, "example.com", "/index.html")
                .await
                .expect("exec curl over Unix socket should succeed");

        assert_eq!(
            code, 200,
            "Expected 200 over Unix socket, got {} stderr={}",
            code, stderr
        );
        assert!(
            body.contains("hello from unix absolute"),
            "Body mismatch: {}",
            body
        );

        container.stop().await.unwrap();
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

        common::write_file(
            webroot_dir.path().join("index.html"),
            b"hello from unix relative",
        )
        .unwrap();

        // Relative path: resolved against server's WORKDIR (/var/log/ferron in Dockerfile.test)
        // So "ferron.sock" becomes "/var/log/ferron/ferron.sock" after canonicalization (if parent exists)
        // The file does not exist beforehand, so it stays relative, then socket2 creates it relative to CWD.
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
            .with_cmd(vec![
                "/bin/sh",
                "-c",
                "ferron daemon -c /etc/ferron.conf --pid-file /tmp/ferron.pid; sleep 1; wait $(cat /tmp/ferron.pid)",
            ])
            .start()
            .await
            .unwrap();

        // Relative socket should appear at /var/log/ferron/ferron.sock inside container
        let container_socket_path = "/var/log/ferron/ferron.sock";
        assert!(
            wait_for_socket_in_container(&container, container_socket_path, std::time::Duration::from_secs(10)).await,
            "Relative Unix socket should be canonicalized and appear at {}",
            container_socket_path
        );

        // Also check that the relative name without prefix works via curl's --unix-socket
        // (curl resolves relative to its CWD, but we use absolute inside container)
        let (code, body, _) =
            exec_curl_unix(&container, container_socket_path, "example.com", "/index.html")
                .await
                .expect("HTTP over relative Unix socket should succeed");

        assert_eq!(code, 200);
        assert!(body.contains("hello from unix relative"));

        // TCP should still be disabled
        assert!(
            container
                .get_host_port_ipv4(ContainerPort::Tcp(80))
                .await
                .is_err(),
            "TCP should be disabled for relative unix socket"
        );

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
            .with_cmd(vec![
                "/bin/sh",
                "-c",
                "ferron daemon -c /etc/ferron.conf --pid-file /tmp/ferron.pid; sleep 1; wait $(cat /tmp/ferron.pid)",
            ])
            .start()
            .await
            .unwrap();

        let container_socket_path = "/tmp/unix/ferron.sock";
        assert!(
            wait_for_socket_in_container(&container, container_socket_path, std::time::Duration::from_secs(10)).await,
            "Unix socket should exist"
        );

        // Unix should work
        let (code, _, _) = exec_curl_unix(&container, container_socket_path, "example.com", "/index.html")
            .await
            .unwrap();
        assert_eq!(code, 200);

        // TCP must be disabled: exec curl to 127.0.0.1:80 should fail
        assert!(
            exec_curl_tcp_should_fail(&container, 80).await,
            "TCP 80 should be disabled when Unix is enabled"
        );
        // Also host-side mapping should not exist
        assert!(
            container
                .get_host_port_ipv4(ContainerPort::Tcp(80))
                .await
                .is_err()
        );

        container.stop().await.unwrap();
    }
}
