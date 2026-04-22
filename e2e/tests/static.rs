use reqwest::header;
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};
use std::{
    io::{Read, Write},
    path::Path,
};

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

struct StaticTestContext {
    _container: ContainerAsync<GenericImage>,
    base_url: String,
    client: reqwest::Client,
    // We need to keep these alive
    _webroot_dir: tempfile::TempDir,
    #[cfg(unix)]
    _config_file: tempfile::NamedTempFile,
    #[cfg(not(unix))]
    _config_file: tempfile::NamedTempFile,
}

impl StaticTestContext {
    async fn new() -> Self {
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
    index "basic.txt"

    match PRECOMPRESSED {
      request.uri.path ~ "^/precompressed(?:$|/)"
    }

    match DIRLISTING {
      request.uri.path ~ "^/dirlisting(?:$|/)"
    }

    if PRECOMPRESSED {
      precompressed true
    }

    if DIRLISTING {
      directory_listing true
    }
  }
  "#
                .as_bytes(),
            )
            .unwrap();

        let basic_content = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Maecenas id dignissim leo, ac imperdiet tellus. Orci varius natoque penatibus et magnis dis parturient montes, nascetur ridiculus mus. Maecenas id erat finibus, auctor odio eu, efficitur libero. Aenean aliquet vehicula nisi ac tincidunt. Donec non vulputate dolor. Sed faucibus pulvinar augue eget viverra. Donec ornare lacus non mi mollis lacinia. Nulla suscipit vestibulum maximus. Nulla sit amet ex quis purus imperdiet vestibulum eget quis ex. Nullam accumsan nibh massa, vitae rhoncus sapien ultricies vel.";
        let unicode_content = "Thiś iś ą Uńićódę tęśt fiłę.\n";

        common::write_file(
            webroot_dir.path().join("basic.txt"),
            basic_content.as_bytes(),
        )
        .unwrap();
        common::write_file(
            webroot_dir.path().join("unicode.txt"),
            unicode_content.as_bytes(),
        )
        .unwrap();

        common::create_dir(webroot_dir.path().join("dirlisting")).unwrap();
        common::write_file(webroot_dir.path().join("dirlisting/.gitkeep"), b"").unwrap();

        common::create_dir(webroot_dir.path().join("dirnolisting")).unwrap();
        common::write_file(webroot_dir.path().join("dirnolisting/.gitkeep"), b"").unwrap();

        common::create_dir(webroot_dir.path().join("precompressed")).unwrap();
        common::write_file(
            webroot_dir.path().join("precompressed/basic.txt"),
            basic_content.as_bytes(),
        )
        .unwrap();

        // Create precompressed gzip file
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(basic_content.as_bytes()).unwrap();
        let compressed_content = encoder.finish().unwrap();
        common::write_file(
            webroot_dir.path().join("precompressed/basic.txt.gz"),
            &compressed_content,
        )
        .unwrap();

        let container = create_ferron_container(webroot_dir.path(), config_file.path())
            .await
            .unwrap();

        let port = container
            .get_host_port_ipv4(ContainerPort::Tcp(80))
            .await
            .unwrap();
        let base_url = format!("http://localhost:{}", port);
        let client = reqwest::Client::builder()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .build()
            .unwrap();

        Self {
            _container: container,
            base_url,
            client,
            _webroot_dir: webroot_dir,
            _config_file: config_file,
        }
    }
}

const BASIC_CONTENT: &str = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Maecenas id dignissim leo, ac imperdiet tellus. Orci varius natoque penatibus et magnis dis parturient montes, nascetur ridiculus mus. Maecenas id erat finibus, auctor odio eu, efficitur libero. Aenean aliquet vehicula nisi ac tincidunt. Donec non vulputate dolor. Sed faucibus pulvinar augue eget viverra. Donec ornare lacus non mi mollis lacinia. Nulla suscipit vestibulum maximus. Nulla sit amet ex quis purus imperdiet vestibulum eget quis ex. Nullam accumsan nibh massa, vitae rhoncus sapien ultricies vel.";
const UNICODE_CONTENT: &str = "Thiś iś ą Uńićódę tęśt fiłę.\n";

