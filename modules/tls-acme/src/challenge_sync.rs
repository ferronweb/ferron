//! Shared challenge files for distributed HTTP-01 / TLS-ALPN-01 validation.
//!
//! When `cache` points at a shared filesystem, the node that creates the ACME
//! order publishes challenge data into the same directory so any peer can
//! answer the CA's validation request:
//! - `challenge_http_<hash(token)>` → JSON `{key_authorization, expires_at_unix}`
//! - `challenge_tls_<hash(identifier)>` → JSON
//!   `{certificate_chain_pem, private_key_pem, expires_at_unix}`
//!
//! Keys have no extension, matching the existing `account_*` /
//! `certificate_*` convention. Each challenge uses a unique key, so publishing
//! never needs read-modify-write and needs no lock. Readers validate expiry
//! and treat corrupt/expired entries as a miss.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rustls_pki_types::pem::PemObject;

use crate::cache::{get_challenge_http_key, get_challenge_tls_key, AcmeCache};

/// How long a published challenge stays valid (order lifetime + slack).
pub const CHALLENGE_TTL_SECS: u64 = 15 * 60;

/// Shared HTTP-01 challenge file payload.
#[derive(serde::Serialize, serde::Deserialize)]
struct Http01ChallengeFile {
    key_authorization: String,
    expires_at_unix: u64,
}

