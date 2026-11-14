//! On-demand ACME configuration and background task management.
//!
//! Handles lazy certificate issuance when a TLS handshake occurs for a
//! hostname that doesn't yet have a certificate.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::cache::{get_hostname_cache_key, AcmeCache};
use crate::config::{build_rustls_client_config, AcmeConfig, AcmeOnDemandConfigData};
use crate::resolver::AcmeResolver;

/// Reads cached domains for an on-demand config.
pub async fn get_cached_domains(
    port: u16,
    sni_hostname: Option<&str>,
    cache_path: &Option<PathBuf>,
) -> Vec<String> {
    if let Some(ref pathbuf) = cache_path {
        let hostname_cache_key = get_hostname_cache_key(port, sni_hostname);
        let hostname_cache = AcmeCache::File(pathbuf.clone());
        match hostname_cache.get(&hostname_cache_key).await {
            Some(data) => serde_json::from_slice(&data).unwrap_or_default(),
            None => Vec::new(),
        }
    } else {
        Vec::new()
    }
}

/// Adds a domain to the on-demand cache.
///
/// Uses the paired `lock_hostname_*` lockfile when file caching is used so
/// concurrent nodes appending different domains to the same `hostname_*` file
/// don't lose each other's updates. Re-reads under the lock and dedupes, so
/// retries and duplicate requests can't grow the file unboundedly.
pub async fn add_domain_to_cache(
    port: u16,
    sni_hostname: Option<&str>,
    cache_path: &Option<PathBuf>,
    domain: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let Some(ref pathbuf) = cache_path else {
        return Ok(());
    };
    let hostname_cache_key = get_hostname_cache_key(port, sni_hostname);
    let lock_key = crate::cache::get_hostname_lock_key(port, sni_hostname);

    // Serialize read-modify-write across nodes sharing the cache.
    let guard = match crate::provision::dist_lock::acquire_with_timeout(
        pathbuf,
        &lock_key,
        domain,
        "",
        std::time::Duration::from_secs(3),
    )
    .await
    {
        Ok((guard_opt, _)) => guard_opt,
        Err(_) => None, // Fail open to best-effort append below.
    };

    let hostname_cache = AcmeCache::File(pathbuf.clone());
    let mut cached_domains: Vec<String> = match hostname_cache.get(&hostname_cache_key).await {
        Some(data) => serde_json::from_slice(&data).unwrap_or_default(),
        None => Vec::new(),
    };
    if !cached_domains.iter().any(|d| d == domain) {
        cached_domains.push(domain.to_string());
        let data = serde_json::to_vec(&cached_domains)?;
        hostname_cache.set(&hostname_cache_key, data).await?;
    }

    if let Some(g) = guard {
        g.release().await;
    }
    Ok(())
}

/// Resolves cache paths for on-demand config conversion.
fn resolve_cache_paths(
    cache_path: &Option<PathBuf>,
    port: u16,
    sni_hostname: &str,
) -> (Option<PathBuf>, Option<PathBuf>) {
    if let Some(mut pathbuf) = cache_path.clone() {
        let base_pathbuf = pathbuf.clone();
        let append_hash = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            xxhash_rust::xxh3::xxh3_128(format!("{port}-{sni_hostname}").as_bytes()).to_be_bytes(),
        );
        pathbuf.push(append_hash);
        (Some(base_pathbuf), Some(pathbuf))
    } else {
        (None, None)
    }
}

