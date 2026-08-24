use std::sync::{Arc, OnceLock};

use ferron_core::config::validator::{ConfigurationValidationError, ConfigurationValidator};
use ferron_core::config_validator_scoped_key;
use ferron_core::loader::ModuleLoader;
use ferron_core::providers::Provider;
use ferron_observability::{build_composite_sink, CompositeEventSink};
use ferron_tls::builder::build_server_config_builder;
use ferron_tls::config::TlsServerConfig;
use ferron_tls::{observability, validate_tls_common, TlsContext, TlsResolver};
use rustls::ServerConfig;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};

/// Global event sink for the `tls-manual` module, populated from
/// [`TlsManualModuleLoader::register_modules`] and read by
/// [`TlsManualProvider::execute`].
static EVENT_SINK: OnceLock<Arc<CompositeEventSink>> = OnceLock::new();

fn resolve_host(ctx: &TlsContext<'_>) -> String {
    ctx.domain
        .host
        .clone()
        .or_else(|| ctx.domain.ip.map(|i| i.to_canonical().to_string()))
        .unwrap_or_default()
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

pub struct TlsManualResolver {
    config: Arc<ServerConfig>,
}

#[async_trait::async_trait(?Send)]
impl TlsResolver for TlsManualResolver {
    #[inline]
    fn get_tls_config(&self) -> Arc<ServerConfig> {
        self.config.clone()
    }
}

pub struct TlsManualProvider;

impl<'a> Provider<TlsContext<'a>> for TlsManualProvider {
    fn name(&self) -> &str {
        "manual"
    }

    fn execute(&self, ctx: &mut TlsContext) -> Result<(), Box<dyn std::error::Error>> {
        let tls_config = TlsServerConfig::from_config(ctx.config)
            .map_err(|e| std::io::Error::other(format!("Invalid TLS configuration: {e}")))?;

        let config_builder =
            build_server_config_builder(&tls_config.crypto, &tls_config.client_auth)?;

        let ticketer = ferron_tls::builder::build_ticketer(ctx.config);

        let certs = load_certs(&tls_config.cert_path.ok_or(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "'cert' TLS parameter missing or invalid".to_string(),
        ))?)
        .map_err(|e| std::io::Error::other(format!("Error while loading TLS certificate: {e}")))?;

        let private_key = load_private_key(&tls_config.key_path.ok_or(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "'cert' TLS parameter missing or invalid".to_string(),
        ))?)
        .map_err(|e| std::io::Error::other(format!("Error while loading TLS private key: {e}")))?;

        // Emit the unified `ferron.tls.certificate_not_after` gauge for the
        // leaf certificate that is about to be mounted into the in-memory
        // rustls context.
        if let (Some(sink), Some(leaf)) = (EVENT_SINK.get(), certs.first()) {
            observability::emit_certificate_not_after(sink, "manual", &resolve_host(ctx), leaf);
        }

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
                    // The same leaf is being mounted into the OCSP service; emit
                    // the unified cert expiration gauge a second time so
                    // observers can see that preload as a distinct mount.
                    if let Some(sink) = EVENT_SINK.get() {
                        observability::emit_certificate_not_after(
                            sink,
                            "manual",
                            &resolve_host(ctx),
                            leaf,
                        );
                    }
                }
                // It would be already preloaded, even without Must-Staple extension,
                // no need to check for Must-Staple...
                ocsp_handle.preload_with_host(certified_key.cert.clone(), resolve_host(ctx));
            }
        }

        let config = Arc::new(config_with_tickets);

        ctx.resolver = Some(Arc::new(TlsManualResolver { config }));

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
    ) -> Result<(), ConfigurationValidationError> {
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
        registry.with_provider::<TlsContext, _>(|| Arc::new(TlsManualProvider))
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
        let event_sink = build_composite_sink(&registry, &config.global_config, None)?;
        let _ = EVENT_SINK.set(event_sink);
        Ok(())
    }
}