#[tokio::test]
async fn test_basic_serving() {
    let ctx = StaticTestContext::new().await;
    let response = ctx
        .client
        .get(&format!("{}/basic.txt", ctx.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), BASIC_CONTENT);
}

#[tokio::test]
async fn test_unicode_serving() {
    let ctx = StaticTestContext::new().await;
    let response = ctx
        .client
        .get(&format!("{}/unicode.txt", ctx.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), UNICODE_CONTENT);
}

#[tokio::test]
async fn test_compression_gzip() {
    let ctx = StaticTestContext::new().await;
    let response = ctx
        .client
        .get(&format!("{}/basic.txt", ctx.base_url))
        .header(header::ACCEPT_ENCODING, "gzip")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_ENCODING).unwrap(),
        "gzip"
    );
    let bytes = response.bytes().await.unwrap();
    let mut decoder = flate2::read::GzDecoder::new(&bytes[..]);
    let mut decoded = String::new();
    decoder.read_to_string(&mut decoded).unwrap();
    assert_eq!(decoded, BASIC_CONTENT);
}

#[tokio::test]
async fn test_compression_deflate() {
    let ctx = StaticTestContext::new().await;
    let response = ctx
        .client
        .get(&format!("{}/basic.txt", ctx.base_url))
        .header(header::ACCEPT_ENCODING, "deflate")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_ENCODING).unwrap(),
        "deflate"
    );
    let bytes = response.bytes().await.unwrap();
    let mut decoder = flate2::read::DeflateDecoder::new(&bytes[..]);
    let mut decoded = String::new();
    decoder.read_to_string(&mut decoded).unwrap();
    assert_eq!(decoded, BASIC_CONTENT);
}

#[tokio::test]
async fn test_compression_brotli() {
    let ctx = StaticTestContext::new().await;
    let response = ctx
        .client
        .get(&format!("{}/basic.txt", ctx.base_url))
        .header(header::ACCEPT_ENCODING, "br")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_ENCODING).unwrap(),
        "br"
    );
    let bytes = response.bytes().await.unwrap();
    let mut decoder = brotli::Decompressor::new(&bytes[..], 4096);
    let mut decoded = String::new();
    decoder.read_to_string(&mut decoded).unwrap();
    assert_eq!(decoded, BASIC_CONTENT);
}

#[tokio::test]
async fn test_compression_zstd() {
    let ctx = StaticTestContext::new().await;
    let response = ctx
        .client
        .get(&format!("{}/basic.txt", ctx.base_url))
        .header(header::ACCEPT_ENCODING, "zstd")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_ENCODING).unwrap(),
        "zstd"
    );
    let bytes = response.bytes().await.unwrap();
    let decoded = zstd::stream::decode_all(&bytes[..]).unwrap();
    assert_eq!(String::from_utf8(decoded).unwrap(), BASIC_CONTENT);
}

#[tokio::test]
async fn test_precompression() {
    let ctx = StaticTestContext::new().await;

    // Re-create compressed content to verify against
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(BASIC_CONTENT.as_bytes()).unwrap();
    let compressed_content = encoder.finish().unwrap();

    let response = ctx
        .client
        .get(&format!("{}/precompressed/basic.txt", ctx.base_url))
        .header(header::ACCEPT_ENCODING, "gzip")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_ENCODING).unwrap(),
        "gzip"
    );
    let bytes = response.bytes().await.unwrap();
    assert_eq!(bytes.to_vec(), compressed_content);
}

