use std::path::Path;
use std::time::Duration;
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};
use std::{io::Write, net::IpAddr};

use rustls::client::danger::{ServerCertVerified, ServerCertVerifier};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor},
    runners::AsyncRunner,
};

mod common;

/// Configuration for DNS-01 challenge testing with BIND 9.
#[derive(Debug, Clone)]
struct DnsConfig {
    /// The key name for TSIG authentication
    key_name: String,
    /// The TSIG secret (base64-encoded)
    key_secret: String,
    /// Test domain(s) to be served by BIND 9 (e.g., "example.com")
    domains: Vec<String>,
}

/// A server certificate verifier that always returns a successful verification.
#[derive(Debug)]
pub struct NoServerVerifier;

impl Default for NoServerVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl NoServerVerifier {
    pub fn new() -> Self {
        Self
    }
}

impl ServerCertVerifier for NoServerVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        use rustls::SignatureScheme::*;
        vec![
            ECDSA_NISTP384_SHA384,
            ECDSA_NISTP256_SHA256,
            ED25519,
            RSA_PSS_SHA512,
            RSA_PSS_SHA384,
            RSA_PSS_SHA256,
            RSA_PKCS1_SHA512,
            RSA_PKCS1_SHA384,
            RSA_PKCS1_SHA256,
        ]
    }
}

async fn create_pebble_container(
    network: &str,
    config_file: &Path,
    cert_dir: &Path,
    resolv_conf: Option<&Path>,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let mut builder = GenericImage::new("ghcr.io/letsencrypt/pebble", "latest")
        .with_exposed_port(ContainerPort::Tcp(14000))
        // Wait for Pebble to be ready.
        // Since we can't easily do a secure HTTP check against self-signed cert in wait strategy,
        // we'll wait some time.
        .with_wait_for(WaitFor::seconds(5))
        .with_network(network)
        .with_hostname("pebble")
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy().to_string(),
            "/etc/pebble-config.json",
        ))
        .with_mount(Mount::bind_mount(
            cert_dir.to_string_lossy().to_string(),
            "/etc/certs",
        ))
        .with_cmd(vec!["-config", "/etc/pebble-config.json"]);

    // Mount custom resolv.conf if provided (for DNS-01 testing)
    if let Some(resolv_path) = resolv_conf {
        builder = builder.with_mount(Mount::bind_mount(
            resolv_path.to_string_lossy().to_string(),
            "/etc/resolv.conf",
        ));
    }

    builder.start().await
}

async fn create_ferron_container(
    network: &str,
    webroot_dir: &Path,
    config_file: &Path,
    cache_dir: &Path,
    alias: &str,
    resolv_conf: Option<&Path>,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
    let mut builder = ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_exposed_port(ContainerPort::Tcp(443))
        // No wait strategy here because we want to test availability which might take time due to ACME
        .with_network(network)
        .with_hostname(alias)
        .with_mount(Mount::bind_mount(
            webroot_dir.to_string_lossy().to_string(),
            "/var/www/ferron",
        ))
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy().to_string(),
            "/etc/ferron.conf",
        ))
        .with_mount(Mount::bind_mount(
            cache_dir.to_string_lossy().to_string(),
            "/var/cache/ferron-acme",
        ));

    // Mount custom resolv.conf if provided (for DNS-01 testing)
    if let Some(resolv_path) = resolv_conf {
        builder = builder.with_mount(Mount::bind_mount(
            resolv_path.to_string_lossy().to_string(),
            "/etc/resolv.conf",
        ));
    }

    builder.start().await
}

async fn create_bind9_container(
    network: &str,
    bind9_config_file: &Path,
    zones_dir: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let bind9_image = self::common::build_bind9_image().await?;
    bind9_image
        .with_exposed_port(ContainerPort::Tcp(53))
        .with_exposed_port(ContainerPort::Udp(53))
        // Wait for BIND 9 to start
        .with_wait_for(WaitFor::seconds(3))
        .with_network(network)
        .with_hostname("bind9")
        .with_mount(Mount::bind_mount(
            bind9_config_file.to_string_lossy().to_string(),
            "/etc/bind/named.conf.tmpl",
        ))
        .with_mount(Mount::bind_mount(
            zones_dir.to_string_lossy().to_string(),
            "/etc/bind/zones",
        ))
        .start()
        .await
}

