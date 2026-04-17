//! ACME TLS provider module for Ferron.
//!
//! Supports eager and on-demand TLS certificate issuance via the ACME protocol.
//!
//! Supported challenge types:
//! - HTTP-01
//! - TLS-ALPN-01
//! - DNS-01
//!
//! # Example Configuration
//!
//! ```text
//! example.com:443 {
//!     tls {
//!         provider "acme"
//!         challenge http-01
//!         contact "admin@example.com"
//!     }
//!     root "/var/www/example.com"
//! }
//! ```

pub mod cache;
pub mod challenge;
pub mod config;
pub mod errors;
pub mod on_demand;
pub mod provision;
pub mod resolver;
pub mod stages;

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::loader::ModuleLoader;
use ferron_core::providers::Provider;
use ferron_core::registry::{ProviderRegistry, RegistryBuilder, GLOBAL_REGISTRY};
use ferron_core::{runtime::Runtime, Module};
use ferron_dns::{DnsClient, DnsContext};
use ferron_observability::{
    build_composite_sink, Event, LogEvent, LogLevel, MetricAttributeValue, MetricEvent, MetricType,
    MetricValue,
};
use ferron_tls::TcpTlsContext;
use instant_acme::ChallengeType;
use tokio::sync::RwLock;

use crate::config::{parse_acme_config, AcmeConfigOrOnDemand, SniResolverLock};
use crate::on_demand::OnDemandRequest;
use crate::resolver::TcpTlsAcmeResolver;

/// Shared state for the ACME background task.
pub struct AcmeTaskState {
    /// Shared list of ACME configs (both eager and dynamically added on-demand).
    pub configs: Arc<RwLock<Vec<crate::config::AcmeConfig>>>,
    /// On-demand configurations for lazy certificate issuance.
    pub on_demand_configs: Arc<RwLock<Vec<crate::config::AcmeOnDemandConfigData>>>,
    /// Channel sender for on-demand certificate requests.
    pub on_demand_tx: async_channel::Sender<OnDemandRequest>,
    /// Channel receiver for on-demand certificate requests.
    pub on_demand_rx: async_channel::Receiver<OnDemandRequest>,
    /// Shared TLS-ALPN-01 resolver locks.
    pub tls_alpn_01_resolvers: Arc<RwLock<Vec<crate::challenge::TlsAlpn01DataLock>>>,
    /// Shared HTTP-01 resolver locks.
    pub http_01_resolvers: Arc<RwLock<Vec<crate::challenge::Http01DataLock>>>,
    /// Shared memory account cache.
    pub memory_account_cache: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    /// Shared SNI resolver lock.
    pub sni_resolver_lock: SniResolverLock,
    /// Event sink for observability.
    pub event_sink: Arc<ferron_observability::CompositeEventSink>,
}

impl Default for AcmeTaskState {
    fn default() -> Self {
        // Default creates with an empty event sink - should be overridden before use
        Self::new(Arc::new(ferron_observability::CompositeEventSink::new(
            Vec::new(),
        )))
    }
}

impl AcmeTaskState {
    pub fn new(event_sink: Arc<ferron_observability::CompositeEventSink>) -> Self {
        let (tx, rx) = async_channel::unbounded();
        Self {
            configs: Arc::new(RwLock::new(Vec::new())),
            on_demand_configs: Arc::new(RwLock::new(Vec::new())),
            on_demand_tx: tx,
            on_demand_rx: rx,
            tls_alpn_01_resolvers: Arc::new(RwLock::new(Vec::new())),
            http_01_resolvers: Arc::new(RwLock::new(Vec::new())),
            memory_account_cache: Arc::new(RwLock::new(HashMap::new())),
            sni_resolver_lock: Arc::new(RwLock::new(HashMap::new())),
            event_sink,
        }
    }
}

/// Global AcmeTaskState holder with interior mutability for the event sink.
struct GlobalTaskState {
    inner: std::sync::OnceLock<Arc<AcmeTaskState>>,
    event_sink: parking_lot::Mutex<Option<Arc<ferron_observability::CompositeEventSink>>>,
}

impl GlobalTaskState {
    fn new() -> Self {
        Self {
            inner: std::sync::OnceLock::new(),
            event_sink: parking_lot::Mutex::new(None),
        }
    }

    fn set_event_sink(&self, event_sink: Arc<ferron_observability::CompositeEventSink>) {
        *self.event_sink.lock() = Some(event_sink);
    }

