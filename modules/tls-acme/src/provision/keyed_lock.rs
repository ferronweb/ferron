//! Process-wide async locks keyed by name.
//!
//! Certificates are provisioned concurrently, so steps that touch state shared
//! between orders (an ACME account, the challenge for a name) take the lock for
//! that state's key first. The locks work within one process only.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

/// Serializes loading, creating, and recreating each cached ACME account.
pub(crate) static ACCOUNT_LOCKS: LazyLock<KeyedLocks> = LazyLock::new(KeyedLocks::default);
/// Serializes orders that share a name, whose challenges would collide.
pub(crate) static NAME_LOCKS: LazyLock<KeyedLocks> = LazyLock::new(KeyedLocks::default);

/// Async mutexes keyed by name, created on first use.
#[derive(Default)]
pub(crate) struct KeyedLocks(parking_lot::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>);

impl KeyedLocks {
    /// Waits for and takes the lock for `key`.
    pub(crate) async fn lock(&self, key: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = self.0.lock().entry(key.to_string()).or_default().clone();
        lock.lock_owned().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_same_key_waits_other_keys_do_not() {
        let locks = KeyedLocks::default();
        let held = locks.lock("a").await;
        // Another key is free while "a" is held.
        let other = tokio::time::timeout(Duration::from_millis(100), locks.lock("b")).await;
        assert!(other.is_ok());
        // The same key waits until the holder lets go.
        let same = tokio::time::timeout(Duration::from_millis(100), locks.lock("a")).await;
        assert!(same.is_err());
        drop(held);
        let same = tokio::time::timeout(Duration::from_millis(100), locks.lock("a")).await;
        assert!(same.is_ok());
    }
}
