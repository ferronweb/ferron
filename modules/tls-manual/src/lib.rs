use std::{collections::HashMap, sync::Arc};

use ferron_core::loader::ModuleLoader;
use ferron_core::providers::Provider;
use ferron_tls::{TcpTlsContext, TcpTlsResolver};
use rustls::ServerConfig;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};

pub struct TcpTlsManualResolver {
    config: Arc<ServerConfig>,
}

#[async_trait::async_trait(?Send)]
impl TcpTlsResolver for TcpTlsManualResolver {
    #[inline]
    fn get_tls_config(&self) -> Arc<ServerConfig> {
        self.config.clone()
    }
}

pub struct TcpTlsManualProvider;

impl<'a> Provider<TcpTlsContext<'a>> for TcpTlsManualProvider {
    fn name(&self) -> &str {
        "manual"
    }

    fn execute(&self, ctx: &mut TcpTlsContext) -> Result<(), Box<dyn std::error::Error>> {
        // TODO: configure TLS crypto provider
        let provider = rustls::crypto::aws_lc_rs::default_provider();
        // TODO: mTLS
        let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(provider))
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(
                load_certs(
                    ctx.config
                        .get_value("cert")
                        .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                        .as_deref()
                        .ok_or(std::io::Error::other(
                            "'cert' TLS parameter missing or invalid",
                        ))?,
                )?,
                load_private_key(
                    ctx.config
                        .get_value("key")
                        .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                        .as_deref()
                        .ok_or(std::io::Error::other(
                            "'key' TLS parameter missing or invalid",
                        ))?,
                )?,
            )?;
        if let Some(alpn_protocols) = ctx.alpn.as_ref() {
            config.alpn_protocols = alpn_protocols.clone();
        }
        let config = Arc::new(config);

        ctx.resolver = Some(Arc::new(TcpTlsManualResolver { config }));

        Ok(())
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

#[derive(Clone, Default)]
pub struct TlsManualModuleLoader;

impl ModuleLoader for TlsManualModuleLoader {
    fn register_providers(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        registry.with_provider::<TcpTlsContext, _>(|| Arc::new(TcpTlsManualProvider))
    }
}
