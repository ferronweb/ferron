use std::io::Write;
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor},
    runners::AsyncRunner,
};

mod common;

async fn create_backend_container(
    network: &str,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let backend_image = self::common::build_backend_image().await?;
    backend_image
        .with_exposed_port(ContainerPort::Tcp(3000))
        .with_wait_for(WaitFor::Http(Box::new(
            testcontainers::core::wait::HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(3000))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_hostname("backend")
        .with_env_var("BACKEND_NAME", "srv-backend")
        .start()
        .await
}

async fn create_bind9_container(
    network: &str,
    bind9_config_file: &std::path::Path,
    zones_dir: &std::path::Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let bind9_image = self::common::build_bind9_image().await?;
    bind9_image
        .with_exposed_port(ContainerPort::Tcp(53))
        .with_exposed_port(ContainerPort::Udp(53))
        .with_network(network)
        .with_hostname("bind9")
        .with_mount(Mount::bind_mount(
            bind9_config_file.to_string_lossy(),
            "/etc/bind/named.conf.tmpl",
        ))
        .with_mount(Mount::bind_mount(
            zones_dir.to_string_lossy(),
            "/etc/bind/zones",
        ))
        .start()
        .await
}

async fn create_ferron_container(
    network: &str,
    config_file: &std::path::Path,
    resolv_conf: &std::path::Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_wait_for(WaitFor::Http(Box::new(
            testcontainers::core::wait::HttpWaitStrategy::new("/%")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_hostname("ferron")
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy(),
            "/etc/ferron.conf",
        ))
        .with_mount(Mount::bind_mount(
            resolv_conf.to_string_lossy(),
            "/etc/resolv.conf",
        ))
        .start()
        .await
}

#[tokio::test]
async fn test_proxy_srv_resolution() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let zones_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(unix)]
    let mut bind9_config = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(unix)]
    let mut ferron_config = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(unix)]
    let mut resolv_conf = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();

    #[cfg(not(unix))]
    let zones_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut bind9_config = tempfile::NamedTempFile::new().unwrap();
    #[cfg(not(unix))]
    let mut ferron_config = tempfile::NamedTempFile::new().unwrap();
    #[cfg(not(unix))]
    let mut resolv_conf = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-proxy-srv";

    // Start backend
    let backend = create_backend_container(network).await.unwrap();
    let backend_ip = backend.get_bridge_ip_address().await.unwrap();

    // Prepare BIND9 config
    let tsig_key_name = "ferron-test-key";
    let tsig_key_secret = "c2VjcmV0c2FsdDEyMzQ1Njc4"; // "secretsalt12345678" in base64

    let named_conf = format!(
        r#"key "{tsig_key_name}" {{
    algorithm HMAC-SHA256;
    secret "{tsig_key_secret}";
}};

options {{
    directory "/var/lib/bind";
    allow-query {{ any; }};
    dnssec-validation no;
}};

zone "backend.test" {{
    type primary;
    file "/etc/bind/zones/db.backend.test";
    allow-update {{ key "{tsig_key_name}"; }};
}};

zone "." {{
    type forward;
    {{{{FORWARDERS}}}}
    forward only;
}};"#
    );

    bind9_config
        .as_file_mut()
        .write_all(named_conf.as_bytes())
        .unwrap();

    let zone_file = format!(
        r#"$TTL 300
@   IN  SOA bind9. admin.backend.test. (
            2024051901  ; serial
            3600        ; refresh
            1800        ; retry
            604800      ; expire
            300         ; minimum
            )

    IN  NS  bind9.

_http._tcp.backend.test. IN SRV 10 60 3000 backend.backend.test.
backend.backend.test. IN A {backend_ip}
"#
    );

    std::fs::write(
        zones_dir.path().join("db.backend.test"),
        zone_file.as_bytes(),
    )
    .unwrap();

    // Start BIND9
    let bind9 = create_bind9_container(network, bind9_config.path(), zones_dir.path())
        .await
        .unwrap();

    // Get BIND9 IP using the exposed port
    let bind9_ip_str = "127.0.0.1";
    let bind9_port = bind9
        .get_host_port_ipv4(ContainerPort::Tcp(53))
        .await
        .unwrap();
    println!("--- BIND9 IP: {}, Port: {} ---", bind9_ip_str, bind9_port);

    // Prepare resolv.conf
    resolv_conf
        .as_file_mut()
        .write_all(
            format!(
                "nameserver {}\n",
                bind9.get_bridge_ip_address().await.unwrap()
            )
            .as_bytes(),
        )
        .unwrap();

    // Prepare Ferron config
    ferron_config
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    srv _http._tcp.backend.test
  }
}
"#,
        )
        .unwrap();

    // Start Ferron
    let ferron = create_ferron_container(network, ferron_config.path(), resolv_conf.path())
        .await
        .unwrap();

    let port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    // Request should be proxied to the backend resolved via SRV
    let mut success = false;
    for _ in 0..10 {
        if let Ok(resp) = client
            .get(format!("http://localhost:{port}/whoami"))
            .send()
            .await
        {
            if resp.status().is_success() {
                let body = resp.text().await.unwrap();
                if body.trim() == "srv-backend" {
                    success = true;
                    break;
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    assert!(success, "Failed to proxy request via SRV resolution");

    ferron.stop().await.unwrap();
}
