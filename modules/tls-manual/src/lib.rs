use std::sync::{Arc, OnceLock};

use ferron_core::config_validator_scoped_key;
use ferron_core::providers::Provider;
use ferron_core::{config::validator::ConfigurationValidator, loader::ModuleLoader};
use ferron_observability::{build_composite_sink, CompositeEventSink};
use ferron_tls::validate_tls_common;
use ferron_tls::{
    builder::build_server_config_builder, config::TlsServerConfig, observability, TcpTlsContext,
    TcpTlsResolver,
};
use rustls::ServerConfig;
use rustls_pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};

/// Global event sink for the `tls-manual` module, populated from
/// [`TlsManualModuleLoader::register_modules`] and read by
/// [`TcpTlsManualProvider::execute`].
static EVENT_SINK: OnceLock<Arc<CompositeEventSink>> = OnceLock::new();

/// Set the event sink for the `tls-manual` module. Call during module
/// initialization. Multiple calls are ignored; only the first one wins.
pub fn set_event_sink(event_sink: Arc<CompositeEventSink>) {
    let _ = EVENT_SINK.set(event_sink);
}

fn event_sink() -> Option<Arc<CompositeEventSink>> {
    EVENT_SINK.get().cloned()
}

fn resolve_host(ctx: &TcpTlsContext<'_>) -> String {
    ctx.domain
        .host
        .clone()
        .or_else(|| ctx.domain.ip.map(|i| i.to_canonical().to_string()))
        .unwrap_or_default()
}

/// Check if a certificate has the OCSP Must-Staple (TLS Feature status_request) extension.
///
/// Per RFC 7633, the TLS Feature extension contains a SEQUENCE of feature values.
/// The `status_request` feature (value 5) indicates OCSP Must-Staple.
fn cert_has_must_staple(leaf: &CertificateDer<'_>) -> bool {
    use x509_parser::prelude::*;

    let Ok((_, cert)) = X509Certificate::from_der(leaf.as_ref()) else {
        return false;
    };

    for ext in cert.extensions() {
        // ext.oid.as_bytes() returns BER-encoded OID bytes
        // BER encoding of 1.3.6.1.5.5.7.1.24: 2b 06 01 05 05 07 01 18
        if ext.oid.as_bytes() == [0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x01, 0x18] {
            if let Ok((_, root)) = der_parser::der::parse_der(ext.value) {
                if let Ok(items) = root.as_sequence() {
                    return items
                        .iter()
                        .any(|item: &der_parser::ber::BerObject| item.as_u32().ok() == Some(5));
                }
            }
        }
    }
    false
}

/// Build a `rustls::sign::CertifiedKey` from loaded certs and private key.
///
/// Used to preload certificates with Must-Staple into the OCSP service for
/// immediate fetching.
fn build_certified_key(
    certs: &[CertificateDer<'static>],
    private_key: &PrivateKeyDer<'static>,
) -> Option<rustls::sign::CertifiedKey> {
    use rustls::crypto::aws_lc_rs::sign::any_supported_type;

    let signing_key = any_supported_type(private_key).ok()?;
    Some(rustls::sign::CertifiedKey::new(certs.to_vec(), signing_key))
}

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
        // Parse TLS configuration from the config block
        let tls_config = TlsServerConfig::from_config(ctx.config)
            .map_err(|e| std::io::Error::other(format!("Invalid TLS configuration: {e}")))?;

        // Build the ServerConfig up to the verifier stage using the shared builder
        let config_builder =
            build_server_config_builder(&tls_config.crypto, &tls_config.client_auth)?;

        // Parse ticket key configuration
        let ticketer = ferron_tls::builder::build_ticketer(ctx.config);

        // Load certificates
        let certs = load_certs(&tls_config.cert_path.ok_or(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "'cert' TLS parameter missing or invalid".to_string(),
        ))?)
        .map_err(|e| std::io::Error::other(format!("Error while loading TLS certificate: {e}")))?;

        // Load private key
        let private_key = load_private_key(&tls_config.key_path.ok_or(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "'cert' TLS parameter missing or invalid".to_string(),
        ))?)
        .map_err(|e| std::io::Error::other(format!("Error while loading TLS private key: {e}")))?;

        // Emit the unified `ferron.tls.certificate_not_after` gauge for the
        // leaf certificate that is about to be mounted into the in-memory
        // rustls context.
        if let (Some(sink), Some(leaf)) = (event_sink(), certs.first()) {
            observability::emit_certificate_not_after(&sink, "manual", &resolve_host(ctx), leaf);
        }

        // Build the config with certificates
        let mut config_with_tickets =
            config_builder.with_single_cert(certs.clone(), private_key.clone_key())?;

        // Attach the ticketer
        if let Some(ticketer) = ticketer {
            config_with_tickets.ticketer = ticketer;
        }

        if let Some(alpn_protocols) = ctx.alpn.as_ref() {
            config_with_tickets.alpn_protocols = alpn_protocols.clone();
        }

        // Wrap cert_resolver with OCSP stapler if enabled
        if tls_config.ocsp.enabled {
            let ocsp_handle = ferron_ocsp::get_service_handle()
                .expect("OCSP service handle should always be available");
            let inner_resolver = config_with_tickets.cert_resolver.clone();
            config_with_tickets.cert_resolver =
                Arc::new(ferron_ocsp::OcspStapler::new(inner_resolver, &ocsp_handle));

            // Preload the certificate for immediate OCSP fetching.
            // Without preloading, the first TLS handshake for each server
            // would not include a stapled OCSP response because the fetch
            // hasn't completed yet. Preloading ensures the background task
            // starts fetching as soon as the config is loaded.
            if let Some(certified_key) = build_certified_key(&certs, &private_key) {
                if let Some(leaf) = certs.first() {
                    if cert_has_must_staple(leaf) {
                        ferron_core::log_info!(
                            "OCSP stapling enabled — Must-Staple detected, preloading certificate"
                        );
                    }
                    // The same leaf is being mounted into the OCSP service; emit
                    // the unified cert expiration gauge a second time so
                    // observers can see that preload as a distinct mount.
                    if let Some(sink) = event_sink() {
                        observability::emit_certificate_not_after(
                            &sink,
                            "manual",
                            &resolve_host(ctx),
                            leaf,
                        );
                    }
                }
                ocsp_handle.preload(certified_key.cert.clone());
            }
        }

        let config = Arc::new(config_with_tickets);

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

pub struct TlsManualConfigurationValidator;

impl ConfigurationValidator for TlsManualConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        validator_ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        validate_tls_common!(config, validator_ctx);

        Ok(())
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

    fn register_scoped_configuration_validators(
        &mut self,
        registry: &mut std::collections::HashMap<
            ferron_core::config::validator::ConfigurationValidatorScopedKey,
            Box<dyn ferron_core::config::validator::ConfigurationValidator>,
        >,
    ) {
        registry.insert(
            config_validator_scoped_key!("tls", "manual"),
            Box::new(TlsManualConfigurationValidator),
        );
    }

    fn register_modules(
        &mut self,
        registry: Arc<ferron_core::registry::Registry>,
        _modules: &mut Vec<Arc<dyn ferron_core::Module>>,
        config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let event_sink = build_composite_sink(&registry, &config.global_config)?;
        set_event_sink(event_sink);
        Ok(())
    }
}
