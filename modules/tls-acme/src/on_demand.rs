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
pub async fn add_domain_to_cache(
    port: u16,
    sni_hostname: Option<&str>,
    cache_path: &Option<PathBuf>,
    domain: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(ref pathbuf) = cache_path {
        let hostname_cache_key = get_hostname_cache_key(port, sni_hostname);
        let hostname_cache = AcmeCache::File(pathbuf.clone());
        let mut cached_domains = get_cached_domains(port, sni_hostname, cache_path).await;
        cached_domains.push(domain.to_string());
        let data = serde_json::to_vec(&cached_domains)?;
        hostname_cache.set(&hostname_cache_key, data).await?;
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

pub async fn check_ask_endpoint(
    domain: &str,
    on_demand_ask: Option<&str>,
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
    let request = hyper::Request::builder()
        .method(hyper::Method::GET)
        .uri(endpoint_url)
        .body(http_body_util::Empty::<hyper::body::Bytes>::new())?;
    let response = client.request(request).await?;

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
    fn test_match_hostname_wildcard() {
        assert!(match_hostname("*.example.com", "www.example.com"));
        assert!(match_hostname("*.example.com", "api.example.com"));
        assert!(!match_hostname("*.example.com", "example.com"));
        assert!(!match_hostname("*.example.com", "www.sub.example.com"));
    }
}
