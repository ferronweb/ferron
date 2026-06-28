use reqwest::header;
use std::io::{Read, Write};

use testcontainers::{ContainerAsync, GenericImage, core::ContainerPort};

mod common;

struct StaticTestContext {
    _container: ContainerAsync<GenericImage>,
    base_url: String,
    client: reqwest::Client,
    // We need to keep these alive
    _webroot_dir: tempfile::TempDir,
    _config_file: tempfile::NamedTempFile,
}

impl StaticTestContext {
    async fn new() -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();

        let webroot_dir = common::create_temp_dir();
        let mut config_file = common::create_temp_file();

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

        let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
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
        .get(format!("{}/basic.txt", ctx.base_url))
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
        .get(format!("{}/unicode.txt", ctx.base_url))
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
        .get(format!("{}/basic.txt", ctx.base_url))
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
        .get(format!("{}/basic.txt", ctx.base_url))
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
        .get(format!("{}/basic.txt", ctx.base_url))
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
        .get(format!("{}/basic.txt", ctx.base_url))
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
        .get(format!("{}/precompressed/basic.txt", ctx.base_url))
        .header(header::ACCEPT_ENCODING, "gzip")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_ENCODING).unwrap(),
        "gzip"
    );

    // Precompressed responses must include Content-Length
    assert!(response.headers().get(header::CONTENT_LENGTH).is_some());

    let bytes = response.bytes().await.unwrap();
    assert_eq!(bytes.to_vec(), compressed_content);
}