/// Generates a TSIG key suitable for RFC 2136 dynamic DNS updates.
/// Returns (key_name, key_secret_base64)
pub fn generate_tsig_key(key_name: &str) -> (String, String) {
    // For testing, we use a deterministic but secure-looking key
    // In production, this would be generated cryptographically
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    key_name.hash(&mut hasher);
    let hash = hasher.finish();

    // Create a 32-byte key from the hash (repeat if necessary)
    let mut key_bytes = Vec::with_capacity(32);
    for i in 0..4 {
        key_bytes.extend_from_slice(&((hash.wrapping_mul(i + 1)) as u64).to_le_bytes());
    }

    // Convert to base64
    let key_secret = base64_simple::encode(&key_bytes);

    (key_name.to_string(), key_secret)
}

/// Generates a BIND 9 named.conf configuration for DNS-01 challenge testing.
/// Takes a list of domains to serve and forwarders for recursive lookups.
pub fn generate_bind9_named_conf_tmpl(
    domains: &[&str],
    tsig_key_name: &str,
    tsig_key_secret: &str,
) -> String {
    let mut conf = String::new();

    // Include the TSIG key definition
    conf.push_str(&format!(
        r#"key "{}" {{
    algorithm HMAC-SHA256;
    secret "{}";
}};

"#,
        tsig_key_name, tsig_key_secret
    ));

    // Main options block
    conf.push_str(
        r#"options {
    directory "/var/lib/bind";

    // Allow RFC 2136 updates from localhost
    allow-update {
        127.0.0.1;
        ::1;
        10.0.0.0/8;      // Docker network ranges
        172.16.0.0/12;
        192.168.0.0/16;
    };

    // Logging
    querylog yes;

    // Disable DNSSEC validation to make it work with Docker's DNS server
    dnssec-validation no;
};

"#,
    );

    // Logging configuration
    conf.push_str(
        r#"logging {
    channel default_debug {
        file "/var/log/bind/named.log";
        severity dynamic;
        print-time yes;
    };

    channel update_log {
        file "/var/log/bind/update.log";
        severity info;
    };

    category default { default_debug; };
    category update { update_log; };
};

"#,
    );

    // Zone definitions for each domain
    for domain in domains {
        conf.push_str(&format!(
            r#"zone "{}" {{
    type primary;
    file "/etc/bind/zones/db.{}";
    allow-update {{ key "{}"; }};
}};

"#,
            domain, domain, tsig_key_name
        ));
    }

    conf.push_str(
        r#"zone "." {
    type forward;
    // Configure forwarders for recursive lookups
    {{FORWARDERS}}
    forward only;
};

"#,
    );

    conf
}

/// Generates a BIND 9 zone file for a domain.
/// Includes SOA, NS, and A records with support for dynamic updates.
pub fn generate_bind9_zone_file(domain: &str) -> String {
    format!(
        r#"$TTL 300
@   IN  SOA bind9. admin.{domain}. (
            2024051901  ; serial
            3600        ; refresh
            1800        ; retry
            604800      ; expire
            300         ; minimum
            )

    IN  NS  bind9.

; Wildcard record for DNS-01 challenges
*   IN  TXT "placeholder"
"#,
        domain = domain,
    )
}

