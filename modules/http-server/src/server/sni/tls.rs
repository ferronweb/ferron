use super::hostname_radix_tree::HostnameRadixTree;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use std::sync::Arc;

/// The type for the SNI resolver lock, which is a vector of tuples containing the hostname and the corresponding certificate resolver.
pub type SniResolverLock = Arc<tokio::sync::RwLock<HostnameRadixTree<Arc<dyn ResolvesServerCert>>>>;

/// Custom SNI resolver, consisting of multiple resolvers
#[derive(Debug)]
pub struct CustomSniResolver {
    fallback_resolver: Option<Arc<dyn ResolvesServerCert>>,
    resolvers: SniResolverLock,
}

impl CustomSniResolver {
    /// Creates a custom SNI resolver
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            fallback_resolver: None,
            resolvers: Arc::new(tokio::sync::RwLock::new(HostnameRadixTree::new())),
        }
    }

    /// Creates a custom SNI resolver with provided resolvers lock
    #[allow(dead_code)]
    pub fn with_resolvers(resolvers: SniResolverLock) -> Self {
        Self {
            fallback_resolver: None,
            resolvers,
        }
    }

    /// Loads a fallback certificate resolver for a specific host
    pub fn load_fallback_resolver(&mut self, fallback_resolver: Arc<dyn ResolvesServerCert>) {
        self.fallback_resolver = Some(fallback_resolver);
    }

    /// Loads a host certificate resolver for a specific host
    pub fn load_host_resolver(&mut self, host: &str, resolver: Arc<dyn ResolvesServerCert>) {
        self.resolvers
            .blocking_write()
            .insert(host.to_string(), resolver);
    }
}

impl ResolvesServerCert for CustomSniResolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let hostname = client_hello
            .server_name()
            .map(|hn| hn.strip_suffix('.').unwrap_or(hn));
        if let Some(hostname) = hostname {
            // If blocking_read() method is used when only Tokio is used, the program would panic on resolving a TLS certificate.
            // In this case, `vibeio` is used as a primary runtime, so no issue.
            let resolvers = self.resolvers.blocking_read();

            if let Some(resolver) = resolvers.get(hostname).cloned() {
                return resolver.resolve(client_hello);
            }
        }
        self.fallback_resolver
            .as_ref()
            .and_then(|r| r.resolve(client_hello))
    }
}