#[tokio::test]
async fn test_partial_content() {
    let ctx = StaticTestContext::new().await;

    // Bytes=0-11
    let response = ctx
        .client
        .get(format!("{}/basic.txt", ctx.base_url))
        .header(header::RANGE, "bytes=0-11")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.text().await.unwrap(), &BASIC_CONTENT[0..12]);

    // Bytes=-999 (Suffix)
    let response = ctx
        .client
        .get(format!("{}/basic.txt", ctx.base_url))
        .header(header::RANGE, "bytes=-999")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.text().await.unwrap(), BASIC_CONTENT);

    // Bytes=999- (Out of range, should return 206 with available content per RFC 7233)
    let response = ctx
        .client
        .get(format!("{}/basic.txt", ctx.base_url))
        .header(header::RANGE, "bytes=999-")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::PARTIAL_CONTENT);

    // Bytes=0-999 (end beyond file, should return 206 with full content)
    let response = ctx
        .client
        .get(format!("{}/basic.txt", ctx.base_url))
        .header(header::RANGE, "bytes=0-999")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.text().await.unwrap(), BASIC_CONTENT);

    // Bytes=100-50 (start > end, unsatisfiable)
    let response = ctx
        .client
        .get(format!("{}/basic.txt", ctx.base_url))
        .header(header::RANGE, "bytes=100-50")
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
        .head(format!("{}/basic.txt", ctx.base_url))
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
        .get(format!("{}/basic.txt", ctx.base_url))
        .header(header::IF_NONE_MATCH, &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_MODIFIED);

    // If-None-Match with gzip
    let response = ctx
        .client
        .head(format!("{}/basic.txt", ctx.base_url))
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
        .get(format!("{}/basic.txt", ctx.base_url))
        .header(header::ACCEPT_ENCODING, "gzip")
        .header(header::IF_NONE_MATCH, etag_gzip)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_MODIFIED);

    // Multiple ETags
    let response = ctx
        .client
        .get(format!("{}/basic.txt", ctx.base_url))
        .header(header::IF_NONE_MATCH, format!("{}, \"something\"", etag))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_MODIFIED);

    // If-Match (Precondition Failed)
    let response = ctx
        .client
        .get(format!("{}/basic.txt", ctx.base_url))
        .header(header::IF_MATCH, &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::PRECONDITION_FAILED);

    // If-Match *
    let response = ctx
        .client
        .get(format!("{}/basic.txt", ctx.base_url))
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
        .get(format!("{}{}", ctx.base_url, traversal_path))
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
        .head(format!("{}/basic.txt", ctx.base_url))
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
        .get(format!("{}/doesntexist.txt", ctx.base_url))
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
        .get(format!("{}/dirlisting", ctx.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let text = response.text().await.unwrap();
    assert!(text.contains("Directory:") || text.contains("dirlisting"));

    // Disabled
    let response = ctx
        .client
        .get(format!("{}/dirnolisting", ctx.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);

    // Trailing slash
    let response = ctx
        .client
        .get(format!("{}/dirlisting/", ctx.base_url))
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
        .get(format!("{}/", ctx.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), BASIC_CONTENT);
}

#[tokio::test]
async fn test_post_if_none_match() {
    let ctx = StaticTestContext::new().await;

    // Get ETag via HEAD
    let response = ctx
        .client
        .head(format!("{}/basic.txt", ctx.base_url))
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

    // POST with matching If-None-Match should return 412 Precondition Failed
    let response = ctx
        .client
        .post(format!("{}/basic.txt", ctx.base_url))
        .header(header::IF_NONE_MATCH, &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::PRECONDITION_FAILED);
}

#[tokio::test]
async fn test_on_the_fly_compression_with_precompressed() {
    let ctx = StaticTestContext::new().await;

    // Create a file in the precompressed directory WITHOUT a .gz counterpart
    // Content must be > 256 bytes for compression to be possible
    let nogz_content = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Maecenas id dignissim leo, ac imperdiet tellus. Orci varius natoque penatibus et magnis dis parturient montes, nascetur ridiculus mus. Maecenas id erat finibus, auctor odio eu, efficitur libero. Aenean aliquet vehicula nisi ac tincidunt. Donec non vulputate dolor. Sed faucibus pulvinar augue eget viverra. Donec ornare lacus non mi mollis lacinia. Nulla suscipit vestibulum maximus.\n";
    common::write_file(
        ctx._webroot_dir.path().join("precompressed/nogz.txt"),
        nogz_content.as_bytes(),
    )
    .unwrap();

    // Request with Accept-Encoding: gzip — should compress on-the-fly
    let response = ctx
        .client
        .get(format!("{}/precompressed/nogz.txt", ctx.base_url))
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
    assert_eq!(decoded, nogz_content);
}

#[tokio::test]
async fn test_multipart_range_content() {
    let ctx = StaticTestContext::new().await;

    // Request two non-contiguous ranges
    let response = ctx
        .client
        .get(format!("{}/basic.txt", ctx.base_url))
        .header(header::RANGE, "bytes=0-11,100-200")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::PARTIAL_CONTENT);

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(content_type.starts_with("multipart/byteranges"));

    let body = response.bytes().await.unwrap();
    let body_str = String::from_utf8_lossy(&body);

    // Each part must have the "bytes " prefix in content-range
    assert!(body_str.contains("content-range: bytes 0-11/"));
    assert!(body_str.contains("content-range: bytes 100-200/"));

    // Extract boundary from Content-Type
    let boundary = content_type.split("boundary=").nth(1).unwrap().to_string();

    // Split on boundary to get individual parts
    let parts: Vec<&str> = body_str.split(&format!("--{}", boundary)).collect();
    // parts[0] = preamble (empty), parts[last] = "--\r\n" (epilogue)
    // Intermediate parts are actual range data

    for (i, part) in parts.iter().enumerate().skip(1).take(parts.len() - 2) {
        // Remove leading \r\n
        let trimmed = part.trim_start_matches("\r\n");
        // Split headers from body
        if let Some(body_section) = trimmed.split_once("\r\n\r\n") {
            let part_body = body_section.1.trim_end_matches("\r\n");
            // Verify each part has at least 1 byte of content
            assert!(!part_body.is_empty(), "part {} should have content", i);
        }
    }
}

#[tokio::test]
async fn test_invalid_range_syntax_returns_200() {
    let ctx = StaticTestContext::new().await;

    // Invalid range syntax should return 200 (treat as absent) per RFC 7233 §3.1
    let response = ctx
        .client
        .get(format!("{}/basic.txt", ctx.base_url))
        .header(header::RANGE, "bytes=abc-def")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), BASIC_CONTENT);
}

#[tokio::test]
async fn test_if_match_star_post() {
    let ctx = StaticTestContext::new().await;

    // If-Match: * with POST should pass (matches any current representation)
    let response = ctx
        .client
        .post(format!("{}/basic.txt", ctx.base_url))
        .header(header::IF_MATCH, "*")
        .send()
        .await
        .unwrap();
    assert_ne!(response.status(), reqwest::StatusCode::PRECONDITION_FAILED);
}

#[tokio::test]
async fn test_if_range_etag_match() {
    let ctx = StaticTestContext::new().await;

    // Get ETag
    let response = ctx
        .client
        .head(format!("{}/basic.txt", ctx.base_url))
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

    // If-Range with matching ETag + Range → 206 Partial Content
    let response = ctx
        .client
        .get(format!("{}/basic.txt", ctx.base_url))
        .header(header::RANGE, "bytes=0-9")
        .header(header::IF_RANGE, &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.text().await.unwrap(), &BASIC_CONTENT[0..10]);
}

#[tokio::test]
async fn test_if_range_etag_mismatch() {
    let ctx = StaticTestContext::new().await;

    // If-Range with non-matching ETag + Range → 200 (full response)
    let response = ctx
        .client
        .get(format!("{}/basic.txt", ctx.base_url))
        .header(header::RANGE, "bytes=0-9")
        .header(header::IF_RANGE, "W/\"nonexistent\"")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), BASIC_CONTENT);
}
