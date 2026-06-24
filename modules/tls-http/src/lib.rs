use std::any::Any;
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;

use ferron_core::config_validator_scoped_key;
use ferron_core::loader::ModuleLoader;
use ferron_core::providers::Provider;
use ferron_observability::{build_composite_sink, CompositeEventSink};
use ferron_tls::builder::build_server_config_builder;
use ferron_tls::config::TlsServerConfig;
use ferron_tls::{TcpTlsContext, TcpTlsResolver};
use rustls::ServerConfig;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::config::{TlsHttpConfig, TlsHttpOnDemandConfigData};
use crate::fetch::{
    fetch_tls_cert_loop, run_tls_http_background_task, CertifiedKeyLock, ErrorMessageLock,
    SniCertLock, TlsHttpOnDemandResolver, TlsHttpResolver,
};

mod config;
mod fetch;
mod validator;

type TcpTlsHttpTaskData = (
    TlsHttpConfig,
    CertifiedKeyLock,
    ErrorMessageLock,
    String,
    Arc<CancellationToken>,
);

/// Global state for on-demand TLS HTTP mode, shared across all on-demand
/// config blocks and the background listener.
pub struct TlsHttpTaskState {
    pub on_demand_configs: Arc<RwLock<Vec<TlsHttpOnDemandConfigData>>>,
    pub on_demand_tx: async_channel::Sender<(String, u16)>,
    pub on_demand_rx: async_channel::Receiver<(String, u16)>,
    pub sni_cert_lock: SniCertLock,
    pub event_sink: Arc<parking_lot::RwLock<Option<Arc<CompositeEventSink>>>>,
}

impl TlsHttpTaskState {
    pub fn new() -> Self {
        let (tx, rx) = async_channel::unbounded();
        Self {
            on_demand_configs: Arc::new(RwLock::new(Vec::new())),
            on_demand_tx: tx,
            on_demand_rx: rx,
            sni_cert_lock: Arc::new(RwLock::new(HashMap::new())),
            event_sink: Arc::new(parking_lot::RwLock::new(None)),
        }
    }
}

impl Default for TlsHttpTaskState {
    fn default() -> Self {
        Self::new()
    }
}

struct GlobalTaskState {
    inner: std::sync::OnceLock<Arc<TlsHttpTaskState>>,
    event_sink: parking_lot::Mutex<Option<Arc<CompositeEventSink>>>,
}

impl GlobalTaskState {
    fn new() -> Self {
        Self {
            inner: std::sync::OnceLock::new(),
            event_sink: parking_lot::Mutex::new(None),
        }
    }

    fn set_event_sink(&self, event_sink: Arc<CompositeEventSink>) {
        *self.event_sink.lock() = Some(event_sink);
    }

    fn get_or_init(&self) -> Arc<TlsHttpTaskState> {
        let state = self
            .inner
            .get_or_init(|| Arc::new(TlsHttpTaskState::new()))
            .clone();
        if let Some(event_sink) = self.event_sink.lock().clone() {
            *state.event_sink.write() = Some(event_sink);
        }
        state
    }
}

static GLOBAL_TASK_STATE: std::sync::LazyLock<GlobalTaskState> =
    std::sync::LazyLock::new(GlobalTaskState::new);

pub struct TcpTlsHttpResolver {
    config: Arc<ServerConfig>,
    error_message: ErrorMessageLock,
}

#[async_trait::async_trait(?Send)]
impl TcpTlsResolver for TcpTlsHttpResolver {
    #[inline]
    fn get_tls_config(&self) -> Arc<ServerConfig> {
        self.config.clone()
    }

    #[inline]
    fn get_tls_background_error(&self) -> Option<String> {
        self.error_message.read().clone()
    }
}

/// TcpTlsResolver for on-demand mode.
///
/// Wraps a ServerConfig with the SNI-aware TlsHttpOnDemandResolver.
pub struct TcpTlsHttpOnDemandResolver {
    config: Arc<ServerConfig>,
    error_message: ErrorMessageLock,
}

#[async_trait::async_trait(?Send)]
impl TcpTlsResolver for TcpTlsHttpOnDemandResolver {
    #[inline]
    fn get_tls_config(&self) -> Arc<ServerConfig> {
        self.config.clone()
    }