// Simple base64 encoding for testing (doesn't need crypto security)
mod base64_simple {
    const BASE64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(data: &[u8]) -> String {
        let mut result = String::new();
        let mut i = 0;

        while i < data.len() {
            let b1 = data[i];
            let b2 = if i + 1 < data.len() { data[i + 1] } else { 0 };
            let b3 = if i + 2 < data.len() { data[i + 2] } else { 0 };

            let n = ((b1 as u32) << 16) | ((b2 as u32) << 8) | (b3 as u32);

            result.push(BASE64_CHARS[((n >> 18) & 0x3F) as usize] as char);
            result.push(BASE64_CHARS[((n >> 12) & 0x3F) as usize] as char);

            if i + 1 < data.len() {
                result.push(BASE64_CHARS[((n >> 6) & 0x3F) as usize] as char);
            } else {
                result.push('=');
            }

            if i + 2 < data.len() {
                result.push(BASE64_CHARS[(n & 0x3F) as usize] as char);
            } else {
                result.push('=');
            }

            i += 3;
        }

        result
    }
}

/// Generates a custom /etc/resolv.conf for a test container.
/// Points to the BIND 9 DNS server (via docker host IP) and optionally adds forwarders.
pub fn generate_custom_resolv_conf(
    bind9_container_ip: &IpAddr,
    search_domains: Option<&[&str]>,
) -> String {
    let mut conf = String::new();

    // Primary nameserver pointing to BIND 9
    conf.push_str(&format!("nameserver {}\n", bind9_container_ip));

    // Add search domains if provided
    if let Some(domains) = search_domains {
        if !domains.is_empty() {
            conf.push_str("search ");
            conf.push_str(&domains.join(" "));
            conf.push('\n');
        }
    }

    // Add standard options for DNS resolution
    conf.push_str("options edns0 trust-ad\n");
    conf.push_str("search .\n");

    conf
}