#[tokio::test]
async fn test_partial_content() {
    let ctx = StaticTestContext::new().await;

    // Bytes=0-11
    let response = ctx
        .client
        .get(&format!("{}/basic.txt", ctx.base_url))
        .header(header::RANGE, "bytes=0-11")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.text().await.unwrap(), &BASIC_CONTENT[0..12]);

    // Bytes=-999 (Suffix)
    let response = ctx
        .client
        .get(&format!("{}/basic.txt", ctx.base_url))
        .header(header::RANGE, "bytes=-999")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.text().await.unwrap(), BASIC_CONTENT);

    // Bytes=999- (Out of range)
    let response = ctx
        .client
        .get(&format!("{}/basic.txt", ctx.base_url))
        .header(header::RANGE, "bytes=999-")
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        reqwest::StatusCode::RANGE_NOT_SATISFIABLE
    );

    // Malformed
    let response = ctx
        .client
        .get(&format!("{}/basic.txt", ctx.base_url))
        .header(header::RANGE, "malformed")
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        reqwest::StatusCode::RANGE_NOT_SATISFIABLE
    );
}

#[tokio::test]
async fn test_etags() {
    let ctx = StaticTestContext::new().await;

    // Get ETag
    let response = ctx
        .client
        .head(&format!("{}/basic.txt", ctx.base_url))
        .send()
        .await
        .unwrap();
    let etag = response
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // If-None-Match (Not Modified)
    let response = ctx
        .client
        .get(&format!("{}/basic.txt", ctx.base_url))
        .header(header::IF_NONE_MATCH, &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_MODIFIED);

    // If-None-Match with gzip
    let response = ctx
        .client
        .head(&format!("{}/basic.txt", ctx.base_url))
        .header(header::ACCEPT_ENCODING, "gzip")
        .send()
        .await
        .unwrap();
    let etag_gzip = response
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap();
    let response = ctx
        .client
        .get(&format!("{}/basic.txt", ctx.base_url))
        .header(header::ACCEPT_ENCODING, "gzip")
        .header(header::IF_NONE_MATCH, etag_gzip)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_MODIFIED);

    // Multiple ETags
    let response = ctx
        .client
        .get(&format!("{}/basic.txt", ctx.base_url))
        .header(header::IF_NONE_MATCH, format!("{}, \"something\"", etag))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_MODIFIED);

    // If-Match (Precondition Failed)
    let response = ctx
        .client
        .get(&format!("{}/basic.txt", ctx.base_url))
        .header(header::IF_MATCH, &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::PRECONDITION_FAILED);

    // If-Match *
    let response = ctx
        .client
        .get(&format!("{}/basic.txt", ctx.base_url))
        .header(header::IF_MATCH, "*")
        .send()
        .await
        .unwrap();
    assert_ne!(response.status(), reqwest::StatusCode::PRECONDITION_FAILED);
    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn test_path_traversal() {
    let ctx = StaticTestContext::new().await;
    let traversal_path = "/%2e%2e/%2e%2e/%2e%2e/%2e%2e/etc/passwd";
    let response = ctx
        .client
        .get(&format!("{}{}", ctx.base_url, traversal_path))
        .send()
        .await
        .unwrap();
    assert_ne!(response.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn test_head_request() {
    let ctx = StaticTestContext::new().await;
    let response = ctx
        .client
        .head(&format!("{}/basic.txt", ctx.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(response.headers().get(header::CONTENT_LENGTH).is_some());
}

#[tokio::test]
async fn test_404_not_found() {
    let ctx = StaticTestContext::new().await;
    let response = ctx
        .client
        .get(&format!("{}/doesntexist.txt", ctx.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_directory_listing() {
    let ctx = StaticTestContext::new().await;

    // Enabled
    let response = ctx
        .client
        .get(&format!("{}/dirlisting", ctx.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let text = response.text().await.unwrap();
    assert!(text.contains("Directory:") || text.contains("dirlisting"));

    // Disabled
    let response = ctx
        .client
        .get(&format!("{}/dirnolisting", ctx.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);

    // Trailing slash
    let response = ctx
        .client
        .get(&format!("{}/dirlisting/", ctx.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn test_custom_index() {
    let ctx = StaticTestContext::new().await;
    let response = ctx
        .client
        .get(&format!("{}/", ctx.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), BASIC_CONTENT);
}