    #[inline]
    fn get_tls_background_error(&self) -> Option<String> {
        self.error_message.read().clone()
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
        let error_message = Arc::new(parking_lot::RwLock::new(None));

        let host = ctx
            .domain
            .host
            .clone()
            .or_else(|| ctx.domain.ip.map(|i| i.to_canonical().to_string()))
            .unwrap_or_default();

        if http_config.on_demand {
            let task_state = GLOBAL_TASK_STATE.get_or_init();

            let port = ctx.port;

            let on_demand_data = TlsHttpOnDemandConfigData {
                url: http_config.url,
                no_verification: http_config.no_verification,
                refresh_interval: http_config.refresh_interval,
                on_demand_ask: http_config.on_demand_ask,
                on_demand_ask_auth: http_config.on_demand_ask_auth,
                on_demand_ask_no_verification: http_config.on_demand_ask_no_verification,
                sni_hostname: Some(host),
                port,
                error_message: error_message.clone(),
            };

            task_state
                .on_demand_configs
                .blocking_write()
                .push(on_demand_data);

            let on_demand_resolver = TlsHttpOnDemandResolver::new(
                task_state.sni_cert_lock.clone(),
                task_state.on_demand_tx.clone(),
                port,
            );

            let mut config_with_tickets =
                config_builder.with_cert_resolver(Arc::new(on_demand_resolver));

            if let Some(ticketer) = ticketer {
                config_with_tickets.ticketer = ticketer;
            }

            if let Some(alpn_protocols) = ctx.alpn.as_ref() {
                config_with_tickets.alpn_protocols = alpn_protocols.clone();
            }

            if tls_config.ocsp.enabled {
                let ocsp_handle = ferron_ocsp::get_service_handle()
                    .expect("OCSP service handle should always be available");
                let inner_resolver = config_with_tickets.cert_resolver.clone();
                config_with_tickets.cert_resolver =
                    Arc::new(ferron_ocsp::OcspStapler::new(inner_resolver, &ocsp_handle));
            }

            let config = Arc::new(config_with_tickets);

            ctx.resolver = Some(Arc::new(TcpTlsHttpOnDemandResolver {
                config,
                error_message,
            }));
        } else {
            let certified_key = Arc::new(parking_lot::RwLock::new(None));
            let resolver = TlsHttpResolver::new(certified_key.clone());

            let mut config_with_tickets = config_builder.with_cert_resolver(Arc::new(resolver));

            if let Some(ticketer) = ticketer {
                config_with_tickets.ticketer = ticketer;
            }

            if let Some(alpn_protocols) = ctx.alpn.as_ref() {
                config_with_tickets.alpn_protocols = alpn_protocols.clone();
            }

            if tls_config.ocsp.enabled {
                let ocsp_handle = ferron_ocsp::get_service_handle()
                    .expect("OCSP service handle should always be available");
                let inner_resolver = config_with_tickets.cert_resolver.clone();
                config_with_tickets.cert_resolver =
                    Arc::new(ferron_ocsp::OcspStapler::new(inner_resolver, &ocsp_handle));
            }

            let config = Arc::new(config_with_tickets);

            ctx.resolver = Some(Arc::new(TcpTlsHttpResolver {
                config,
                error_message: error_message.clone(),
            }));

            let _ = self.tx.try_send((
                http_config,
                certified_key,
                error_message,
                host,
                ferron_core::shutdown::RELOAD_TOKEN.load().clone(),
            ));
        }

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
            while let Ok((config, certified_key, resolver_error, host, cancel_token)) =
                rx.recv().await
            {
                tokio::spawn(cancel_token.deref().clone().run_until_cancelled_owned(
                    fetch_tls_cert_loop(config, certified_key, resolver_error, host, sink.clone()),
                ));
            }
        });

        // Start the on-demand background listener if there are on-demand configs
        let task_state = GLOBAL_TASK_STATE.get_or_init();
        if !task_state.on_demand_configs.blocking_read().is_empty() {
            let on_demand_rx = task_state.on_demand_rx.clone();
            let on_demand_configs = task_state.on_demand_configs.clone();
            let sni_cert_lock = task_state.sni_cert_lock.clone();
            let event_sink = self.event_sink.clone();

            runtime.spawn_secondary_task(async move {
                run_tls_http_background_task(
                    on_demand_rx,
                    on_demand_configs,
                    sni_cert_lock,
                    event_sink,
                )
                .await;
            });
        }

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
        let event_sink = build_composite_sink(&registry, &config.global_config, None)?;

        // Store the event sink in the global task state for on-demand mode
        GLOBAL_TASK_STATE.set_event_sink(event_sink.clone());

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
