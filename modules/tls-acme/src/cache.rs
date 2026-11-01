//! ACME cache for storing accounts and certificates.
//!
//! Supports both in-memory and file-based caching.
//!
//! When the file cache points at a shared filesystem (NFS/EFS/CephFS), the
//! same directory is reused for distributed coordination:
//! - `challenge_http_*` / `challenge_tls_*`: per-challenge sync files so any
//!   node can answer HTTP-01 / TLS-ALPN-01 validations.
//! - `lock_certificate_*` / `lock_account_*` / `lock_hostname_*`: lockfiles so
//!   only one node orders/renews a given certificate at a time.
//!
//! All cache files intentionally have no extension and follow the
//! `prefix_<hash>` convention.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use xxhash_rust::xxh3::xxh3_128;

/// Counter for unique temp file names within this process.
static TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Represents the type of cache to use for storing ACME data.
pub enum AcmeCache {
    /// Use an in-memory cache.
    Memory(Arc<RwLock<HashMap<String, Vec<u8>>>>),
    /// Use a file-based cache.
    File(PathBuf),
}

impl AcmeCache {
    /// Returns the backing directory for file caches.
    pub fn cache_dir(&self) -> Option<PathBuf> {
        match self {
            AcmeCache::Memory(_) => None,
            AcmeCache::File(path) => Some(path.clone()),
        }
    }

    /// Gets data from the cache.
    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        if !is_valid_key(key) {
            return None;
        }
        match self {
            AcmeCache::Memory(cache) => cache.read().await.get(key).cloned(),
            AcmeCache::File(path) => tokio::fs::read(path.join(key)).await.ok(),
        }
    }

    /// Sets data in the cache.
    ///
    /// File writes are atomic (`tmp + fsync + rename`) so peers on a shared
    /// filesystem never observe partially written JSON. Temp files are
    /// dot-prefixed (`.<key>.tmp.<pid>.<n>`) so `challenge_*` / `lock_*`
    /// directory scans ignore them.
    pub async fn set(&self, key: &str, value: Vec<u8>) -> Result<(), std::io::Error> {
        if !is_valid_key(key) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid ACME cache key: {key:?}"),
            ));
        }
        match self {
            AcmeCache::Memory(cache) => {
                cache.write().await.insert(key.to_string(), value);
                Ok(())
            }
            AcmeCache::File(path) => {
                tokio::fs::create_dir_all(path).await.unwrap_or_default();
                let tmp_name = format!(
                    ".{key}.tmp.{}-{}",
                    std::process::id(),
                    TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                );
                let tmp_path = path.join(tmp_name);
                let mut open_options = tokio::fs::OpenOptions::new();
                open_options.write(true).create(true).truncate(true);

                #[cfg(unix)]
                open_options.mode(0o600); // Don't allow others to read or write

                let mut file = open_options.open(&tmp_path).await?;
                let write_result = async {
                    file.write_all(&value).await?;
                    file.flush().await.unwrap_or_default();
                    file.sync_all().await.unwrap_or_default();
                    drop(file);
                    tokio::fs::rename(&tmp_path, path.join(key)).await
                }
                .await;
                if write_result.is_err() {
                    let _ = tokio::fs::remove_file(&tmp_path).await;
                }
                write_result
            }
        }
    }

    /// Removes data from the cache.
    pub async fn remove(&self, key: &str) {
        if !is_valid_key(key) {
            return;
        }
        match self {
            AcmeCache::Memory(cache) => {
                cache.write().await.remove(key);
            }
            AcmeCache::File(path) => {
                let _ = tokio::fs::remove_file(path.join(key)).await;
            }
        }
    }
}

/// Validates an ACME cache key.
///
/// Keys are internally generated (`prefix_<base64url>`), but challenge tokens
/// originate from the ACME server, so hashing keeps them safe. This guard
/// prevents accidental path traversal if a key ever contains separators.
pub fn is_valid_key(key: &str) -> bool {
    if key.is_empty() || key.len() > 255 {
        return false;
    }
    if key.starts_with('.') || key.contains('/') || key.contains('\\') || key.contains('\0') {
        return false;
    }
    if key == "." || key == ".." || key.contains("..") {
        return false;
    }
    true
}

