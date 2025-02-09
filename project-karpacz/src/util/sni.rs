use crate::project_karpacz_util::match_hostname::match_hostname;
use rustls::{server::ResolvesServerCert, sign::CertifiedKey};
use std::{collections::HashMap, sync::Arc};

#[derive(Debug)]
pub struct CustomSniResolver {
  fallback_cert_key: Option<Arc<CertifiedKey>>,
  cert_keys: HashMap<String, Arc<CertifiedKey>>,
}

impl CustomSniResolver {
  pub fn new() -> Self {
    CustomSniResolver {
      fallback_cert_key: None,
      cert_keys: HashMap::new(),
    }
  }

  pub fn load_fallback_cert_key(&mut self, fallback_cert_key: Arc<CertifiedKey>) {
    self.fallback_cert_key = Some(fallback_cert_key);
  }

  pub fn load_host_cert_key(&mut self, host: &str, cert_key: Arc<CertifiedKey>) {
    self.cert_keys.insert(String::from(host), cert_key);
  }
}

impl ResolvesServerCert for CustomSniResolver {
  fn resolve(
    &self,
    client_hello: rustls::server::ClientHello<'_>,
  ) -> Option<Arc<rustls::sign::CertifiedKey>> {
    let hostname = client_hello.server_name();
    if let Some(hostname) = hostname {
      let keys_iterator = self.cert_keys.keys();
      for configured_hostname in keys_iterator {
        if match_hostname(Some(configured_hostname), Some(hostname)) {
          return self.cert_keys.get(configured_hostname).cloned();
        }
      }
      self.fallback_cert_key.clone()
    } else {
      self.fallback_cert_key.clone()
    }
  }
}
