use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
};
use ferron_tls::TcpTlsContext;

/// Result of automatic TLS provider selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TlsAutoSelection {
    /// Use the "local" provider.
    Local,
    /// Use the "acme" provider.
    Acme,
    /// No automatic selection possible.
    None,
}

/// Select the appropriate automatic TLS provider for a host.
pub fn select_auto_tls_provider(
    registry: &ferron_core::registry::Registry,
    host: Option<&str>,
    ip: Option<&str>,
) -> TlsAutoSelection {
    let local_available = registry
        .get_provider_registry::<TcpTlsContext>()
        .is_some_and(|r| r.get("local").is_some());

    let acme_available = registry
        .get_provider_registry::<TcpTlsContext>()
        .is_some_and(|r| r.get("acme").is_some());

    let is_localhost = match (host, ip) {
        (Some(h), _) if is_loopback_host(h) => true,
        (_, Some(i)) if is_loopback_ip(i) => true,
        _ => false,
    };

    if is_localhost {
        if local_available {
            TlsAutoSelection::Local
        } else {
            TlsAutoSelection::None
        }
    } else if (host.is_some() || ip.is_some()) && acme_available {
        TlsAutoSelection::Acme
    } else {
        TlsAutoSelection::None
    }
}

/// Returns true if the hostname is a loopback / development name.
pub fn is_loopback_host(hostname: &str) -> bool {
    matches!(hostname, "localhost")
}

/// Returns true if the IP is a loopback address.
pub fn is_loopback_ip(ip: &str) -> bool {
    matches!(ip, "127.0.0.1" | "::1")
}

/// Create a synthetic `tls` directive entry for a specific provider.
pub fn create_synthetic_tls_directive(provider_name: &str) -> ServerConfigurationDirectiveEntry {
    let synthetic_children = ServerConfigurationBlock {
        directives: Arc::new(HashMap::from([(
            "provider".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(
                    provider_name.to_string(),
                    None,
                )],
                ..Default::default()
            }],
        )])),
        matchers: HashMap::new(),
        span: None,
    };

    ServerConfigurationDirectiveEntry {
        args: vec![ServerConfigurationValue::Boolean(true, None)],
        children: Some(synthetic_children),
        span: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::registry::Registry;

    fn create_test_registry(local_available: bool, acme_available: bool) -> Registry {
        let registry = Registry::new();

        if local_available {
            registry.register_provider::<TcpTlsContext<'static>, _>(|| {
                use ferron_core::providers::Provider;
                struct MockLocalProvider;
                impl Provider<TcpTlsContext<'static>> for MockLocalProvider {
                    fn name(&self) -> &str {
                        "local"
                    }
                    fn execute(
                        &self,
                        _ctx: &mut TcpTlsContext<'static>,
                    ) -> Result<(), Box<dyn std::error::Error>> {
                        Ok(())
                    }
                }
                Arc::new(MockLocalProvider)
            });
        }

        if acme_available {
            registry.register_provider::<TcpTlsContext<'static>, _>(|| {
                use ferron_core::providers::Provider;
                struct MockAcmeProvider;
                impl Provider<TcpTlsContext<'static>> for MockAcmeProvider {
                    fn name(&self) -> &str {
                        "acme"
                    }
                    fn execute(
                        &self,
                        _ctx: &mut TcpTlsContext<'static>,
                    ) -> Result<(), Box<dyn std::error::Error>> {
                        Ok(())
                    }
                }
                Arc::new(MockAcmeProvider)
            });
        }

        registry
    }

    #[test]
    fn test_localhost_selects_local_when_available() {
        let registry = create_test_registry(true, false);

        // Test hostname localhost
        let selection = select_auto_tls_provider(&registry, Some("localhost"), None);
        assert_eq!(selection, TlsAutoSelection::Local);

        // Test IP 127.0.0.1
        let selection = select_auto_tls_provider(&registry, None, Some("127.0.0.1"));
        assert_eq!(selection, TlsAutoSelection::Local);

        // Test IP ::1
        let selection = select_auto_tls_provider(&registry, None, Some("::1"));
        assert_eq!(selection, TlsAutoSelection::Local);
    }

    #[test]
    fn test_localhost_returns_none_when_local_not_available() {
        let registry = create_test_registry(false, true);

        let selection = select_auto_tls_provider(&registry, Some("localhost"), None);
        assert_eq!(selection, TlsAutoSelection::None);
    }

    #[test]
    fn test_non_localhost_selects_acme_when_available() {
        let registry = create_test_registry(false, true);

        let selection = select_auto_tls_provider(&registry, Some("example.com"), None);
        assert_eq!(selection, TlsAutoSelection::Acme);

        let selection = select_auto_tls_provider(&registry, None, Some("192.168.1.1"));
        assert_eq!(selection, TlsAutoSelection::Acme);
    }

    #[test]
    fn test_non_localhost_returns_none_when_no_providers_available() {
        let registry = create_test_registry(false, false);

        let selection = select_auto_tls_provider(&registry, Some("example.com"), None);
        assert_eq!(selection, TlsAutoSelection::None);
    }

    #[test]
    fn test_synthetic_tls_directive_creation() {
        let directive = create_synthetic_tls_directive("local");
        assert!(directive
            .args
            .iter()
            .any(|arg| matches!(arg, ServerConfigurationValue::Boolean(true, _))));

        let children = directive.children.unwrap();
        let provider_directive = children.directives.get("provider").unwrap();
        let provider_entry = &provider_directive[0];

        assert_eq!(provider_entry.args.len(), 1);
        if let ServerConfigurationValue::String(provider_name, _) = &provider_entry.args[0] {
            assert_eq!(provider_name, "local");
        } else {
            panic!("Expected string provider name");
        }
    }

    #[test]
    fn test_loopback_detection() {
        // Test hostname detection
        assert!(is_loopback_host("localhost"));
        assert!(!is_loopback_host("example.com"));
        assert!(!is_loopback_host("localhost.example.com"));

        // Test IP detection
        assert!(is_loopback_ip("127.0.0.1"));
        assert!(is_loopback_ip("::1"));
        assert!(!is_loopback_ip("192.168.1.1"));
        assert!(!is_loopback_ip("::ffff:192.168.1.1"));
    }
}