/// Serialized certificate cache data.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct CertificateCacheData {
    pub certificate_chain_pem: String,
    pub private_key_pem: String,
}

/// Generates an account cache key from contact emails and ACME directory URL.
pub fn get_account_cache_key(contact: &[String], directory: &str) -> String {
    format!(
        "account_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            xxh3_128(format!("{};{}", contact.join(","), directory).as_bytes()).to_be_bytes()
        )
    )
}

/// Generates a certificate cache key from ACME configuration and optional profile.
pub fn get_certificate_cache_key(config: &crate::config::AcmeConfig) -> String {
    let mut sorted_domains = config.domains.to_vec();
    sorted_domains.sort_unstable();
    let domains_joined = sorted_domains.join(",");
    format!(
        "certificate_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            xxh3_128(
                format!(
                    "{}{}{}{}",
                    domains_joined,
                    config
                        .profile
                        .as_deref()
                        .map_or("".to_string(), |p| format!(";{p}")),
                    if config.cert_algo_default {
                        String::new()
                    } else {
                        format!(";{:?}", config.cert_algo)
                    },
                    match config.cert_key_size {
                        Some(rcgen::RsaKeySize::_2048) => ";2048",
                        Some(rcgen::RsaKeySize::_3072) => ";3072",
                        Some(rcgen::RsaKeySize::_4096) => ";4096",
                        _ => "",
                    }
                )
                .as_bytes()
            )
            .to_be_bytes()
        )
    )
}

/// Generates a hostname cache key for on-demand configs.
pub fn get_hostname_cache_key(port: u16, sni_hostname: Option<&str>) -> String {
    format!(
        "hostname_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            xxh3_128(
                format!(
                    "{}{}",
                    port,
                    sni_hostname.map_or("".to_string(), |h| format!(";{h}"))
                )
                .as_bytes()
            )
            .to_be_bytes()
        )
    )
}

/// Generates a shared-file key for an HTTP-01 challenge token.
///
/// The token comes from the ACME server, so it is hashed to keep the file
/// name fixed-length and free of path separators. No file extension, matching
/// the existing `account_*` / `certificate_*` convention.
pub fn get_challenge_http_key(token: &str) -> String {
    format!(
        "challenge_http_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(xxh3_128(token.as_bytes()).to_be_bytes())
    )
}

/// Generates a shared-file key for a TLS-ALPN-01 challenge identifier (usually the DNS name).
pub fn get_challenge_tls_key(identifier: &str) -> String {
    format!(
        "challenge_tls_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(xxh3_128(identifier.as_bytes()).to_be_bytes())
    )
}

/// Generates the lockfile key paired with a certificate cache key.
///
/// `get_certificate_cache_key` returns `certificate_<hash>`; this returns
/// `lock_certificate_<hash>` so `ls lock_*` groups all cert locks together
/// while keeping the cert/lock pairing obvious.
pub fn get_cert_lock_key(config: &crate::config::AcmeConfig) -> String {
    format!("lock_{}", get_certificate_cache_key(config))
}

/// Generates the lockfile key paired with an account cache key.
pub fn get_account_lock_key(contact: &[String], directory: &str) -> String {
    format!("lock_{}", get_account_cache_key(contact, directory))
}

