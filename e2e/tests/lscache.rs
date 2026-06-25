use std::io::Write;

use reqwest::Method;
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

mod common;

async fn create_backend_container(
    network: &str,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let backend_image = self::common::build_lscache_backend_image().await?;
    backend_image
        .with_exposed_port(ContainerPort::Tcp(3000))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(3000))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_hostname("backend")
        .start()
        .await
}

async fn create_ferron_container(
    network: &str,
    config_file: &std::path::Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_hostname("ferron")
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy().to_string(),
            "/etc/ferron.conf",
        ))
        .start()
        .await
}

struct LSCacheTestContext {
    _backend: ContainerAsync<GenericImage>,
    _ferron: ContainerAsync<GenericImage>,
    base_url: String,
    client: reqwest::Client,
}

impl LSCacheTestContext {
    async fn new(test_name: &str, config: &str) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();

        #[cfg(unix)]
        nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

        #[cfg(unix)]
        let mut config_file = common::create_temp_file();
        #[cfg(not(unix))]
        let mut config_file = tempfile::NamedTempFile::new().unwrap();

        let network = format!("e2e-test-lscache-{}", test_name);

        let backend = create_backend_container(&network).await.unwrap();

        config_file
            .as_file_mut()
            .write_all(config.as_bytes())
            .unwrap();

        let ferron = create_ferron_container(&network, config_file.path())
            .await
            .unwrap();

        let port = ferron
            .get_host_port_ipv4(ContainerPort::Tcp(80))
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        Self {
            _backend: backend,
            _ferron: ferron,
            base_url: format!("http://localhost:{}", port),
            client: reqwest::Client::new(),
        }
    }

    async fn get(&self, path: &str) -> reqwest::Response {
        self.client
            .get(format!(
                "{}/{}",
                self.base_url,
                path.trim_start_matches('/')
            ))
            .send()
            .await
            .unwrap()
    }

    async fn get_with_headers(&self, path: &str, headers: &[(&str, &str)]) -> reqwest::Response {
        let mut req = self.client.get(format!(
            "{}/{}",
            self.base_url,
            path.trim_start_matches('/')
        ));
        for (name, value) in headers {
            req = req.header(*name, *value);
        }
        req.send().await.unwrap()
    }

    async fn purge(&self, path: &str) -> reqwest::Response {
        let method = Method::from_bytes(b"PURGE").unwrap();
        self.client
            .request(
                method,
                format!("{}/{}", self.base_url, path.trim_start_matches('/')),
            )
            .send()
            .await
            .unwrap()
    }
}

const BASE_CONFIG_EMIT_LS: &str = r#"
*:80 {
  proxy "http://backend:3000"
  cache {
    emit_litespeed_headers true
  }
}
"#;

const BASE_CONFIG_OVERRIDE_LS: &str = r#"
*:80 {
  proxy "http://backend:3000"
  cache {
    emit_litespeed_headers true
    litespeed_override_cache_control true
  }
}
"#;

