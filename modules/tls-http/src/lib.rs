use std::{any::Any, ops::Deref, sync::Arc};

use ferron_core::providers::Provider;
use ferron_core::{config_validator_scoped_key, loader::ModuleLoader};
use ferron_observability::{build_composite_sink, CompositeEventSink};
use ferron_tls::{
    builder::build_server_config_builder, config::TlsServerConfig, TcpTlsContext, TcpTlsResolver,
};
use rustls::ServerConfig;
use tokio_util::sync::CancellationToken;

use crate::{
    config::TlsHttpConfig,
    fetch::{fetch_tls_cert_loop, CertifiedKeyLock, TlsHttpResolver},
};

mod config;
mod fetch;
mod validator;

type TcpTlsHttpTaskData = (
    TlsHttpConfig,
    CertifiedKeyLock,
    String,
    Arc<CancellationToken>,
);

pub struct TcpTlsHttpResolver {
    config: Arc<ServerConfig>,
}

#[async_trait::async_trait(?Send)]
impl TcpTlsResolver for TcpTlsHttpResolver {
    #[inline]
    fn get_tls_config(&self) -> Arc<ServerConfig> {
        self.config.clone()
    }
}

pub struct TcpTlsHttpProvider {
    tx: async_channel::Sender<TcpTlsHttpTaskData>,
}

impl<'a> Provider<TcpTlsContext<'a>> for TcpTlsHttpProvider {
    fn name(&self) -> &str {
        "http"
    }

    fn execute(&self, ctx: &mut TcpTlsContext) -> Result<(), Box<dyn std::error::Error>> {
        // Parse TLS configuration from the config block
        let tls_config = TlsServerConfig::from_config(ctx.config)
            .map_err(|e| std::io::Error::other(format!("Invalid TLS configuration: {e}")))?;
        let http_config = TlsHttpConfig::from_config(ctx.config)
            .map_err(|e| std::io::Error::other(format!("Invalid `tls-http` configuration: {e}")))?;

        // Build the ServerConfig up to the verifier stage using the shared builder
        let config_builder =
            build_server_config_builder(&tls_config.crypto, &tls_config.client_auth)?;

        // Parse ticket key configuration
        let ticketer = ferron_tls::builder::build_ticketer(ctx.config);

        let certified_key = Arc::new(parking_lot::RwLock::new(None));
        let resolver = TlsHttpResolver::new(certified_key.clone());

        // Build the config with certificates
        let mut config_with_tickets = config_builder.with_cert_resolver(Arc::new(resolver));

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
        }

        let config = Arc::new(config_with_tickets);

        ctx.resolver = Some(Arc::new(TcpTlsHttpResolver { config }));

        let host = ctx
            .domain
            .host
            .clone()
            .or_else(|| ctx.domain.ip.map(|i| i.to_canonical().to_string()))
            .unwrap_or_default();

        let _ = self.tx.try_send((
            http_config,
            certified_key,
            host,
            ferron_core::shutdown::RELOAD_TOKEN.load().clone(),
        ));
        Ok(())
    }
}

pub struct TcpTlsHttpModule {
    rx: async_channel::Receiver<TcpTlsHttpTaskData>,
    event_sink: Arc<CompositeEventSink>,
}

impl ferron_core::Module for TcpTlsHttpModule {
    fn name(&self) -> &str {
        "tls-http"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn start(
        &self,
        runtime: &mut ferron_core::runtime::Runtime,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let rx = self.rx.clone();
        let sink = self.event_sink.clone();
        runtime.spawn_secondary_task(async move {
            while let Ok((config, certified_key, host, cancel_token)) = rx.recv().await {
                tokio::spawn(cancel_token.deref().clone().run_until_cancelled_owned(
                    fetch_tls_cert_loop(config, certified_key, host, sink.clone()),
                ));
            }
        });
        Ok(())
    }
}

#[derive(Clone)]
pub struct TlsHttpModuleLoader {
    tx: async_channel::Sender<TcpTlsHttpTaskData>,
    rx: async_channel::Receiver<TcpTlsHttpTaskData>,
    loaded_module: Option<Arc<dyn ferron_core::Module>>,
}

impl Default for TlsHttpModuleLoader {
    fn default() -> Self {
        let (tx, rx) = async_channel::unbounded();
        Self {
            tx,
            rx,
            loaded_module: None,
        }
    }
}

impl ModuleLoader for TlsHttpModuleLoader {
    fn register_providers(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        let tx = self.tx.clone();
        registry.with_provider::<TcpTlsContext, _>(move || {
            Arc::new(TcpTlsHttpProvider { tx: tx.clone() })
        })
    }

    fn register_modules(
        &mut self,
        registry: Arc<ferron_core::registry::Registry>,
        modules: &mut Vec<Arc<dyn ferron_core::Module>>,
        config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Build the composite event sink from observability providers
        let event_sink = build_composite_sink(&registry, &config.global_config)?;

        if self.loaded_module.is_none() {
            let module = Arc::new(TcpTlsHttpModule {
                rx: self.rx.clone(),
                event_sink,
            });
            modules.push(module);
        }
        Ok(())
    }

    fn register_scoped_configuration_validators(
        &mut self,
        registry: &mut std::collections::HashMap<
            ferron_core::config::validator::ConfigurationValidatorScopedKey,
            Box<dyn ferron_core::config::validator::ConfigurationValidator>,
        >,
    ) {
        registry.insert(
            config_validator_scoped_key!("tls", "http"),
            Box::new(validator::TlsHttpConfigurationValidator),
        );
    }
}