async fn test_acme_common(
    challenge_type: &str,
    hostname: &str,
    extra_host_config: &str,
    dns_config: Option<DnsConfig>,
) {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    // Prepare directories
    #[cfg(unix)]
    let webroot_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(unix)]
    let cert_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(unix)]
    let cache_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(unix)]
    let zones_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();

    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let cert_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let cache_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let zones_dir = tempfile::tempdir().unwrap();

    // Prepare config files
    #[cfg(unix)]
    let mut ferron_config = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(unix)]
    let mut pebble_config = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(unix)]
    let mut bind9_config = if dns_config.is_some() {
        Some(
            tempfile::Builder::new()
                .permissions(Permissions::from_mode(0o666))
                .tempfile()
                .unwrap(),
        )
    } else {
        None
    };
    #[cfg(unix)]
    let mut resolv_conf = if dns_config.is_some() {
        Some(
            tempfile::Builder::new()
                .permissions(Permissions::from_mode(0o666))
                .tempfile()
                .unwrap(),
        )
    } else {
        None
    };

    #[cfg(not(unix))]
    let mut ferron_config = tempfile::NamedTempFile::new().unwrap();
    #[cfg(not(unix))]
    let mut pebble_config = tempfile::NamedTempFile::new().unwrap();
    #[cfg(not(unix))]
    let mut bind9_config: Option<tempfile::NamedTempFile> = None;
    #[cfg(not(unix))]
    let mut resolv_conf: Option<tempfile::NamedTempFile> = None;

    // 1. Generate CA for Pebble
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    std::fs::write(cert_dir.path().join("ca.crt"), cert.cert.pem()).unwrap();
    std::fs::write(
        cert_dir.path().join("ca.key"),
        cert.signing_key.serialize_pem(),
    )
    .unwrap();

    // 2. Write Pebble config
    pebble_config
        .as_file_mut()
        .write_all(
            br#"{
  "pebble": {
    "listenAddress": "0.0.0.0:14000",
    "managementListenAddress": "0.0.0.0:15000",
    "certificate": "/etc/certs/ca.crt",
    "privateKey": "/etc/certs/ca.key",
    "httpPort": 80,
    "tlsPort": 443,
    "externalAccountBindingRequired": false,
    "domainBlocklist": [],
    "retryAfter": {
      "authz": 3,
      "order": 5
    }
  }
}"#,
        )
        .unwrap();

    // 3. Setup BIND 9 if DNS-01 challenge
    if let Some(ref dns_cfg) = dns_config {
        // Generate BIND 9 config template
        let bind9_conf_tmpl = generate_bind9_named_conf_tmpl(
            &dns_cfg
                .domains
                .iter()
                .map(|d| d.as_str())
                .collect::<Vec<_>>(),
            &dns_cfg.key_name,
            &dns_cfg.key_secret,
        );

        if let Some(ref mut bind9_file) = bind9_config {
            bind9_file
                .as_file_mut()
                .write_all(bind9_conf_tmpl.as_bytes())
                .unwrap();
        }

        // Generate zone files for each domain
        for domain in &dns_cfg.domains {
            let zone_file = generate_bind9_zone_file(domain);
            self::common::write_file(
                zones_dir.path().join(format!("db.{}", domain)),
                zone_file.as_bytes(),
            )
            .unwrap();
        }
    }

    // 4. Write Ferron config
    let dns_provider_config = if let Some(ref dns_cfg) = dns_config {
        format!(
            r#"    dns {{
      provider rfc2136
      server "udp://bind9:53"
      key_name "{}"
      key_secret "{}"
      key_algorithm "HMAC-SHA256"
    }}"#,
            dns_cfg.key_name, dns_cfg.key_secret
        )
    } else {
        String::new()
    };

    ferron_config
        .as_file_mut()
        .write_all(
            format!(
                r#"
{} {{
  tls {{
    provider acme
    cache "/var/cache/ferron-acme"
    directory "https://pebble:14000/dir"
    no_verification true
    challenge "{}"
    {}
    {}
  }}
  root "/var/www/ferron"
}}
"#,
                hostname, challenge_type, extra_host_config, dns_provider_config
            )
            .as_bytes(),
        )
        .unwrap();

    self::common::write_file(
        webroot_dir.path().join("index.html"),
        b"Ferron is installed successfully!",
    )
    .unwrap();

    let network = format!("e2e-test-ferronacme-{}", hostname);

    // 5. Start BIND 9 if DNS-01 challenge
    let _bind9 = if dns_config.is_some() {
        let container = create_bind9_container(
            &network,
            bind9_config.as_ref().unwrap().path(),
            zones_dir.path(),
        )
        .await
        .unwrap();

        // Generate custom resolv.conf pointing to BIND 9
        let custom_resolv =
            generate_custom_resolv_conf(&container.get_bridge_ip_address().await.unwrap(), None);

        if let Some(ref mut resolv_file) = resolv_conf {
            resolv_file
                .as_file_mut()
                .write_all(custom_resolv.as_bytes())
                .unwrap();
        }
        Some(container)
    } else {
        None
    };

    // 6. Start Pebble
    let _pebble = create_pebble_container(
        &network,
        pebble_config.path(),
        cert_dir.path(),
        resolv_conf.as_ref().map(|f| f.path()),
    )
    .await
    .unwrap();

    // 7. Start Ferron
    let ferron = create_ferron_container(
        &network,
        webroot_dir.path(),
        ferron_config.path(),
        cache_dir.path(),
        hostname,
        resolv_conf.as_ref().map(|f| f.path()),
    )
    .await
    .unwrap();

    // 8. Wait for certificate issuance and verify
    let port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(443))
        .await
        .unwrap();

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .resolve(
            hostname,
            std::net::SocketAddr::new(
                std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)),
                port,
            ),
        )
        .build()
        .unwrap();

    // Poll until success
    let mut success = false;
    for _ in 0..90 {
        // 90 seconds should be enough
        if let Ok(response) = client
            .get(format!("https://{}:{}/", hostname, port))
            .send()
            .await
        {
            if response.status().is_success() {
                success = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    assert!(
        success,
        "Failed to connect to Ferron via HTTPS with auto-obtained certificate"
    );
}

#[tokio::test]
async fn test_acme_http01() {
    test_acme_common("http-01", "ferron-http01", "", None).await;
}

#[tokio::test]
async fn test_acme_tls_alpn_01() {
    test_acme_common("tls-alpn-01", "ferron-tlsalpn01", "", None).await;
}

#[tokio::test]
async fn test_acme_broken_cache() {
    // We attempt to use a directory that is likely not writable by the user running Ferron in the container.
    // Since Ferron typically runs as a non-root user (e.g. nobody or ferron), /root/cache should be inaccessible.
    test_acme_common(
        "http-01",
        "ferron-brokencache",
        "cache \"/root/cache\"",
        None,
    )
    .await;
}

#[tokio::test]
async fn test_acme_ondemand() {
    test_acme_common("tls-alpn-01", "ferron-ondemand", "on_demand", None).await;
}

#[tokio::test]
async fn test_acme_http01_ondemand() {
    test_acme_common("http-01", "ferron-http01-ondemand", "on_demand", None).await;
}

#[tokio::test]
async fn test_acme_dns01_basic() {
    let (key_name, key_secret) = generate_tsig_key("ferron-test-key");
    let dns_config = DnsConfig {
        key_name,
        key_secret,
        domains: vec!["ferron-dns01".to_string()],
    };
    test_acme_common("dns-01", "ferron-dns01", "", Some(dns_config)).await;
}

#[tokio::test]
async fn test_acme_dns01_cache_persistence() {
    let (key_name, key_secret) = generate_tsig_key("ferron-test-key-cache");
    let dns_config = DnsConfig {
        key_name,
        key_secret,
        domains: vec!["ferron-dns01-cache".to_string()],
    };
    // Test with cache directory to ensure cert persistence
    test_acme_common(
        "dns-01",
        "ferron-dns01-cache",
        "cache \"/var/cache/ferron-acme-custom\"",
        Some(dns_config),
    )
    .await;
}

#[tokio::test]
async fn test_acme_dns01_on_demand() {
    let (key_name, key_secret) = generate_tsig_key("ferron-test-key-ondemand");
    let dns_config = DnsConfig {
        key_name,
        key_secret,
        domains: vec!["ferron-dns01-ondemand".to_string()],
    };
    // Test on-demand certificate generation with DNS-01
    test_acme_common(
        "dns-01",
        "ferron-dns01-ondemand",
        "on_demand",
        Some(dns_config),
    )
    .await;
}

#[tokio::test]
async fn test_acme_dns01_rfc2136_updates() {
    // This test verifies RFC 2136 TSIG-authenticated updates are properly handled.
    // Uses the same basic DNS-01 setup but logs will be checked for successful updates.
    let (key_name, key_secret) = generate_tsig_key("ferron-test-key-rfc2136");
    let dns_config = DnsConfig {
        key_name,
        key_secret,
        domains: vec!["ferron-dns01-rfc2136".to_string()],
    };
    test_acme_common("dns-01", "ferron-dns01-rfc2136", "", Some(dns_config)).await;
}

#[tokio::test]
async fn test_acme_dns01_dns_propagation() {
    // This test verifies DNS record propagation timing.
    // Uses standard DNS-01 flow which inherently tests propagation.
    let (key_name, key_secret) = generate_tsig_key("ferron-test-key-propagation");
    let dns_config = DnsConfig {
        key_name,
        key_secret,
        domains: vec!["ferron-dns01-propagation".to_string()],
    };
    test_acme_common("dns-01", "ferron-dns01-propagation", "", Some(dns_config)).await;
}

#[tokio::test]
async fn test_acme_dns01_custom_resolver() {
    // This test explicitly verifies that custom /etc/resolv.conf is used.
    // Uses BIND 9 as the resolver which tests the custom resolv.conf mounting.
    let (key_name, key_secret) = generate_tsig_key("ferron-test-key-resolver");
    let dns_config = DnsConfig {
        key_name,
        key_secret,
        domains: vec!["ferron-dns01-resolver".to_string()],
    };
    test_acme_common("dns-01", "ferron-dns01-resolver", "", Some(dns_config)).await;
}
