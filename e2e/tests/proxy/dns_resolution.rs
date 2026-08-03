use std::io::Write;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor},
    runners::AsyncRunner,
};

async fn create_backend_container(
    network: &str,
    hostname: &str,
    backend_name: &str,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let backend_image = crate::common::build_backend_image().await?;
    backend_image
        .with_exposed_port(ContainerPort::Tcp(3000))
        .with_wait_for(WaitFor::Http(Box::new(
            testcontainers::core::wait::HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(3000))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_hostname(hostname)
        .with_env_var("BACKEND_NAME", backend_name)
        .start()
        .await
}

async fn create_bind9_container(
    network: &str,
    bind9_config_file: &std::path::Path,
    zones_dir: &std::path::Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let bind9_image = crate::common::build_bind9_image().await?;
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
    let ferron_image = crate::common::build_ferron_image().await?;
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
async fn test_proxy_dns_resolution_stale() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let zones_dir = crate::common::create_temp_dir();
    #[cfg(unix)]
    let mut bind9_config = crate::common::create_temp_file();
    #[cfg(unix)]
    let mut ferron_config = crate::common::create_temp_file();
    #[cfg(unix)]
    let mut resolv_conf = crate::common::create_temp_file();

    #[cfg(not(unix))]
    let zones_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut bind9_config = tempfile::NamedTempFile::new().unwrap();
    #[cfg(not(unix))]
    let mut ferron_config = tempfile::NamedTempFile::new().unwrap();
    #[cfg(not(unix))]
    let mut resolv_conf = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-proxy-dns-resolution";

    let backend = create_backend_container(network, "backend", "dns-resolution-backend")
        .await
        .unwrap();
    let backend_ip = backend.get_bridge_ip_address().await.unwrap();

    let another_backend = create_backend_container(network, "backend2", "dns-resolution-backend2")
        .await
        .unwrap();
    let another_backend_ip = another_backend.get_bridge_ip_address().await.unwrap();

    let named_conf = r#"
options {
    directory "/var/lib/bind";
    allow-query { any; };
    dnssec-validation no;
};

zone "dns.test" {
    type primary;
    file "/etc/bind/zones/db.dns.test";
};

zone "." {
    type forward;
    {{FORWARDERS}}
    forward only;
};"#;

    bind9_config
        .as_file_mut()
        .write_all(named_conf.as_bytes())
        .unwrap();

    let zone_file = format!(
        r#"$TTL 1
@   IN  SOA bind9. admin.dns.test. (
            2024051901  ; serial
            3600        ; refresh
            1800        ; retry
            604800      ; expire
            300         ; minimum
            )

    IN  NS  bind9.

backend.dns.test. IN A {backend_ip}
"#
    );

    let alt_zone_file = format!(
        r#"$TTL 1
@   IN  SOA bind9. admin.dns.test. (
            2024051901  ; serial
            3600        ; refresh
            1800        ; retry
            604800      ; expire
            300         ; minimum
            )

    IN  NS  bind9.

backend.dns.test. IN A {another_backend_ip}
"#
    );

    std::fs::write(zones_dir.path().join("db.dns.test"), zone_file.as_bytes()).unwrap();

    let bind9 = create_bind9_container(network, bind9_config.path(), zones_dir.path())
        .await
        .unwrap();

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

    // Prepare Ferron config — use hostname, not IP
    ferron_config
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream http://backend.dns.test:3000
  }
}
"#,
        )
        .unwrap();

    let ferron = create_ferron_container(network, ferron_config.path(), resolv_conf.path())
        .await
        .unwrap();

    let port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://localhost:{port}/whoami"))
        .send()
        .await
        .expect("first request should have succeeded");
    assert!(resp.status().is_success());
    let body = resp.text().await.unwrap();
    assert_eq!(body.trim(), "dns-resolution-backend");

    // Wait for some time for the DNS record to expire
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Replace the zone file with one with another backend IP,
    // and reload the DNS server (requirement when using BIND9 with static zone files)
    std::fs::write(
        zones_dir.path().join("db.dns.test"),
        alt_zone_file.as_bytes(),
    )
    .unwrap();
    // In production setup, SIGHUP or rndc would be used, but since this is just a test setup,
    // we can tolerate some downtime, so stop the container, then start it again.
    bind9.stop_with_timeout(Some(1)).await.unwrap();
    bind9.start().await.unwrap();

    // Wait for more time for the DNS record to expire
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let resp = client
        .get(format!("http://localhost:{port}/whoami"))
        .send()
        .await
        .expect("second request should have succeeded");
    assert!(resp.status().is_success());
    let body = resp.text().await.unwrap();
    assert_eq!(body.trim(), "dns-resolution-backend2");

    ferron.stop().await.unwrap();
}