    fn get_or_init(&self) -> Arc<AcmeTaskState> {
        self.inner
            .get_or_init(|| {
                let event_sink = self.event_sink.lock().clone().unwrap_or_else(|| {
                    Arc::new(ferron_observability::CompositeEventSink::new(Vec::new()))
                });
                Arc::new(AcmeTaskState::new(event_sink))
            })
            .clone()
    }

    #[allow(dead_code)]
    fn get(&self) -> Option<Arc<AcmeTaskState>> {
        self.inner.get().cloned()
    }
}

/// Global ACME task state, lazily initialized.
static GLOBAL_TASK_STATE: std::sync::LazyLock<GlobalTaskState> =
    std::sync::LazyLock::new(GlobalTaskState::new);

/// Set the event sink for the ACME module. Call during module initialization.
pub fn set_event_sink(event_sink: Arc<ferron_observability::CompositeEventSink>) {
    GLOBAL_TASK_STATE.set_event_sink(event_sink);
}

fn get_or_init_task_state() -> Arc<AcmeTaskState> {
    GLOBAL_TASK_STATE.get_or_init()
}

/// ACME TLS provider.
///
/// Implements `Provider<TcpTlsContext>` to handle `tls { provider "acme"; ... }` blocks.
pub struct TcpTlsAcmeProvider;

impl Provider<TcpTlsContext<'_>> for TcpTlsAcmeProvider {
    fn name(&self) -> &str {
        "acme"
    }

    fn execute(&self, ctx: &mut TcpTlsContext) -> Result<(), Box<dyn std::error::Error>> {
        let domain = ctx
            .domain
            .host
            .clone()
            .or_else(|| ctx.domain.ip.map(|i| i.to_canonical().to_string()))
            .ok_or("ACME TLS provider requires a domain name or IP address")?;
        let port: u16 = ctx.port;

        // Resolve DNS client from nested dns { } block if present
        let dns_client = resolve_dns_client_from_config(ctx.config)?;

        let task_state = get_or_init_task_state();

        let acme_result = parse_acme_config(
            ctx.config,
            &domain,
            port,
            task_state.memory_account_cache.clone(),
            task_state.tls_alpn_01_resolvers.clone(),
            task_state.http_01_resolvers.clone(),
            task_state.sni_resolver_lock.clone(),
            dns_client,
        )
        .map_err(|e| format!("Failed to parse ACME config: {e}"))?;

        match acme_result {
            AcmeConfigOrOnDemand::Eager(acme_config) => {
                let certified_key_lock = acme_config.certified_key_lock.clone();
                let challenge_type = acme_config.challenge_type.clone();

                // Add to configs list
                task_state.configs.blocking_write().push(acme_config);

                // Build TLS resolver
                let tls_alpn_resolvers = if challenge_type == ChallengeType::TlsAlpn01 {
                    Some(task_state.tls_alpn_01_resolvers.clone())
                } else {
                    None
                };

                let alpn_protocols = ctx.alpn.clone().unwrap_or_default();

                // Parse OCSP and ticket key configuration
                let ocsp_config = ferron_tls::config::OcspConfig::from_config(ctx.config);
                let ocsp_handle = crate::resolver::get_ocsp_handle_if_enabled(&ocsp_config);
                let ticketer = ferron_tls::builder::build_ticketer(ctx.config);

                let acme_resolver = TcpTlsAcmeResolver::new(
                    certified_key_lock,
                    tls_alpn_resolvers,
                    alpn_protocols,
                    ocsp_config,
                    ocsp_handle,
                    ticketer,
                );

                ctx.resolver = Some(Arc::new(acme_resolver));
            }
            AcmeConfigOrOnDemand::OnDemand(on_demand_config) => {
                let challenge_type = on_demand_config.challenge_type.clone();

                // Store on-demand config for later use by the background task
                task_state
                    .on_demand_configs
                    .blocking_write()
                    .push(on_demand_config.clone_for_state());

                // Install a placeholder resolver for on-demand mode
                let certified_key_lock = Arc::new(RwLock::new(None));
                let tls_alpn_resolvers = if challenge_type == ChallengeType::TlsAlpn01 {
                    Some(task_state.tls_alpn_01_resolvers.clone())
                } else {
                    None
                };

                let alpn_protocols = ctx.alpn.clone().unwrap_or_default();

                // Parse OCSP and ticket key configuration
                let ocsp_config = ferron_tls::config::OcspConfig::from_config(ctx.config);
                let ocsp_handle = crate::resolver::get_ocsp_handle_if_enabled(&ocsp_config);
                let ticketer = ferron_tls::builder::build_ticketer(ctx.config);

                let acme_resolver = TcpTlsAcmeResolver::new(
                    certified_key_lock,
                    tls_alpn_resolvers,
                    alpn_protocols,
                    ocsp_config,
                    ocsp_handle,
                    ticketer,
                );
                ctx.resolver = Some(Arc::new(acme_resolver));
            }
        }

        Ok(())
    }
}