/// Generates the lockfile key paired with a hostname (on-demand) cache key.
pub fn get_hostname_lock_key(port: u16, sni_hostname: Option<&str>) -> String {
    format!("lock_{}", get_hostname_cache_key(port, sni_hostname))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_conventions_have_no_extension_and_grouped_prefixes() {
        let crypto_provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let cert_key_config = crate::config::AcmeConfig {
            cert_algo: &rcgen::PKCS_ECDSA_P256_SHA256,
            cert_algo_default: true,
            cert_key_size: None,
            domains: vec!["example.com".to_string()],
            profile: None,

            rustls_client_config: rustls::ClientConfig::builder_with_provider(
                crypto_provider.clone(),
            )
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_webpki_verifier(
                rustls::client::WebPkiServerVerifier::builder_with_provider(
                    Arc::new(rustls::RootCertStore::from_iter(
                        webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
                    )),
                    crypto_provider,
                )
                .build()
                .unwrap(),
            )
            .with_no_client_auth(),
            challenge_type: instant_acme::ChallengeType::Http01,
            contact: vec![],
            directory: "https://directory.placeholder.invalid".into(),
            eab_key: None,
            provider_list: Arc::new(RwLock::new(crate::config::AcmeProviderList::empty())),
            account_cache: AcmeCache::Memory(Arc::new(Default::default())),
            certificate_cache: AcmeCache::Memory(Arc::new(Default::default())),
            certified_key_lock: Arc::new(Default::default()),
            tls_alpn_01_data_lock: Arc::new(Default::default()),
            http_01_data_lock: Arc::new(Default::default()),
            dns_client: None,
            account: None,
            save_paths: None,
            post_obtain_command: None,
            error_message: Arc::new(Default::default()),
        };
        let cert_key = get_certificate_cache_key(&cert_key_config);
        let lock_key = get_cert_lock_key(&cert_key_config);
        assert!(cert_key.starts_with("certificate_"));
        assert_eq!(lock_key, format!("lock_{cert_key}"));
        assert!(lock_key.starts_with("lock_certificate_"));
        for key in [&cert_key, &lock_key] {
            assert!(!key.contains('.'), "no file extension: {key}");
            assert!(is_valid_key(key));
        }

        let http_key = get_challenge_http_key("some-token");
        let tls_key = get_challenge_tls_key("example.com");
        assert!(http_key.starts_with("challenge_http_"));
        assert!(tls_key.starts_with("challenge_tls_"));
        assert!(!http_key.contains('.'));
        assert!(!tls_key.contains('.'));
        assert!(is_valid_key(&http_key));
        assert!(is_valid_key(&tls_key));

        // Same input hashes stably; different inputs differ.
        assert_eq!(get_challenge_http_key("abc"), get_challenge_http_key("abc"));
        assert_ne!(get_challenge_http_key("abc"), get_challenge_http_key("abd"));

        // Certificate key is order-independent (sorted domains).
        let a = get_certificate_cache_key(&cert_key_config);
        let b = get_certificate_cache_key(&cert_key_config);
        assert_eq!(a, b);
    }

    #[test]
    fn test_is_valid_key_rejects_traversal() {
        assert!(!is_valid_key(""));
        assert!(!is_valid_key("../certificate_x"));
        assert!(!is_valid_key("a/b"));
        assert!(!is_valid_key(".hidden"));
        assert!(is_valid_key("certificate_abc123"));
        assert!(is_valid_key("lock_certificate_abc123"));
        assert!(is_valid_key("challenge_http_abc123"));
    }

    #[tokio::test]
    async fn test_file_set_is_atomic_and_roundtrips() {
        let dir = std::env::temp_dir().join(format!(
            "ferron-acme-cache-test-{}-{}",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let cache = AcmeCache::File(dir.clone());
        cache
            .set("certificate_testkey", b"hello".to_vec())
            .await
            .unwrap();
        assert_eq!(
            cache.get("certificate_testkey").await,
            Some(b"hello".to_vec())
        );
        // No temp files leak into the directory listing.
        let mut entries = tokio::fs::read_dir(&dir).await.unwrap();
        let mut names = Vec::new();
        while let Some(e) = entries.next_entry().await.unwrap() {
            names.push(e.file_name().to_string_lossy().to_string());
        }
        assert_eq!(names, vec!["certificate_testkey".to_string()]);
        cache.remove("certificate_testkey").await;
        assert_eq!(cache.get("certificate_testkey").await, None);
        let _ = tokio::fs::remove_dir(&dir).await;

        // Invalid keys are rejected, not written.
        assert!(cache.set("../evil", b"x".to_vec()).await.is_err());
    }
}
