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
pub mod on_demand;
pub mod provision;
pub mod resolver;
pub mod stages;

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::loader::ModuleLoader;
use ferron_core::providers::Provider;
use ferron_core::registry::RegistryBuilder;
use ferron_core::{runtime::Runtime, Module};
use ferron_dns::DnsClient;
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
    /// DNS client lookup cache (provider name -> client).
    pub dns_clients: Arc<RwLock<HashMap<String, Arc<dyn DnsClient>>>>,
}

impl Default for AcmeTaskState {
    fn default() -> Self {
        Self::new()
    }
}

impl AcmeTaskState {
    pub fn new() -> Self {
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
            dns_clients: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

/// Global ACME task state, lazily initialized.
static ACME_TASK_STATE: std::sync::OnceLock<Arc<AcmeTaskState>> = std::sync::OnceLock::new();

fn get_or_init_task_state() -> Arc<AcmeTaskState> {
    ACME_TASK_STATE
        .get_or_init(|| Arc::new(AcmeTaskState::new()))
        .clone()
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

        let task_state = get_or_init_task_state();

        // Resolve DNS client if configured
        let dns_client = ctx
            .config
            .get_value("dns")
            .and_then(|v| v.as_string_with_interpolations(&std::collections::HashMap::new()))
            .and_then(|name| {
                let dns_clients = task_state.dns_clients.blocking_read();
                dns_clients.get(&name).cloned()
            });

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
                let ocsp_config = crate::resolver::parse_ocsp_config(ctx.config);
                let ocsp_handle = crate::resolver::get_ocsp_handle_if_enabled(&ocsp_config);
                let ticketer = crate::resolver::build_ticketer(ctx.config);

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
                let ocsp_config = crate::resolver::parse_ocsp_config(ctx.config);
                let ocsp_handle = crate::resolver::get_ocsp_handle_if_enabled(&ocsp_config);
                let ticketer = crate::resolver::build_ticketer(ctx.config);

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
        let configs_count = self.task_state.configs.blocking_read().len();
        if configs_count == 0 {
            return Ok(());
        }

        ferron_core::log_debug!(
            "ACME background task started with {} configuration(s)",
            configs_count
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

        runtime.spawn_secondary_task(async move {
            run_acme_background_task(
                configs,
                on_demand_rx,
                on_demand_configs,
                memory_account_cache,
                sni_resolver_lock,
                tls_alpn_01_resolvers,
                http_01_resolvers,
            )
            .await;
        });

        Ok(())
    }
}

/// Runs the ACME provisioning loop for both eager and on-demand configs.
async fn run_acme_background_task(
    configs: Arc<RwLock<Vec<crate::config::AcmeConfig>>>,
    on_demand_rx: async_channel::Receiver<OnDemandRequest>,
    on_demand_configs: Vec<crate::config::AcmeOnDemandConfigData>,
    memory_account_cache: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    sni_resolver_lock: Arc<RwLock<HashMap<String, Arc<dyn rustls::server::ResolvesServerCert>>>>,
    tls_alpn_01_resolvers: Arc<RwLock<Vec<crate::challenge::TlsAlpn01DataLock>>>,
    http_01_resolvers: Arc<RwLock<Vec<crate::challenge::Http01DataLock>>>,
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

    // Main provisioning loop
    loop {
        // Try to receive on-demand requests (non-blocking check first)
        if let Ok((sni_hostname, port)) = on_demand_rx.try_recv() {
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
            for config in configs_guard.iter_mut() {
                if config.domains.is_empty() {
                    continue;
                }
                if let Err(e) = crate::provision::provision_certificate(config).await {
                    ferron_core::log_warn!("ACME certificate provisioning error: {}", e);
                }
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }
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
        _config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Collect DNS clients from the DNS provider registry
        if let Some(dns_registry) = registry.get_provider_registry::<ferron_dns::DnsContext<'_>>() {
            let task_state = get_or_init_task_state();
            let mut dns_clients = task_state.dns_clients.blocking_write();
            for provider in dns_registry.get_all() {
                // Execute the provider to get the DNS client
                // We use a static empty config since we only need the client instance
                static DUMMY_CONFIG: std::sync::OnceLock<
                    ferron_core::config::ServerConfigurationBlock,
                > = std::sync::OnceLock::new();
                let dummy_config =
                    DUMMY_CONFIG.get_or_init(|| ferron_core::config::ServerConfigurationBlock {
                        directives: Arc::new(HashMap::new()),
                        matchers: HashMap::new(),
                        span: None,
                    });

                let mut dns_ctx = ferron_dns::DnsContext {
                    config: dummy_config,
                    client: None,
                };
                let _ = provider.execute(&mut dns_ctx);
                if let Some(client) = dns_ctx.client {
                    dns_clients.insert(provider.name().to_string(), client);
                }
            }
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_name() {
        assert_eq!(TcpTlsAcmeProvider.name(), "acme");
    }

    #[test]
    fn test_task_state_initialization() {
        let state = AcmeTaskState::new();
        assert!(state.configs.blocking_read().is_empty());
        assert!(state.tls_alpn_01_resolvers.blocking_read().is_empty());
        assert!(state.http_01_resolvers.blocking_read().is_empty());
    }
}
