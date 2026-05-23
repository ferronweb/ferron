use std::io::Read;

use testcontainers::core::ContainerPort;

mod common;

#[tokio::test]
async fn test_dynamic_compression_gzip() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let config_file = common::create_temp_file();

    common::write_file(
        config_file.path().to_path_buf(),
        r#"
*:80 {
  root "/var/www/ferron"

  compressed false
  dynamic_compressed true
}
"#
        .as_bytes(),
    )
    .unwrap();

    common::write_file(webroot_dir.path().join("small.txt"), b"test content").unwrap();

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    // Test gzip compression
    let response = client
        .get(format!("http://localhost:{}/small.txt", port))
        .header("Accept-Encoding", "gzip")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let bytes = response.bytes().await.unwrap();
    let mut decompressed = Vec::new();
    let _ = flate2::read::GzDecoder::new(bytes.as_ref())
        .read_to_end(&mut decompressed)
        .unwrap();
    assert_eq!(decompressed, b"test content");

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_dynamic_compression_deflate() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let config_file = common::create_temp_file();

    common::write_file(
        config_file.path().to_path_buf(),
        r#"
*:80 {
  root "/var/www/ferron"

  compressed false
  dynamic_compressed true
}
"#
        .as_bytes(),
    )
    .unwrap();

    common::write_file(webroot_dir.path().join("small.txt"), b"test content").unwrap();

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    // Test gzip compression
    let response = client
        .get(format!("http://localhost:{}/small.txt", port))
        .header("Accept-Encoding", "deflate")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let bytes = response.bytes().await.unwrap();
    let mut decompressed = Vec::new();
    let _ = flate2::read::DeflateDecoder::new(bytes.as_ref())
        .read_to_end(&mut decompressed)
        .unwrap();
    assert_eq!(decompressed, b"test content");

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_dynamic_compression_brotli() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let config_file = common::create_temp_file();

    common::write_file(
        config_file.path().to_path_buf(),
        r#"
*:80 {
  root "/var/www/ferron"

  compressed false
  dynamic_compressed true
}
"#
        .as_bytes(),
    )
    .unwrap();

    common::write_file(webroot_dir.path().join("small.txt"), b"test content").unwrap();

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    // Test gzip compression
    let response = client
        .get(format!("http://localhost:{}/small.txt", port))
        .header("Accept-Encoding", "br")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let mut bytes = std::io::Cursor::new(response.bytes().await.unwrap());
    let mut decompressed = Vec::new();

    brotli::BrotliDecompress(&mut bytes, &mut decompressed).unwrap();
    assert_eq!(decompressed, b"test content");

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_dynamic_compression_zstd() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let config_file = common::create_temp_file();

    common::write_file(
        config_file.path().to_path_buf(),
        r#"
*:80 {
  root "/var/www/ferron"

  compressed false
  dynamic_compressed true
}
"#
        .as_bytes(),
    )
    .unwrap();

    common::write_file(webroot_dir.path().join("small.txt"), b"test content").unwrap();

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    // Test gzip compression
    let response = client
        .get(format!("http://localhost:{}/small.txt", port))
        .header("Accept-Encoding", "zstd")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let mut bytes = std::io::Cursor::new(response.bytes().await.unwrap());
    let mut decompressed = Vec::new();

    let mut decoder = zstd::Decoder::new(&mut bytes).unwrap();
    std::io::copy(&mut decoder, &mut decompressed).unwrap();

    assert_eq!(decompressed, b"test content");

    container.stop().await.unwrap();
}
