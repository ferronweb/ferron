use super::hostname_radix_tree::HostnameRadixTree;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use std::sync::Arc;

/// The type for the SNI resolver lock, which is a vector of tuples containing the hostname and the corresponding certificate resolver.
pub type SniResolverLock = HostnameRadixTree<Arc<dyn ResolvesServerCert>>;

/// Custom SNI resolver, consisting of multiple resolvers
#[derive(Debug)]
pub struct CustomSniResolver {
    fallback_resolver: Option<Arc<dyn ResolvesServerCert>>,
    resolvers: SniResolverLock,
}

impl CustomSniResolver {
    /// Creates a custom SNI resolver
    pub fn new() -> Self {
        Self {
            fallback_resolver: None,
            resolvers: HostnameRadixTree::new(),
        }
    }

    /// Loads a fallback certificate resolver for a specific host
    pub fn load_fallback_resolver(&mut self, fallback_resolver: Arc<dyn ResolvesServerCert>) {
        self.fallback_resolver = Some(fallback_resolver);
    }

    /// Loads a host certificate resolver for a specific host
    pub fn load_host_resolver(&mut self, host: &str, resolver: Arc<dyn ResolvesServerCert>) {
        self.resolvers.insert(host.to_string(), resolver);
    }
}

impl ResolvesServerCert for CustomSniResolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let hostname = client_hello
            .server_name()
            .map(|hn| hn.strip_suffix('.').unwrap_or(hn));
        if let Some(hostname) = hostname {
            if let Some(resolver) = self.resolvers.get(hostname).cloned() {
                return resolver.resolve(client_hello);
            }
        }
        self.fallback_resolver
            .as_ref()
            .and_then(|r| r.resolve(client_hello))
    }
}