#[tokio::test]
async fn test_lscache_miss_then_hit() {
    let ctx = LSCacheTestContext::new("miss-hit", BASE_CONFIG_EMIT_LS).await;

    let resp = ctx
        .get_with_headers(
            "/cache-test",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Body", "first"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert_eq!(ls_cache, "miss");
    assert_eq!(resp.text().await.unwrap(), "first");

    let resp = ctx
        .get_with_headers(
            "/cache-test",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Body", "second"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert_eq!(ls_cache, "hit");
    assert_eq!(resp.text().await.unwrap(), "first");
}

#[tokio::test]
async fn test_lscache_private_cache() {
    let ctx = LSCacheTestContext::new("private", BASE_CONFIG_EMIT_LS).await;

    let resp = ctx
        .get_with_headers(
            "/private-test",
            &[
                ("X-Test-Cache-Control", "private,max-age=60"),
                ("X-Test-Body", "private-content"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert_eq!(ls_cache, "miss");

    let resp = ctx
        .get_with_headers(
            "/private-test",
            &[
                ("X-Test-Cache-Control", "private,max-age=60"),
                ("X-Test-Body", "private-content"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert_eq!(ls_cache, "hit,private");
}

#[tokio::test]
async fn test_lscache_no_store() {
    let ctx = LSCacheTestContext::new("no-store", BASE_CONFIG_EMIT_LS).await;

    let resp = ctx
        .get_with_headers(
            "/nostore",
            &[
                ("X-Test-Cache-Control", "no-store"),
                ("X-Test-Body", "v1"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert!(ls_cache == "miss" || ls_cache == "bypass");
    assert_eq!(resp.text().await.unwrap(), "v1");

    let resp = ctx
        .get_with_headers(
            "/nostore",
            &[
                ("X-Test-Cache-Control", "no-store"),
                ("X-Test-Body", "v2"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "v2");
}

#[tokio::test]
async fn test_lscache_no_cache() {
    let ctx = LSCacheTestContext::new("no-cache", BASE_CONFIG_EMIT_LS).await;

    let resp = ctx
        .get_with_headers(
            "/nocache",
            &[
                ("X-Test-Cache-Control", "no-cache"),
                ("X-Test-Body", "v1"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "v1");

    let resp = ctx
        .get_with_headers(
            "/nocache",
            &[
                ("X-Test-Cache-Control", "no-cache"),
                ("X-Test-Body", "v2"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "v2");
}

#[tokio::test]
async fn test_lscache_override_standard_no_store() {
    let ctx = LSCacheTestContext::new("override", BASE_CONFIG_OVERRIDE_LS).await;

    let resp = ctx
        .get_with_headers(
            "/override",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Upstream-Cache-Control", "no-store"),
                ("X-Test-Body", "cached"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "cached");

    let resp = ctx
        .get_with_headers(
            "/override",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Upstream-Cache-Control", "no-store"),
                ("X-Test-Body", "should-not-appear"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert_eq!(ls_cache, "hit");
    assert_eq!(resp.text().await.unwrap(), "cached");
}

#[tokio::test]
async fn test_lscache_vary_cookie() {
    let ctx = LSCacheTestContext::new("vary-cookie", BASE_CONFIG_EMIT_LS).await;

    let resp = ctx
        .get_with_headers(
            "/vary-cookie",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Vary", "cookie=session"),
                ("X-Test-Body", "no-cookie"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "no-cookie");

    let resp = ctx
        .get_with_headers(
            "/vary-cookie",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Vary", "cookie=session"),
                ("X-Test-Body", "session-abc"),
                ("Cookie", "session=abc"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "session-abc");

    let resp = ctx
        .get_with_headers(
            "/vary-cookie",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Vary", "cookie=session"),
                ("X-Test-Body", "session-abc-repeat"),
                ("Cookie", "session=abc"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "session-abc");

    let resp = ctx
        .get_with_headers(
            "/vary-cookie",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Vary", "cookie=session"),
                ("X-Test-Body", "no-cookie-hit"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "no-cookie");
}

#[tokio::test]
async fn test_lscache_tag() {
    let ctx = LSCacheTestContext::new("tag", BASE_CONFIG_EMIT_LS).await;

    let resp = ctx
        .get_with_headers(
            "/tag-test",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Tag", "Page1,Cat1"),
                ("X-Test-Body", "tagged"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert_eq!(ls_cache, "miss");
    assert_eq!(resp.text().await.unwrap(), "tagged");

    let resp = ctx
        .get_with_headers(
            "/tag-test",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Tag", "Page1,Cat1"),
                ("X-Test-Body", "tagged-hit"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert_eq!(ls_cache, "hit");
    assert_eq!(resp.text().await.unwrap(), "tagged");
}

#[tokio::test]
async fn test_lscache_purge_by_tag() {
    let ctx = LSCacheTestContext::new("purge-tag", BASE_CONFIG_EMIT_LS).await;

    let resp = ctx
        .get_with_headers(
            "/purge-tag-test",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Tag", "MyPage"),
                ("X-Test-Body", "before-purge"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "before-purge");

    let resp = ctx
        .get_with_headers(
            "/purge-tag-test",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Tag", "MyPage"),
                ("X-Test-Body", "before-purge-verify"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert_eq!(ls_cache, "hit");
    assert_eq!(resp.text().await.unwrap(), "before-purge");

    let resp = ctx
        .get_with_headers(
            "/purge-trigger",
            &[
                ("X-Test-Purge", "tag=MyPage"),
                ("X-Test-Body", "purge-response"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let resp = ctx
        .get_with_headers(
            "/purge-tag-test",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Tag", "MyPage"),
                ("X-Test-Body", "after-purge"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert_eq!(ls_cache, "miss");
    assert_eq!(resp.text().await.unwrap(), "after-purge");
}

#[tokio::test]
async fn test_lscache_purge_by_url() {
    let ctx = LSCacheTestContext::new("purge-url", BASE_CONFIG_EMIT_LS).await;

    let resp = ctx.get("/url-purge-test").await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert_eq!(ls_cache, "miss");
    assert_eq!(resp.text().await.unwrap(), "OK");

    let resp = ctx.get("/url-purge-test").await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert_eq!(ls_cache, "hit");

    let resp = ctx
        .get_with_headers(
            "/purge-url-trigger",
            &[
                ("X-Test-Purge", "url=/url-purge-test"),
                ("X-Test-Body", "purge-url-response"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let resp = ctx.get("/url-purge-test").await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert_eq!(ls_cache, "miss");
}

#[tokio::test]
async fn test_lscache_purge_all() {
    let ctx = LSCacheTestContext::new("purge-all", BASE_CONFIG_EMIT_LS).await;

    let resp = ctx
        .get_with_headers(
            "/page-a",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Tag", "TagA"),
                ("X-Test-Body", "page-a"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let resp = ctx
        .get_with_headers(
            "/page-b",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Tag", "TagB"),
                ("X-Test-Body", "page-b"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let resp = ctx
        .get_with_headers("/purge-all-trigger", &[("X-Test-Purge", "*")])
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let resp = ctx
        .get_with_headers(
            "/page-a",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Tag", "TagA"),
                ("X-Test-Body", "page-a-new"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "page-a-new");

    let resp = ctx
        .get_with_headers(
            "/page-b",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Tag", "TagB"),
                ("X-Test-Body", "page-b-new"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "page-b-new");
}

#[tokio::test]
async fn test_lsc_cookie_to_set_cookie() {
    let ctx = LSCacheTestContext::new("lsc-cookie", BASE_CONFIG_EMIT_LS).await;

    let resp = ctx
        .get_with_headers(
            "/cookie-test",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-LSC-Cookie", "test_cookie=123"),
                ("X-Test-Body", "cookie-set"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "cookie-set");

    let resp = ctx
        .get_with_headers(
            "/cookie-test",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-LSC-Cookie", "test_cookie=123"),
                ("X-Test-Body", "cookie-hit"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert_eq!(ls_cache, "hit");
    let set_cookie = resp
        .headers()
        .get_all("Set-Cookie")
        .iter()
        .find(|h| h.to_str().unwrap().contains("test_cookie=123"));
    assert!(
        set_cookie.is_some(),
        "Set-Cookie header with test_cookie=123 not found"
    );
    assert_eq!(resp.text().await.unwrap(), "cookie-set");
}

#[tokio::test]
async fn test_lscache_s_maxage() {
    let ctx = LSCacheTestContext::new("s-maxage", BASE_CONFIG_EMIT_LS).await;

    let resp = ctx
        .get_with_headers(
            "/smaxage-test",
            &[
                ("X-Test-Cache-Control", "public,s-maxage=2"),
                ("X-Test-Upstream-Cache-Control", "max-age=300"),
                ("X-Test-Body", "s-maxage-v1"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "s-maxage-v1");

    let resp = ctx
        .get_with_headers(
            "/smaxage-test",
            &[
                ("X-Test-Cache-Control", "public,s-maxage=2"),
                ("X-Test-Upstream-Cache-Control", "max-age=300"),
                ("X-Test-Body", "s-maxage-v2"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "s-maxage-v1");

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let resp = ctx
        .get_with_headers(
            "/smaxage-test",
            &[
                ("X-Test-Cache-Control", "public,s-maxage=2"),
                ("X-Test-Upstream-Cache-Control", "max-age=300"),
                ("X-Test-Body", "s-maxage-v3"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "s-maxage-v3");
}

#[tokio::test]
async fn test_lscache_public_tag_with_private_cache() {
    let ctx = LSCacheTestContext::new("public-tag-private", BASE_CONFIG_EMIT_LS).await;

    let resp = ctx
        .get_with_headers(
            "/priv-pub-tag",
            &[
                ("X-Test-Cache-Control", "private,max-age=60"),
                ("X-Test-Tag", "public:PubTag,PrivTag"),
                ("X-Test-Body", "priv-with-pub-tag"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert_eq!(ls_cache, "miss");
    assert_eq!(resp.text().await.unwrap(), "priv-with-pub-tag");

    let resp = ctx
        .get_with_headers(
            "/priv-pub-tag",
            &[
                ("X-Test-Cache-Control", "private,max-age=60"),
                ("X-Test-Tag", "public:PubTag,PrivTag"),
                ("X-Test-Body", "priv-with-pub-tag-hit"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert_eq!(ls_cache, "hit,private");
    assert_eq!(resp.text().await.unwrap(), "priv-with-pub-tag");
}

#[tokio::test]
async fn test_lscache_bypass_uncacheable_status() {
    let ctx = LSCacheTestContext::new("bypass-status", BASE_CONFIG_EMIT_LS).await;

    let resp = ctx
        .get_with_headers(
            "/error",
            &[
                ("X-Test-Status", "500"),
                ("X-Test-Body", "error"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::INTERNAL_SERVER_ERROR);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert!(ls_cache == "bypass" || ls_cache == "miss");
}

#[tokio::test]
async fn test_lscache_shared_cache_control() {
    let ctx = LSCacheTestContext::new("shared", BASE_CONFIG_EMIT_LS).await;

    let resp = ctx
        .get_with_headers(
            "/shared-test",
            &[
                ("X-Test-Cache-Control", "shared,private,max-age=60"),
                ("X-Test-Body", "shared-content"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert!(ls_cache == "miss" || ls_cache == "hit");
    assert_eq!(resp.text().await.unwrap(), "shared-content");

    let resp = ctx
        .get_with_headers(
            "/shared-test",
            &[
                ("X-Test-Cache-Control", "shared,private,max-age=60"),
                ("X-Test-Body", "shared-content-hit"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "shared-content");
}

#[tokio::test]
async fn test_lscache_purge_private_scope() {
    let ctx = LSCacheTestContext::new("purge-private", BASE_CONFIG_EMIT_LS).await;

    let resp = ctx
        .get_with_headers(
            "/priv-purge",
            &[
                ("X-Test-Cache-Control", "private,max-age=60"),
                ("X-Test-Body", "private-v1"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "private-v1");

    let resp = ctx
        .get_with_headers(
            "/priv-purge",
            &[
                ("X-Test-Cache-Control", "private,max-age=60"),
                ("X-Test-Body", "private-v1-verify"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "private-v1");

    let resp = ctx
        .get_with_headers(
            "/priv-purge-trigger",
            &[
                ("X-Test-Purge", "private,*"),
                ("X-Test-Body", "purge-private"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let resp = ctx
        .get_with_headers(
            "/priv-purge",
            &[
                ("X-Test-Cache-Control", "private,max-age=60"),
                ("X-Test-Body", "private-v2"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "private-v2");
}

#[tokio::test]
async fn test_lscache_purge_stale_flag() {
    let ctx = LSCacheTestContext::new("purge-stale", BASE_CONFIG_EMIT_LS).await;

    let resp = ctx
        .get_with_headers(
            "/stale-test",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Tag", "StaleTag"),
                ("X-Test-Body", "stale-v1"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "stale-v1");

    let resp = ctx
        .get_with_headers(
            "/stale-purge",
            &[
                ("X-Test-Purge", "stale,tag=StaleTag"),
                ("X-Test-Body", "purge-stale"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let resp = ctx
        .get_with_headers(
            "/stale-test",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Tag", "StaleTag"),
                ("X-Test-Body", "stale-v2"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "stale-v2");
}

#[tokio::test]
async fn test_lscache_vary_value() {
    // Note: vary values aren't supported by Ferron cache implementation.
    let ctx = LSCacheTestContext::new("vary-value", BASE_CONFIG_EMIT_LS).await;

    let resp = ctx
        .get_with_headers(
            "/vary-value-test",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Vary", "value=mobile"),
                ("X-Test-Body", "desktop"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "desktop");

    let resp = ctx
        .get_with_headers(
            "/vary-value-test",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Vary", "value=mobile"),
                ("X-Test-Body", "mobile-version"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "mobile-version");

    let resp = ctx
        .get_with_headers(
            "/vary-value-test",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Vary", "value=mobile"),
                ("X-Test-Body", "mobile-hit"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "mobile-hit");
}

#[tokio::test]
async fn test_lscache_multiple_vary_cookies() {
    let ctx = LSCacheTestContext::new("vary-multi-cookie", BASE_CONFIG_EMIT_LS).await;

    let resp = ctx
        .get_with_headers(
            "/multi-vary",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Vary", "cookie=lang,cookie=theme"),
                ("X-Test-Body", "default"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "default");

    let resp = ctx
        .get_with_headers(
            "/multi-vary",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Vary", "cookie=lang,cookie=theme"),
                ("X-Test-Body", "en-dark"),
                ("Cookie", "lang=en;theme=dark"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "en-dark");

    let resp = ctx
        .get_with_headers(
            "/multi-vary",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Vary", "cookie=lang,cookie=theme"),
                ("X-Test-Body", "en-dark-hit"),
                ("Cookie", "lang=en;theme=dark"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "en-dark");

    let resp = ctx
        .get_with_headers(
            "/multi-vary",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Vary", "cookie=lang,cookie=theme"),
                ("X-Test-Body", "fr-light"),
                ("Cookie", "lang=fr;theme=light"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "fr-light");
}

#[tokio::test]
async fn test_lscache_combined_vary_cookie_and_value() {
    let ctx = LSCacheTestContext::new("vary-combined", BASE_CONFIG_EMIT_LS).await;

    let resp = ctx
        .get_with_headers(
            "/combined-vary",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Vary", "cookie=region,value=us"),
                ("X-Test-Body", "region-us"),
                ("Cookie", "region=west"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "region-us");

    let resp = ctx
        .get_with_headers(
            "/combined-vary",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Vary", "cookie=region,value=eu"),
                ("X-Test-Body", "region-eu"),
                ("Cookie", "region=east"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "region-eu");

    let resp = ctx
        .get_with_headers(
            "/combined-vary",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Vary", "cookie=region,value=us"),
                ("X-Test-Body", "region-us-hit"),
                ("Cookie", "region=west"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "region-us-hit");
}

const BASE_CONFIG_PURGE_METHOD: &str = r#"
*:80 {
  proxy "http://backend:3000"
  cache {
    emit_litespeed_headers true
    purge_method true
    purge_allowed_ips "0.0.0.0/0"
  }
}
"#;

const BASE_CONFIG_PURGE_METHOD_NO_ALLOW: &str = r#"
*:80 {
  proxy "http://backend:3000"
  cache {
    emit_litespeed_headers true
    purge_method true
  }
}
"#;

#[tokio::test]
async fn test_lscache_purge_method() {
    let ctx = LSCacheTestContext::new("purge-method", BASE_CONFIG_PURGE_METHOD).await;

    // Cache a page
    let resp = ctx
        .get_with_headers(
            "/purge-method-test",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Body", "before-purge"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "before-purge");

    // Verify it's cached
    let resp = ctx
        .get_with_headers(
            "/purge-method-test",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Body", "should-not-appear"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert_eq!(ls_cache, "hit");
    assert_eq!(resp.text().await.unwrap(), "before-purge");

    // Send PURGE request
    let resp = ctx.purge("/purge-method-test").await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // Verify cache was invalidated
    let resp = ctx
        .get_with_headers(
            "/purge-method-test",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Body", "after-purge"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert_eq!(ls_cache, "miss");
    assert_eq!(resp.text().await.unwrap(), "after-purge");
}

#[tokio::test]
async fn test_lscache_purge_method_security() {
    let ctx = LSCacheTestContext::new("purge-method-sec", BASE_CONFIG_PURGE_METHOD_NO_ALLOW).await;

    // PURGE without allowed IPs or auth should be rejected
    let resp = ctx.purge("/any-path").await;
    assert_eq!(resp.status(), reqwest::StatusCode::FORBIDDEN);
}

const BASE_CONFIG_PURGE_PROPAGATION_LOOP_PREVENTION: &str = r#"
*:80 {
  proxy "http://backend:3000"
  cache {
    emit_litespeed_headers true
    purge_method true
    purge_allowed_ips "0.0.0.0/0"
    purge_propagation {
      control_plane_url "http://0.0.0.0:1/cache/purge"
      shared_secret "test-secret"
      node_id "edge-test"
    }
  }
}
"#;

/// Test that a PURGE with `X-Purge-Source: propagation` header executes the
/// purge locally but does NOT attempt to re-propagate to the control-plane.
#[tokio::test]
async fn test_lscache_purge_propagation_loop_prevention() {
    let ctx = LSCacheTestContext::new(
        "purge-propagation-loop",
        BASE_CONFIG_PURGE_PROPAGATION_LOOP_PREVENTION,
    )
    .await;

    // Cache a page
    let resp = ctx
        .get_with_headers(
            "/propagation-loop-test",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Body", "original-content"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "original-content");

    // Verify it's cached
    let resp = ctx
        .get_with_headers(
            "/propagation-loop-test",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Body", "should-not-appear"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert_eq!(ls_cache, "hit");
    assert_eq!(resp.text().await.unwrap(), "original-content");

    // Send PURGE with X-Purge-Source: propagation header.
    // This simulates a broadcast from the control-plane. The edge should purge
    // locally but NOT re-propagate (which would cause a loop).
    let method = Method::from_bytes(b"PURGE").unwrap();
    let resp = ctx
        .client
        .request(method, format!("{}/propagation-loop-test", ctx.base_url))
        .header("X-Purge-Source", "propagation")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // Verify cache was invalidated
    let resp = ctx
        .get_with_headers(
            "/propagation-loop-test",
            &[
                ("X-Test-Cache-Control", "public,max-age=60"),
                ("X-Test-Body", "after-propagated-purge"),
            ],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ls_cache = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap();
    assert_eq!(ls_cache, "miss");
    assert_eq!(resp.text().await.unwrap(), "after-propagated-purge");
}

/// Test that an original PURGE (without X-Purge-Source header) attempts to
/// propagate to the control-plane by verifying the webhook URL is contacted.
/// Uses a mock control-plane Docker container on the same network.
#[tokio::test]
async fn test_lscache_purge_propagation_outbound_webhook() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-lscache-propagation-webhook";

    // Start backend
    let backend_image = common::build_lscache_backend_image().await.unwrap();
    let _backend = backend_image
        .with_exposed_port(ContainerPort::Tcp(3000))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(3000))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_hostname("backend")
        .start()
        .await
        .unwrap();

    // Start mock control-plane
    let control_plane_image = common::build_mock_control_plane_image().await.unwrap();
    let control_plane = control_plane_image
        .with_exposed_port(ContainerPort::Tcp(9090))
        .with_wait_for(WaitFor::message_on_stdout(
            "Mock control-plane listening on port 9090",
        ))
        .with_network(network)
        .with_hostname("control-plane")
        .start()
        .await
        .unwrap();

    let config = r#"
*:80 {
  proxy "http://backend:3000"
  cache {
    emit_litespeed_headers true
    purge_method true
    purge_allowed_ips "0.0.0.0/0"
    purge_propagation {
      control_plane_url "http://control-plane:9090/cache/purge"
      shared_secret "test-secret"
      node_id "edge-1"
    }
  }
}
"#;

    config_file
        .as_file_mut()
        .write_all(config.as_bytes())
        .unwrap();

    let ferron_image = common::build_ferron_image().await.unwrap();
    let ferron = ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_hostname("ferron")
        .with_mount(Mount::bind_mount(
            config_file.path().to_string_lossy().to_string(),
            "/etc/ferron.conf",
        ))
        .start()
        .await
        .unwrap();

    let port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let base_url = format!("http://localhost:{}", port);
    let client = reqwest::Client::new();

    let cp_port = control_plane
        .get_host_port_ipv4(ContainerPort::Tcp(9090))
        .await
        .unwrap();
    let cp_url = format!("http://localhost:{}", cp_port);

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // Cache a page
    let resp = client
        .get(format!("{}/webhook-test", base_url))
        .header("X-Test-Cache-Control", "public,max-age=60")
        .header("X-Test-Body", "cached")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // Send PURGE request (original, not propagated)
    let method = Method::from_bytes(b"PURGE").unwrap();
    let resp = client
        .request(method, format!("{}/webhook-test", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // Wait for async propagation to complete
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Query the mock control-plane's /received endpoint to verify the webhook
    let resp = client
        .get(format!("{}/received", cp_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let webhooks: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert!(
        !webhooks.is_empty(),
        "Expected at least one webhook to the control-plane"
    );
    let webhook = &webhooks[0];
    assert_eq!(webhook["path"], "/webhook-test");
    assert_eq!(webhook["origin"], "edge-1");
}