/// Shared TLS-ALPN-01 challenge file payload.
#[derive(serde::Serialize, serde::Deserialize)]
struct TlsAlpn01ChallengeFile {
    certificate_chain_pem: String,
    private_key_pem: String,
    expires_at_unix: u64,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Returns the shared directory for challenge files, if file caching is used.
///
/// Prefers the account cache directory (which is always the shared base dir,
/// even for on-demand configs where the certificate cache is a per-host
/// subdir), falling back to the certificate cache directory.
pub fn shared_cache_dir(config: &crate::config::AcmeConfig) -> Option<PathBuf> {
    if let Some(dir) = config.account_cache.cache_dir() {
        return Some(dir);
    }
    config.certificate_cache.cache_dir()
}

/// Same as [`shared_cache_dir`] but for an on-demand base cache path.
pub fn shared_dir_for_cache_path(cache_path: &Option<PathBuf>) -> Option<PathBuf> {
    cache_path.clone()
}

async fn write_shared_file(dir: &Path, key: &str, value: Vec<u8>) {
    AcmeCache::File(dir.to_path_buf())
        .set(key, value)
        .await
        .ok();
}

/// Publishes HTTP-01 challenge data for peers.
pub async fn publish_http01_challenge(dir: &Path, token: &str, key_authorization: &str) {
    let payload = Http01ChallengeFile {
        key_authorization: key_authorization.to_string(),
        expires_at_unix: now_unix().saturating_add(CHALLENGE_TTL_SECS),
    };
    if let Ok(bytes) = serde_json::to_vec(&payload) {
        write_shared_file(dir, &get_challenge_http_key(token), bytes).await;
    }
}

/// Loads HTTP-01 key authorization published by any node.
///
/// Returns `None` on miss, expiry, or corrupt data (treated as a miss so a
/// single bad file can never break validation).
pub async fn load_http01_challenge(dir: &Path, token: &str) -> Option<String> {
    let key = get_challenge_http_key(token);
    let bytes = tokio::fs::read(dir.join(&key)).await.ok()?;
    let payload: Http01ChallengeFile = serde_json::from_slice(&bytes).ok()?;
    if payload.expires_at_unix <= now_unix() || payload.key_authorization.is_empty() {
        let _ = tokio::fs::remove_file(dir.join(&key)).await;
        return None;
    }
    Some(payload.key_authorization)
}

/// Sync variant of [`load_http01_challenge`] for request paths running on the
/// primary (non-Tokio) runtime, where `tokio::fs` would panic.
pub fn load_http01_challenge_sync(dir: &Path, token: &str) -> Option<String> {
    let key = get_challenge_http_key(token);
    let bytes = std::fs::read(dir.join(&key)).ok()?;
    parse_http01_bytes_sync(&bytes, dir, &key)
}

fn parse_http01_bytes_sync(bytes: &[u8], dir: &Path, key: &str) -> Option<String> {
    let payload: Http01ChallengeFile = serde_json::from_slice(bytes).ok()?;
    if payload.expires_at_unix <= now_unix() || payload.key_authorization.is_empty() {
        let _ = std::fs::remove_file(dir.join(key));
        return None;
    }
    Some(payload.key_authorization)
}

/// Removes a published HTTP-01 challenge (best-effort).
pub async fn remove_http01_challenge(dir: &Path, token: &str) {
    let _ = tokio::fs::remove_file(dir.join(get_challenge_http_key(token))).await;
}

/// Publishes TLS-ALPN-01 challenge certificate material for peers.
pub async fn publish_tlsalpn01_challenge(
    dir: &Path,
    identifier: &str,
    certificate_chain_pem: &str,
    private_key_pem: &str,
) {
    let payload = TlsAlpn01ChallengeFile {
        certificate_chain_pem: certificate_chain_pem.to_string(),
        private_key_pem: private_key_pem.to_string(),
        expires_at_unix: now_unix().saturating_add(CHALLENGE_TTL_SECS),
    };
    if let Ok(bytes) = serde_json::to_vec(&payload) {
        write_shared_file(dir, &get_challenge_tls_key(identifier), bytes).await;
    }
}

/// Loads raw TLS-ALPN-01 PEM material published by any node.
pub async fn load_tlsalpn01_challenge_data(
    dir: &Path,
    identifier: &str,
) -> Option<(String, String)> {
    let key = get_challenge_tls_key(identifier);
    let bytes = tokio::fs::read(dir.join(&key)).await.ok()?;
    parse_tlsalpn01_bytes(&bytes).or_else(|| {
        let _ = std::fs::remove_file(dir.join(&key));
        None
    })
}

/// Sync variant for request paths running on the primary (non-Tokio) runtime.
pub fn load_tlsalpn01_challenge_data_sync(
    dir: &Path,
    identifier: &str,
) -> Option<(String, String)> {
    let key = get_challenge_tls_key(identifier);
    let bytes = std::fs::read(dir.join(&key)).ok()?;
    parse_tlsalpn01_bytes(&bytes).or_else(|| {
        let _ = std::fs::remove_file(dir.join(&key));
        None
    })
}

fn parse_tlsalpn01_bytes(bytes: &[u8]) -> Option<(String, String)> {
    let payload: TlsAlpn01ChallengeFile = serde_json::from_slice(bytes).ok()?;
    if payload.expires_at_unix <= now_unix()
        || payload.certificate_chain_pem.is_empty()
        || payload.private_key_pem.is_empty()
    {
        return None;
    }
    Some((payload.certificate_chain_pem, payload.private_key_pem))
}

fn certified_key_from_pems(
    chain_pem: &str,
    key_pem: &str,
) -> Option<Arc<rustls::sign::CertifiedKey>> {
    let certs = rustls_pki_types::CertificateDer::pem_slice_iter(chain_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if certs.is_empty() {
        return None;
    }
    let private_key = rustls_pki_types::PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).ok()?;
    let signing_key = rustls::crypto::aws_lc_rs::default_provider()
        .key_provider
        .load_private_key(private_key)
        .ok()?;
    Some(Arc::new(rustls::sign::CertifiedKey::new(
        certs,
        signing_key,
    )))
}

/// Loads a TLS-ALPN-01 challenge as a ready `CertifiedKey`.
///
/// Corrupt PEMs are treated as a miss (returns `None`).
pub async fn load_tlsalpn01_challenge_cert(
    dir: &Path,
    identifier: &str,
) -> Option<Arc<rustls::sign::CertifiedKey>> {
    let (chain_pem, key_pem) = load_tlsalpn01_challenge_data(dir, identifier).await?;
    certified_key_from_pems(&chain_pem, &key_pem)
}

/// Sync variant of [`load_tlsalpn01_challenge_cert`] for request paths running
/// on the primary (non-Tokio) runtime, where `tokio::fs` would panic.
pub fn load_tlsalpn01_challenge_cert_sync(
    dir: &Path,
    identifier: &str,
) -> Option<Arc<rustls::sign::CertifiedKey>> {
    let (chain_pem, key_pem) = load_tlsalpn01_challenge_data_sync(dir, identifier)?;
    certified_key_from_pems(&chain_pem, &key_pem)
}

/// Removes a published TLS-ALPN-01 challenge (best-effort).
pub async fn remove_tlsalpn01_challenge(dir: &Path, identifier: &str) {
    let _ = tokio::fs::remove_file(dir.join(get_challenge_tls_key(identifier))).await;
}

/// Removes expired challenge files. Returns the number removed.
///
/// Called opportunistically from the provisioning loop; never fails the loop.
pub async fn prune_expired_challenges(dir: &Path) -> usize {
    let mut removed = 0;
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return 0;
    };
    let now = now_unix();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let is_challenge =
            name.starts_with("challenge_http_") || name.starts_with("challenge_tls_");
        if !is_challenge {
            continue;
        }
        let Ok(bytes) = tokio::fs::read(entry.path()).await else {
            continue;
        };
        // Both payload shapes share `expires_at_unix`; try TLS shape first
        // (superset), then HTTP shape.
        let expired = serde_json::from_slice::<TlsAlpn01ChallengeFile>(&bytes)
            .map(|p| p.expires_at_unix <= now)
            .unwrap_or_else(|_| {
                serde_json::from_slice::<Http01ChallengeFile>(&bytes)
                    .map(|p| p.expires_at_unix <= now)
                    .unwrap_or(true)
            });
        if expired {
            let _ = tokio::fs::remove_file(entry.path()).await;
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ferron-acme-challenge-test-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ))
    }

    #[tokio::test]
    async fn test_http01_publish_load_remove_roundtrip() {
        let dir = unique_dir("http01");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        assert_eq!(load_http01_challenge(&dir, "tok").await, None);
        publish_http01_challenge(&dir, "tok", "tok.keyauth").await;
        assert_eq!(
            load_http01_challenge(&dir, "tok").await,
            Some("tok.keyauth".to_string())
        );
        // Different token is a miss.
        assert_eq!(load_http01_challenge(&dir, "other").await, None);
        remove_http01_challenge(&dir, "tok").await;
        assert_eq!(load_http01_challenge(&dir, "tok").await, None);
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn test_tlsalpn01_publish_load_cert_roundtrip() {
        let dir = unique_dir("tls");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        // Generate a throwaway self-signed cert with rcgen (already a dependency).
        let key_pair = rcgen::KeyPair::generate().unwrap();
        let params = rcgen::CertificateParams::new(vec!["example.com".to_string()]).unwrap();
        let cert = params.self_signed(&key_pair).unwrap();
        let chain_pem = cert.pem();
        let key_pem = key_pair.serialize_pem();
        publish_tlsalpn01_challenge(&dir, "example.com", &chain_pem, &key_pem).await;
        let loaded = load_tlsalpn01_challenge_cert(&dir, "example.com").await;
        assert!(loaded.is_some());
        assert_eq!(load_tlsalpn01_challenge_data(&dir, "other.com").await, None);
        remove_tlsalpn01_challenge(&dir, "example.com").await;
        assert!(load_tlsalpn01_challenge_cert(&dir, "example.com")
            .await
            .is_none());
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn test_corrupt_and_expired_entries_are_misses() {
        let dir = unique_dir("corrupt");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        // Corrupt file → miss, never panics.
        let bad_key = get_challenge_http_key("bad");
        tokio::fs::write(dir.join(&bad_key), b"not-json")
            .await
            .unwrap();
        assert_eq!(load_http01_challenge(&dir, "bad").await, None);
        // Expired file → miss + removed.
        let payload = Http01ChallengeFile {
            key_authorization: "x".to_string(),
            expires_at_unix: 1,
        };
        let exp_key = get_challenge_http_key("exp");
        tokio::fs::write(dir.join(&exp_key), serde_json::to_vec(&payload).unwrap())
            .await
            .unwrap();
        assert_eq!(load_http01_challenge(&dir, "exp").await, None);
        assert!(!tokio::fs::try_exists(dir.join(&exp_key)).await.unwrap());
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn test_prune_expired_ignores_non_challenge_files() {
        let dir = unique_dir("prune");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("certificate_abc"), b"{}")
            .await
            .unwrap();
        let expired = Http01ChallengeFile {
            key_authorization: "x".to_string(),
            expires_at_unix: 1,
        };
        tokio::fs::write(
            dir.join(get_challenge_http_key("old")),
            serde_json::to_vec(&expired).unwrap(),
        )
        .await
        .unwrap();
        publish_http01_challenge(&dir, "fresh", "fresh.auth").await;
        let removed = prune_expired_challenges(&dir).await;
        assert_eq!(removed, 1);
        assert!(tokio::fs::try_exists(dir.join("certificate_abc"))
            .await
            .unwrap());
        assert_eq!(
            load_http01_challenge(&dir, "fresh").await,
            Some("fresh.auth".to_string())
        );
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
