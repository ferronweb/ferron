//! ACME cache for storing accounts and certificates.
//!
//! Supports both in-memory and file-based caching.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use xxhash_rust::xxh3::xxh3_128;

/// Represents the type of cache to use for storing ACME data.
pub enum AcmeCache {
    /// Use an in-memory cache.
    Memory(Arc<RwLock<HashMap<String, Vec<u8>>>>),
    /// Use a file-based cache.
    File(PathBuf),
}

impl AcmeCache {
    /// Gets data from the cache.
    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        match self {
            AcmeCache::Memory(cache) => cache.read().await.get(key).cloned(),
            AcmeCache::File(path) => tokio::fs::read(path.join(key)).await.ok(),
        }
    }

    /// Sets data in the cache.
    pub async fn set(&self, key: &str, value: Vec<u8>) -> Result<(), std::io::Error> {
        match self {
            AcmeCache::Memory(cache) => {
                cache.write().await.insert(key.to_string(), value);
                Ok(())
            }
            AcmeCache::File(path) => {
                tokio::fs::create_dir_all(path).await.unwrap_or_default();
                let mut open_options = tokio::fs::OpenOptions::new();
                open_options.write(true).create(true).truncate(true);

                #[cfg(unix)]
                open_options.mode(0o600); // Don't allow others to read or write

                let mut file = open_options.open(path.join(key)).await?;
                file.write_all(&value).await?;
                file.flush().await.unwrap_or_default();

                Ok(())
            }
        }
    }

    /// Removes data from the cache.
    pub async fn remove(&self, key: &str) {
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

/// Generates a certificate cache key from sorted domains and optional profile.
pub fn get_certificate_cache_key(domains: &[String], profile: Option<&str>) -> String {
    let mut sorted_domains = domains.to_vec();
    sorted_domains.sort_unstable();
    let domains_joined = sorted_domains.join(",");
    format!(
        "certificate_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            xxh3_128(
                format!(
                    "{}{}",
                    domains_joined,
                    profile.map_or("".to_string(), |p| format!(";{p}"))
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