/// Converts an `AcmeOnDemandConfigData` into an `AcmeConfig` for a specific hostname.
pub async fn convert_on_demand_config(
    data: &AcmeOnDemandConfigData,
    sni_hostname: String,
    memory_acme_account_cache_data: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    sni_resolver_lock: &crate::config::SniResolverLock,
    tls_alpn_01_resolver_lock: &Arc<RwLock<Vec<crate::challenge::TlsAlpn01DataLock>>>,
    http_01_resolver_lock: &Arc<RwLock<Vec<crate::challenge::Http01DataLock>>>,
) -> AcmeConfig {
    let (account_cache_path, cert_cache_path) =
        resolve_cache_paths(&data.cache_path, data.port, &sni_hostname);

    let certified_key_lock = Arc::new(RwLock::new(None));
    let tls_alpn_01_data_lock = Arc::new(RwLock::new(None));
    let http_01_data_lock = Arc::new(RwLock::new(None));

    // Register the resolver
    sni_resolver_lock.write().await.insert(
        sni_hostname.clone(),
        Arc::new(AcmeResolver::new(
            crate::resolver::AcmeResolverInner::Eager(certified_key_lock.clone()),
            None,
        )),
    );

    // Add challenge data locks to shared resolver lists
    match data.challenge_type {
        instant_acme::ChallengeType::TlsAlpn01 => {
            tls_alpn_01_resolver_lock
                .write()
                .await
                .push(tls_alpn_01_data_lock.clone());
        }
        instant_acme::ChallengeType::Http01 => {
            http_01_resolver_lock
                .write()
                .await
                .push(http_01_data_lock.clone());
        }
        _ => {}
    }

    AcmeConfig {
        rustls_client_config: data.rustls_client_config.clone(),
        domains: vec![sni_hostname],
        challenge_type: data.challenge_type.clone(),
        contact: data.contact.clone(),
        directory: data.directory.clone(),
        eab_key: data.eab_key.clone(),
        profile: data.profile.clone(),
        provider_list: data.provider_list.clone(),
        account_cache: if let Some(ref account_cache_path) = account_cache_path {
            AcmeCache::File(account_cache_path.clone())
        } else {
            AcmeCache::Memory(memory_acme_account_cache_data.clone())
        },
        certificate_cache: if let Some(ref cert_cache_path) = cert_cache_path {
            AcmeCache::File(cert_cache_path.clone())
        } else {
            AcmeCache::Memory(Arc::new(RwLock::new(HashMap::new())))
        },
        certified_key_lock,
        tls_alpn_01_data_lock,
        http_01_data_lock,
        dns_client: data.dns_client.clone(),
        account: None,
        save_paths: None,
        post_obtain_command: None,
        error_message: data.error_message.clone(),
        cert_algo: data.cert_algo,
        cert_key_size: data.cert_key_size,
        cert_algo_default: data.cert_algo_default,
    }
}

/// Message sent to the on-demand channel requesting a certificate.
pub type OnDemandRequest = (String, u16); // (sni_hostname, port)

/// Simple hostname pattern matching.
/// Supports exact match and wildcard patterns (e.g. `*.example.com`).
pub fn match_hostname(pattern: &str, hostname: &str) -> bool {
    if pattern == hostname {
        return true;
    }

    if let Some(suffix) = pattern.strip_prefix("*.") {
        if let Some((_, rest)) = hostname.split_once('.') {
            return rest == suffix;
        }
    }

    false
}

/// Returns the indices of the on-demand configs that apply to `hostname` on
/// `port`, in the order they should be tried.
///
/// Each item is a config's SNI pattern and port. A `None` pattern comes from a
/// host block without a hostname. That catch-all block applies only when no
/// host block with a hostname or wildcard matches, so it cannot approve a
/// hostname that a more specific block's approval endpoint denied.
pub fn matching_config_indices<'a>(
    configs: impl IntoIterator<Item = (Option<&'a str>, u16)>,
    hostname: &str,
    port: u16,
) -> Vec<usize> {
    let mut specific = Vec::new();
    let mut catch_all = Vec::new();
    for (index, (pattern, config_port)) in configs.into_iter().enumerate() {
        if config_port != port {
            continue;
        }
        match pattern {
            Some(pattern) if match_hostname(pattern, hostname) => specific.push(index),
            Some(_) => {}
            None => catch_all.push(index),
        }
    }
    if specific.is_empty() {
        catch_all
    } else {
        specific
    }
}

/// Maximum number of queued on-demand certificate requests. A handshake that
/// finds the queue full is dropped; the next handshake for that hostname asks
/// again.
pub const REQUEST_QUEUE_CAPACITY: usize = 1024;

/// How long Ferron waits for the approval endpoint before treating the request
/// as failed. Requests are processed one at a time, so a stalled endpoint would
/// otherwise stop all on-demand issuance.
pub const ASK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// How long a hostname that was not approved waits before Ferron asks again.
pub const DENIED_RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(60);

