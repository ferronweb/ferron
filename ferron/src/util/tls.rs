use crate::util::HostnameRadixTree;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use std::sync::Arc;

/// The type for the SNI resolver lock, which is a vector of tuples containing the hostname and the corresponding certificate resolver.
pub type SniResolverLock = Arc<tokio::sync::RwLock<HostnameRadixTree<Arc<dyn ResolvesServerCert>>>>;

/// Custom SNI resolver, consisting of multiple resolvers
#[derive(Debug)]
pub struct CustomSniResolver {
  fallback_resolver: Option<Arc<dyn ResolvesServerCert>>,
  resolvers: SniResolverLock,
  fallback_sender: Option<(async_channel::Sender<(String, u16)>, u16)>,
}

impl CustomSniResolver {
  /// Creates a custom SNI resolver
  #[allow(dead_code)]
  pub fn new() -> Self {
    Self {
      fallback_resolver: None,
      resolvers: Arc::new(tokio::sync::RwLock::new(HostnameRadixTree::new())),
      fallback_sender: None,
    }
  }

  /// Creates a custom SNI resolver with provided resolvers lock
  pub fn with_resolvers(resolvers: SniResolverLock) -> Self {
    Self {
      fallback_resolver: None,
      resolvers,
      fallback_sender: None,
    }
  }

  /// Loads a fallback certificate resolver for a specific host
  pub fn load_fallback_resolver(&mut self, fallback_resolver: Arc<dyn ResolvesServerCert>) {
    self.fallback_resolver = Some(fallback_resolver);
  }

  /// Loads a host certificate resolver for a specific host
  pub fn load_host_resolver(&mut self, host: &str, resolver: Arc<dyn ResolvesServerCert>) {
    self.resolvers.blocking_write().insert(host.to_string(), resolver);
  }

  /// Loads a fallback sender used for sending SNI hostnames for a specific host
  pub fn load_fallback_sender(&mut self, fallback_sender: async_channel::Sender<(String, u16)>, port: u16) {
    self.fallback_sender = Some((fallback_sender, port));
  }
}

impl ResolvesServerCert for CustomSniResolver {
  fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
    let hostname = client_hello.server_name().map(|hn| hn.strip_suffix('.').unwrap_or(hn));
    if let Some(hostname) = hostname {
      // If blocking_read() method is used when only Tokio is used, the program would panic on resolving a TLS certificate.
      #[cfg(feature = "runtime-monoio")]
      let resolvers = self.resolvers.blocking_read();
      #[cfg(feature = "runtime-tokio")]
      let resolvers = futures_executor::block_on(async { self.resolvers.read().await });

      if let Some(resolver) = resolvers.get(hostname).cloned() {
        return resolver.resolve(client_hello);
      }
    }
    let hostname = hostname.map(|v| v.to_string());
    self
      .fallback_resolver
      .as_ref()
      .and_then(|r| r.resolve(client_hello))
      .or_else(|| {
        if let Some((sender, port)) = &self.fallback_sender {
          if let Some(hostname) = hostname {
            sender.send_blocking((hostname.to_string(), *port)).unwrap_or_default();
          }
        }
        None
      })
  }
}

/// A certificate resolver resolving one certified key
#[derive(Debug)]
pub struct OneCertifiedKeyResolver {
  certified_key: Arc<CertifiedKey>,
}

impl OneCertifiedKeyResolver {
  /// Creates a certificate resolver with a certified key
  pub fn new(certified_key: Arc<CertifiedKey>) -> Self {
    Self { certified_key }
  }
}

impl ResolvesServerCert for OneCertifiedKeyResolver {
  fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
    Some(self.certified_key.clone())
  }
}

/// Loads a public certificate from file
pub fn load_certs(filename: &str) -> std::io::Result<Vec<CertificateDer<'static>>> {
  let mut certfile = std::fs::File::open(filename)?;
  CertificateDer::pem_reader_iter(&mut certfile)
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| match e {
      rustls_pki_types::pem::Error::Io(err) => err,
      err => std::io::Error::other(err),
    })
}

/// Loads a private key from file
pub fn load_private_key(filename: &str) -> std::io::Result<PrivateKeyDer<'static>> {
  let mut keyfile = std::fs::File::open(filename)?;
  match PrivateKeyDer::from_pem_reader(&mut keyfile) {
    Ok(private_key) => Ok(private_key),
    Err(rustls_pki_types::pem::Error::Io(err)) => Err(err),
    Err(err) => Err(std::io::Error::other(err)),
  }
}
