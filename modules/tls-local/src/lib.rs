use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use ferron_core::loader::ModuleLoader;
use ferron_core::providers::Provider;
use ferron_core::registry::RegistryBuilder;
use ferron_core::Module;
use ferron_tls::{
    builder::build_server_config_builder, config::TlsServerConfig, TcpTlsContext, TcpTlsResolver,
};
use rustls::server::ResolvesServerCert;
use rustls::sign::CertifiedKey;
use rustls::ServerConfig;

pub mod cache;
pub mod provision;
#[cfg(test)]
mod tests;

use crate::cache::LocalTlsCache;
use crate::provision::provision_local_cert;

#[derive(Debug)]
struct LocalSingleCertResolver(Arc<CertifiedKey>);

impl ResolvesServerCert for LocalSingleCertResolver {
    fn resolve(&self, _client_hello: rustls::server::ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.clone())
    }
}

pub struct TcpTlsLocalResolver {
    config: Arc<ServerConfig>,
}

#[async_trait]
impl TcpTlsResolver for TcpTlsLocalResolver {
    #[inline]
    fn get_tls_config(&self) -> Arc<ServerConfig> {
        self.config.clone()
    }
}

pub struct TcpTlsLocalProvider {
    cache: Arc<LocalTlsCache>,
}

impl TcpTlsLocalProvider {
    pub fn new(cache_path: PathBuf) -> Self {
        Self {
            cache: Arc::new(LocalTlsCache::new(cache_path)),
        }
    }
}

impl<'a> Provider<TcpTlsContext<'a>> for TcpTlsLocalProvider {
    fn name(&self) -> &str {
        "local"
    }

    fn execute(&self, ctx: &mut TcpTlsContext) -> Result<(), Box<dyn std::error::Error>> {
        // Parse TLS configuration from the config block
        // We reuse the standard TlsServerConfig for crypto/mTLS settings
        let tls_config = TlsServerConfig::from_config(ctx.config)
            .map_err(|e| std::io::Error::other(format!("Invalid TLS configuration: {e}")))?;

        // Provision the local certificate (CA + leaf)
        let certified_key = provision_local_cert(&self.cache, &ctx.domain)?;

        // Build the ServerConfig
        let config_builder =
            build_server_config_builder(&tls_config.crypto, &tls_config.client_auth)?;

        // Install the certificate via a custom resolver since we have a CertifiedKey
        let server_config =
            config_builder.with_cert_resolver(Arc::new(LocalSingleCertResolver(certified_key)));

        ctx.resolver = Some(Arc::new(TcpTlsLocalResolver {
            config: Arc::new(server_config),
        }));

        Ok(())
    }
}

pub struct LocalTlsModuleLoader;

impl ModuleLoader for LocalTlsModuleLoader {
    fn register_providers(&mut self, mut registry: RegistryBuilder) -> RegistryBuilder {
        // Default cache path
        let cache_path = dirs::data_local_dir()
            .and_then(|mut p| {
                let metadata = std::fs::metadata(&p);
                if let Ok(metadata) = metadata {
                    if !metadata.is_dir() || metadata.permissions().readonly() {
                        return None;
                    }

                    p.push("ferron-local-tls");
                    Some(p)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| PathBuf::from(".ferron-local-tls"));

        registry = registry.with_provider::<TcpTlsContext, _>(move || {
            Arc::new(TcpTlsLocalProvider::new(cache_path.clone()))
        });
        registry
    }
}

pub struct LocalTlsModule;

impl Module for LocalTlsModule {
    fn name(&self) -> &str {
        "tls-local"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn start(
        &self,
        _runtime: &mut ferron_core::runtime::Runtime,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }
}
