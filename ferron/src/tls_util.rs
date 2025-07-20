use crate::util::match_hostname;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use std::{collections::HashMap, sync::Arc};

/// Custom SNI resolver, consisting of multiple resolvers
#[derive(Debug)]
pub struct CustomSniResolver {
  fallback_resolver: Option<Arc<dyn ResolvesServerCert>>,
  resolvers: Arc<tokio::sync::RwLock<HashMap<String, Arc<dyn ResolvesServerCert>>>>,
  fallback_sender: Option<(async_channel::Sender<(String, u16)>, u16)>,
}

impl CustomSniResolver {
  /// Creates a custom SNI resolver
  #[allow(dead_code)]
  pub fn new() -> Self {
    Self {
      fallback_resolver: None,
      resolvers: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
      fallback_sender: None,
    }
  }

  /// Creates a custom SNI resolver with provided resolvers lock
  pub fn with_resolvers(resolvers: Arc<tokio::sync::RwLock<HashMap<String, Arc<dyn ResolvesServerCert>>>>) -> Self {
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
    let hostname = client_hello.server_name();
    if let Some(hostname) = hostname {
      let resolvers = self.resolvers.blocking_read();
      let keys_iterator = resolvers.keys();
      for configured_hostname in keys_iterator {
        if match_hostname(Some(configured_hostname), Some(hostname)) {
          return resolvers.get(configured_hostname).and_then(|r| r.resolve(client_hello));
        }
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
  let certfile = std::fs::File::open(filename)?;
  let mut reader = std::io::BufReader::new(certfile);
  rustls_pemfile::certs(&mut reader).collect()
}

/// Loads a private key from file
pub fn load_private_key(filename: &str) -> std::io::Result<PrivateKeyDer<'static>> {
  let keyfile = std::fs::File::open(filename)?;
  let mut reader = std::io::BufReader::new(keyfile);
  match rustls_pemfile::private_key(&mut reader) {
    Ok(Some(private_key)) => Ok(private_key),
    Ok(None) => Err(std::io::Error::new(
      std::io::ErrorKind::InvalidData,
      "Invalid private key",
    )),
    Err(err) => Err(err),
  }
}
