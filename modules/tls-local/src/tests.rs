#[cfg(test)]
mod tests {
    use crate::cache::LocalTlsCache;
    use crate::provision::provision_local_cert;
    use ferron_core::config::ServerConfigurationHostFilters;
    use tempfile::tempdir;

    #[test]
    fn test_local_tls_provisioning() {
        let temp_dir = tempdir().unwrap();
        let cache = LocalTlsCache::new(temp_dir.path().to_path_buf());
        let filters = ServerConfigurationHostFilters {
            host: Some("localhost".to_string()),
            ip: None,
        };

        let result = provision_local_cert(&cache, &filters);
        assert!(result.is_ok());
        let certified_key = result.unwrap();
        assert!(!certified_key.cert.is_empty());

        // Check if CA is generated
        assert!(temp_dir.path().join("ca.crt").exists());
        assert!(temp_dir.path().join("ca.key").exists());

        // Check if leaf cert is generated
        let files: Vec<_> = std::fs::read_dir(temp_dir.path()).unwrap().collect();
        assert!(files.len() >= 4); // ca.cert, ca.key, hash.cert, hash.key
    }

    #[test]
    fn test_local_tls_cache_reuse() {
        let temp_dir = tempdir().unwrap();
        let cache = LocalTlsCache::new(temp_dir.path().to_path_buf());
        let filters = ServerConfigurationHostFilters {
            host: Some("localhost".to_string()),
            ip: None,
        };

        let result1 = provision_local_cert(&cache, &filters).unwrap();
        let result2 = provision_local_cert(&cache, &filters).unwrap();

        // Certificates should be identical if cached
        assert_eq!(result1.cert, result2.cert);
    }
}
