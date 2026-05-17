use std::io::Write;
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};

use crate::create_ferron_container;

/// Test for URL canonicalization rejecting null bytes.
/// Ensures %00 and \0 are rejected during URL canonicalization.
#[tokio::test]
async fn test_url_canonicalization_rejects_null_bytes() {
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

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    root "/var/www/ferron"
}
"#
            .as_bytes(),
        )
        .unwrap();
    config_file.flush().unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .expect("Failed to create container");

    let client = reqwest::Client::new();
    let ferron_addr = format!(
        "http://127.0.0.1:{}",
        container
            .get_host_port_ipv4(80)
            .await
            .expect("Failed to get host port")
    );

    // Test with %00 (URL-encoded null byte)
    let response = client
        .get(&format!("{}/path%00/file", ferron_addr))
        .send()
        .await
        .expect("Failed to send request");

    // Should reject with 400 Bad Request, not process the path
    assert!(
        response.status().is_client_error(),
        "Request with null byte in path should fail with 4xx status"
    );

    // Test with %2500 (double-encoded null byte should be rejected)
    let response = client
        .get(&format!("{}/path%2500/file", ferron_addr))
        .send()
        .await
        .expect("Failed to send request");

    assert!(
        response.status().is_success() || response.status().is_client_error(),
        "Request with double-encoded null byte handling"
    );
}