/// The ACME TLS module that spawns the background provisioning task.
pub struct TlsAcmeModule {
    task_state: Arc<AcmeTaskState>,
}

impl TlsAcmeModule {
    fn new(task_state: Arc<AcmeTaskState>) -> Self {
        Self { task_state }
    }
}

impl Module for TlsAcmeModule {
    fn name(&self) -> &str {
        "tls-acme"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn start(&self, runtime: &mut Runtime) -> Result<(), Box<dyn std::error::Error>> {
        let configs_guard = self.task_state.configs.blocking_read();
        let configs_count = configs_guard.len();
        if configs_count == 0 {
            return Ok(());
        }

        let domains: Vec<_> = configs_guard
            .iter()
            .flat_map(|c| c.domains.iter())
            .cloned()
            .collect();
        drop(configs_guard);

        let event_sink = self.task_state.event_sink.clone();
        emit_log(
            &event_sink,
            LogLevel::Info,
            &format!(
                "ACME background task started with {} configuration(s) for domains: {}",
                configs_count,
                domains.join(", ")
            ),
            "ferron_tls_acme",
        );

        // Clone all state needed for the background task
        let state = self.task_state.clone();
        let on_demand_configs = state.on_demand_configs.blocking_read().clone();
        let memory_account_cache = state.memory_account_cache.clone();
        let on_demand_rx = state.on_demand_rx.clone();
        let configs = state.configs.clone();
        let sni_resolver_lock = state.sni_resolver_lock.clone();
        let tls_alpn_01_resolvers = state.tls_alpn_01_resolvers.clone();
        let http_01_resolvers = state.http_01_resolvers.clone();
        let event_sink = state.event_sink.clone();

        runtime.spawn_secondary_task(async move {
            run_acme_background_task(
                configs,
                on_demand_rx,
                on_demand_configs,
                memory_account_cache,
                sni_resolver_lock,
                tls_alpn_01_resolvers,
                http_01_resolvers,
                event_sink,
            )
            .await;
        });

        Ok(())
    }
}

/// Runs the ACME provisioning loop for both eager and on-demand configs.
#[allow(clippy::too_many_arguments)]
async fn run_acme_background_task(
    configs: Arc<RwLock<Vec<crate::config::AcmeConfig>>>,
    on_demand_rx: async_channel::Receiver<OnDemandRequest>,
    on_demand_configs: Vec<crate::config::AcmeOnDemandConfigData>,
    memory_account_cache: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    sni_resolver_lock: Arc<RwLock<HashMap<String, Arc<dyn rustls::server::ResolvesServerCert>>>>,
    tls_alpn_01_resolvers: Arc<RwLock<Vec<crate::challenge::TlsAlpn01DataLock>>>,
    http_01_resolvers: Arc<RwLock<Vec<crate::challenge::Http01DataLock>>>,
    event_sink: Arc<ferron_observability::CompositeEventSink>,
) {
    // Track which (hostname, port) combinations we've already processed
    let mut existing_combinations = std::collections::HashSet::new();

    // Pre-populate with eager configs that have domains
    {
        let configs_guard = configs.read().await;
        for config in configs_guard.iter() {
            for domain in &config.domains {
                existing_combinations.insert((domain.clone(), 443));
            }
        }
    }

    emit_log(
        &event_sink,
        LogLevel::Debug,
        "ACME provisioning cycle started",
        "ferron_tls_acme",
    );

    // Main provisioning loop
    loop {
        // Try to receive on-demand requests (non-blocking check first)
        if let Ok((sni_hostname, port)) = on_demand_rx.try_recv() {
            emit_log(
                &event_sink,
                LogLevel::Info,
                &format!("On-demand certificate requested for SNI {sni_hostname}:{port}"),
                "ferron_tls_acme",
            );
            emit_metric(
                &event_sink,
                "ferron.acme.on_demand_requests_total",
                MetricValue::U64(1),
                MetricType::Counter,
                Some("{request}"),
                Some("Total on-demand certificate requests"),
                vec![],
            );

            if !existing_combinations.contains(&(sni_hostname.clone(), port)) {
                existing_combinations.insert((sni_hostname.clone(), port));

                // Find matching on-demand config and convert to eager config
                for on_demand_data in &on_demand_configs {
                    if on_demand_data.port == port {
                        if let Some(ref pattern) = on_demand_data.sni_hostname {
                            if crate::on_demand::match_hostname(pattern, &sni_hostname) {
                                let _ = crate::on_demand::add_domain_to_cache(
                                    port,
                                    Some(pattern),
                                    &on_demand_data.cache_path,
                                    &sni_hostname,
                                )
                                .await;

                                let acme_config = crate::on_demand::convert_on_demand_config(
                                    on_demand_data,
                                    sni_hostname.clone(),
                                    memory_account_cache.clone(),
                                    &sni_resolver_lock,
                                    &tls_alpn_01_resolvers,
                                    &http_01_resolvers,
                                )
                                .await;

                                configs.write().await.push(acme_config);
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Provision certificates for all eager configs
        {
            let mut configs_guard = configs.write().await;
            emit_log(
                &event_sink,
                LogLevel::Debug,
                &format!(
                    "ACME provisioning cycle started — checking {} configurations",
                    configs_guard.len()
                ),
                "ferron_tls_acme",
            );

            for config in configs_guard.iter_mut() {
                if config.domains.is_empty() {
                    continue;
                }

                let domains = config.domains.join(", ");
                let challenge_type = format!("{:?}", config.challenge_type).to_lowercase();

                match crate::provision::provision_certificate(config).await {
                    Ok(()) => {
                        emit_log(
                            &event_sink,
                            LogLevel::Info,
                            &format!("ACME certificate issued for domains: {domains}"),
                            "ferron_tls_acme",
                        );
                        emit_metric(
                            &event_sink,
                            "ferron.acme.certificates_issued_total",
                            MetricValue::U64(1),
                            MetricType::Counter,
                            Some("{certificate}"),
                            Some("Total ACME certificate issuance outcomes"),
                            vec![
                                (
                                    "ferron.acme.status",
                                    MetricAttributeValue::StaticStr("success"),
                                ),
                                (
                                    "ferron.acme.challenge_type",
                                    MetricAttributeValue::String(challenge_type),
                                ),
                            ],
                        );
                    }
                    Err(e) => {
                        emit_log(
                            &event_sink,
                            LogLevel::Warn,
                            &format!("ACME certificate provisioning error for {domains}: {e}"),
                            "ferron_tls_acme",
                        );
                        emit_metric(
                            &event_sink,
                            "ferron.acme.certificates_issued_total",
                            MetricValue::U64(1),
                            MetricType::Counter,
                            Some("{certificate}"),
                            Some("Total ACME certificate issuance outcomes"),
                            vec![
                                (
                                    "ferron.acme.status",
                                    MetricAttributeValue::StaticStr("error"),
                                ),
                                (
                                    "ferron.acme.challenge_type",
                                    MetricAttributeValue::String(challenge_type),
                                ),
                            ],
                        );
                    }
                }
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }
}

/// Helper to emit log events through the event sink.
fn emit_log(
    event_sink: &Arc<ferron_observability::CompositeEventSink>,
    level: LogLevel,
    message: &str,
    target: &'static str,
) {
    event_sink.emit(Event::Log(LogEvent {
        level,
        message: message.to_string(),
        target,
    }));
}

/// Helper to emit metric events through the event sink.
fn emit_metric(
    event_sink: &Arc<ferron_observability::CompositeEventSink>,
    name: &'static str,
    value: MetricValue,
    ty: MetricType,
    unit: Option<&'static str>,
    description: Option<&'static str>,
    attributes: Vec<(&'static str, MetricAttributeValue)>,
) {
    event_sink.emit(Event::Metric(MetricEvent {
        name,
        attributes,
        ty,
        value,
        unit,
        description,
    }));
}

/// Module loader for the ACME TLS provider.
#[derive(Clone, Default)]
pub struct TlsAcmeModuleLoader;

static MODULE_CACHE: std::sync::OnceLock<Arc<TlsAcmeModule>> = std::sync::OnceLock::new();

impl ModuleLoader for TlsAcmeModuleLoader {
    fn register_providers(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry.with_provider::<TcpTlsContext<'_>, _>(|| Arc::new(TcpTlsAcmeProvider))
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry.with_stage::<ferron_http::HttpContext, _>(|| {
            Arc::new(stages::http01_stage::AcmeHttp01ChallengeStage)
        })
    }

    fn register_modules(
        &mut self,
        registry: Arc<ferron_core::registry::Registry>,
        modules: &mut Vec<Arc<dyn ferron_core::Module>>,
        config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Store the global registry for later resolution of DNS providers
        // from nested dns { } blocks in TLS configurations.
        GLOBAL_REGISTRY.set(registry.clone()).ok();

        // Build the composite event sink from observability providers
        let event_sink = build_composite_sink(&registry, &config.global_config)?;
        set_event_sink(event_sink);

        // Create and cache the module — the actual task spawning happens in start()
        if MODULE_CACHE.get().is_none() {
            let task_state = get_or_init_task_state();
            let module = Arc::new(TlsAcmeModule::new(task_state));
            MODULE_CACHE.set(module.clone()).ok();
            modules.push(module);
        }

        Ok(())
    }
}

/// Resolve a DNS client from a nested `dns { ... }` block inside the TLS config.
///
/// The block should contain a `provider` directive naming the DNS provider,
/// along with any provider-specific configuration.
///
/// # Example
///
/// ```text
/// tls {
///     provider "acme"
///     challenge dns-01
///     dns {
///         provider "cloudflare"
///         api_token "xxx"
///     }
/// }
/// ```
fn resolve_dns_client_from_config(
    config: &ferron_core::config::ServerConfigurationBlock,
) -> Result<Option<Arc<dyn DnsClient>>, Box<dyn std::error::Error + 'static>> {
    // Look for nested dns { ... } block
    let Some(dns_entries) = config.directives.get("dns") else {
        return Ok(None);
    };
    let Some(dns_entry) = dns_entries.first() else {
        return Ok(None);
    };
    let Some(dns_block) = dns_entry.children.as_ref() else {
        return Ok(None);
    };

    // Get the provider name from the dns block
    let provider_name = dns_block
        .get_value("provider")
        .and_then(|v| v.as_string_with_interpolations(&std::collections::HashMap::new()))
        .ok_or_else(|| anyhow::anyhow!("DNS provider name not specified."))?;

    // Look up the DNS provider registry from the stored global registry
    let global_registry = GLOBAL_REGISTRY
        .get()
        .ok_or_else(|| anyhow::anyhow!("DNS provider registry not initialized."))?;
    // SAFETY: The ProviderRegistry stores provider factories (closures), not
    // references to any DnsContext. The lifetime on DnsContext is only relevant
    // during execute(), where the provider borrows the config temporarily.
    // We transmute the lifetime to 'static so we can call execute with any config block.
    let dns_registry: Arc<ProviderRegistry<DnsContext<'static>>> = unsafe {
        std::mem::transmute(
            global_registry
                .get_provider_registry::<DnsContext<'_>>()
                .ok_or_else(|| {
                    anyhow::anyhow!("DNS provider registry not found for ACME DNS-01 challenge.")
                })?,
        )
    };
    let provider = dns_registry.get(&provider_name).ok_or_else(|| {
        anyhow::anyhow!(
            "DNS provider not found for ACME DNS-01 challenge (provider: {provider_name})."
        )
    })?;

    // Execute the provider with the dns block as config to get the client.
    // SAFETY: The provider only borrows dns_block during execute() and does not
    // store the reference. The returned Arc<dyn DnsClient> is 'static.
    let mut dns_ctx: DnsContext<'static> = unsafe {
        std::mem::transmute::<DnsContext<'_>, DnsContext<'static>>(DnsContext {
            config: dns_block,
            client: None,
        })
    };
    provider
        .execute(&mut dns_ctx)
        .map_err(|e| anyhow::anyhow!("Error initializing '{provider_name}' DNS provider: {e}"))?;
    Ok(Some(dns_ctx.client.ok_or(anyhow::anyhow!(
        "No DNS client configured for ACME DNS-01 challenge (provider: {provider_name})."
    ))?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_name() {
        assert_eq!(TcpTlsAcmeProvider.name(), "acme");
    }

    #[test]
    fn test_task_state_initialization() {
        let state = AcmeTaskState::new(Arc::new(ferron_observability::CompositeEventSink::new(
            Vec::new(),
        )));
        assert!(state.configs.blocking_read().is_empty());
        assert!(state.tls_alpn_01_resolvers.blocking_read().is_empty());
        assert!(state.http_01_resolvers.blocking_read().is_empty());
    }
}
