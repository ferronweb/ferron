//! HTTP-01 ACME challenge implementation.
//!
//! The HTTP-01 challenge requires serving a specific token at a well-known URL:
//! `/.well-known/acme-challenge/<token>`

use super::Http01DataLock;

/// The well-known ACME HTTP-01 challenge path prefix.
pub const ACME_CHALLENGE_PATH_PREFIX: &str = "/.well-known/acme-challenge/";

/// Attempts to handle an HTTP-01 challenge request.
///
/// Given a request path, checks if it matches the ACME challenge path prefix,
/// and if so, looks up the token in the shared resolver locks.
///
/// Returns `Some(key_authorization)` if the token is found, `None` otherwise.
pub fn try_handle_challenge(path: &str, resolvers: &[Http01DataLock]) -> Option<String> {
    let token = path.strip_prefix(ACME_CHALLENGE_PATH_PREFIX)?;

    for lock in resolvers {
        if let Some(data) = lock.try_read().ok().and_then(|guard| guard.clone()) {
            if data.0 == token {
                return Some(data.1);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    #[test]
    fn test_try_handle_challenge_finds_token() {
        let lock: Http01DataLock = Arc::new(RwLock::new(Some((
            "mytoken".to_string(),
            "myauth".to_string(),
        ))));
        let resolvers = vec![lock];
        let result = try_handle_challenge("/.well-known/acme-challenge/mytoken", &resolvers);
        assert_eq!(result, Some("myauth".to_string()));
    }

    #[test]
    fn test_try_handle_challenge_wrong_token() {
        let lock: Http01DataLock = Arc::new(RwLock::new(Some((
            "mytoken".to_string(),
            "myauth".to_string(),
        ))));
        let resolvers = vec![lock];
        let result = try_handle_challenge("/.well-known/acme-challenge/othertoken", &resolvers);
        assert!(result.is_none());
    }

    #[test]
    fn test_try_handle_challenge_wrong_path() {
        let lock: Http01DataLock = Arc::new(RwLock::new(Some((
            "mytoken".to_string(),
            "myauth".to_string(),
        ))));
        let resolvers = vec![lock];
        let result = try_handle_challenge("/other/path", &resolvers);
        assert!(result.is_none());
    }

    #[test]
    fn test_try_handle_challenge_empty_lock() {
        let lock: Http01DataLock = Arc::new(RwLock::new(None));
        let resolvers = vec![lock];
        let result = try_handle_challenge("/.well-known/acme-challenge/mytoken", &resolvers);
        assert!(result.is_none());
    }
}