/// Maximum number of denied hostnames remembered at once.
pub const DENIED_CAPACITY: usize = 10_000;

/// Remembers hostnames whose issuance was not approved, so that repeated
/// handshakes don't flood the approval endpoint, while still letting a
/// hostname that is approved later get a certificate without a reload.
///
/// The map is bounded because, with a catch-all host block, any client can
/// send arbitrary SNI hostnames.
pub struct DeniedHostnames {
    entries: HashMap<(String, u16), std::time::Instant>,
    retry_after: std::time::Duration,
    capacity: usize,
}

impl DeniedHostnames {
    pub fn new(retry_after: std::time::Duration, capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            retry_after,
            capacity,
        }
    }

    /// Returns `true` while a recent denial for this hostname is still in effect.
    pub fn is_denied(&mut self, hostname: &str, port: u16, now: std::time::Instant) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        let key = (hostname.to_string(), port);
        match self.entries.get(&key) {
            Some(denied_at) if now.duration_since(*denied_at) < self.retry_after => true,
            Some(_) => {
                self.entries.remove(&key);
                false
            }
            None => false,
        }
    }

    /// Records a denial. When the map is full, expired entries are dropped
    /// first, then the oldest entry.
    pub fn record(&mut self, hostname: &str, port: u16, now: std::time::Instant) {
        if self.entries.len() >= self.capacity {
            let retry_after = self.retry_after;
            self.entries
                .retain(|_, denied_at| now.duration_since(*denied_at) < retry_after);
        }
        if self.entries.len() >= self.capacity {
            if let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, denied_at)| **denied_at)
                .map(|(key, _)| key.clone())
            {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert((hostname.to_string(), port), now);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

pub async fn check_ask_endpoint(
    domain: &str,
    on_demand_ask: Option<&str>,
    auth: Option<&str>,
    no_verification: bool,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let Some(on_demand_ask) = on_demand_ask else {
        // No on-demand ask endpoint configured, allow issuance
        return Ok(true);
    };
    let on_demand_tls_ask_endpoint = on_demand_ask.parse::<hyper::Uri>()?;
    let mut url_parts = on_demand_tls_ask_endpoint.into_parts();
    let path_and_query_str = if let Some(path_and_query) = url_parts.path_and_query {
        let query = path_and_query.query();
        let query = if let Some(query) = query {
            format!("{}&domain={}", query, urlencoding::encode(domain))
        } else {
            format!("domain={}", urlencoding::encode(domain))
        };
        format!("{}?{}", path_and_query.path(), query)
    } else {
        format!("/?domain={}", urlencoding::encode(domain))
    };

    url_parts.path_and_query = Some(match path_and_query_str.parse() {
        Ok(parsed) => parsed,
        Err(err) => Err(anyhow::anyhow!(
            "Error while formatting the URL for on-demand TLS request: {err}"
        ))?,
    });

    let endpoint_url = match hyper::Uri::from_parts(url_parts) {
        Ok(parsed) => parsed,
        Err(err) => Err(anyhow::anyhow!(
            "Error while formatting the URL for on-demand TLS request: {err}"
        ))?,
    };

    let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
        .build::<_, http_body_util::Empty<hyper::body::Bytes>>(
        hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(build_rustls_client_config(no_verification)?)
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .build(),
    );
    let mut request_builder = hyper::Request::builder()
        .method(hyper::Method::GET)
        .uri(endpoint_url);
    if let Some(auth) = auth {
        request_builder = request_builder.header("Authorization", auth);
    }
    let request = request_builder.body(http_body_util::Empty::<hyper::body::Bytes>::new())?;
    let response = tokio::time::timeout(ASK_TIMEOUT, client.request(request))
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "The on-demand TLS approval endpoint did not respond within {} seconds",
                ASK_TIMEOUT.as_secs()
            )
        })??;

    Ok(response.status().is_success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_hostname_exact() {
        assert!(match_hostname("example.com", "example.com"));
        assert!(!match_hostname("example.com", "www.example.com"));
    }

    #[test]
    fn test_matching_config_indices_uses_catch_all_only_without_other_matches() {
        let configs = [
            (None, 443),
            (Some("*.example.com"), 443),
            (Some("api.example.com"), 443),
            (Some("other.test"), 443),
            (None, 8443),
        ];
        assert_eq!(
            matching_config_indices(configs, "api.example.com", 443),
            vec![1, 2]
        );
        assert_eq!(
            matching_config_indices(configs, "tenant.example.net", 443),
            vec![0]
        );
        assert_eq!(
            matching_config_indices(configs, "tenant.example.net", 8443),
            vec![4]
        );
        assert!(matching_config_indices(configs, "tenant.example.net", 80).is_empty());
    }

    #[test]
    fn test_matching_config_indices_without_catch_all() {
        let configs = [(Some("*.example.com"), 443)];
        assert!(matching_config_indices(configs, "tenant.example.net", 443).is_empty());
    }

    /// Serves one HTTP response on a local port and returns the ask URL.
    fn serve_once(status_line: &'static str) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0; 1024];
            let _ = stream.read(&mut buffer);
            let _ = write!(stream, "{status_line}\r\nContent-Length: 0\r\n\r\n");
        });
        format!("http://{address}/check")
    }

    #[tokio::test]
    async fn test_check_ask_endpoint_approves_on_success() {
        let url = serve_once("HTTP/1.1 204 No Content");
        assert!(check_ask_endpoint("a.test", Some(&url), None, false)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn test_check_ask_endpoint_denies_on_failure_status() {
        let url = serve_once("HTTP/1.1 403 Forbidden");
        assert!(!check_ask_endpoint("a.test", Some(&url), None, false)
            .await
            .unwrap());
    }

    #[tokio::test(start_paused = true)]
    async fn test_check_ask_endpoint_times_out() {
        // The listener accepts TCP connections in the kernel backlog but never
        // answers, like a stalled approval endpoint.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/check", listener.local_addr().unwrap());
        let error = check_ask_endpoint("a.test", Some(&url), None, false)
            .await
            .expect_err("a stalled endpoint must not approve");
        assert!(error.to_string().contains("did not respond"));
        drop(listener);
    }

    #[test]
    fn test_denied_hostnames_retry_after_expiry() {
        let start = std::time::Instant::now();
        let mut denied = DeniedHostnames::new(std::time::Duration::from_secs(60), 10);
        assert!(!denied.is_denied("a.test", 443, start));
        denied.record("a.test", 443, start);
        assert!(denied.is_denied("a.test", 443, start + std::time::Duration::from_secs(59)));
        assert!(!denied.is_denied("a.test", 8443, start));
        assert!(!denied.is_denied("a.test", 443, start + std::time::Duration::from_secs(60)));
        assert!(denied.is_empty());
    }

    #[test]
    fn test_denied_hostnames_is_bounded() {
        let start = std::time::Instant::now();
        let mut denied = DeniedHostnames::new(std::time::Duration::from_secs(60), 3);
        for (offset, name) in ["a.test", "b.test", "c.test", "d.test"].iter().enumerate() {
            denied.record(
                name,
                443,
                start + std::time::Duration::from_secs(offset as u64),
            );
        }
        assert_eq!(denied.len(), 3);
        let now = start + std::time::Duration::from_secs(4);
        assert!(
            !denied.is_denied("a.test", 443, now),
            "oldest entry is evicted"
        );
        assert!(denied.is_denied("d.test", 443, now));

        // Expired entries are dropped before the oldest live entry.
        let later = start + std::time::Duration::from_secs(62);
        denied.record("e.test", 443, later);
        assert!(denied.is_denied("d.test", 443, later));
        assert!(denied.is_denied("e.test", 443, later));
    }

    #[test]
    fn test_match_hostname_wildcard() {
        assert!(match_hostname("*.example.com", "www.example.com"));
        assert!(match_hostname("*.example.com", "api.example.com"));
        assert!(!match_hostname("*.example.com", "example.com"));
        assert!(!match_hostname("*.example.com", "www.sub.example.com"));
    }
}
