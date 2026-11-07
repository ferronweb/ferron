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
//!         provider acme
//!         challenge http-01
//!         contact "admin@example.com"
//!     }
//!     root "/var/www/example.com"
//! }
//! ```

pub mod cache;
pub mod challenge;
pub mod challenge_sync;
pub mod config;
pub mod errors;
pub mod on_demand;
pub mod provision;
pub mod resolver;
pub mod stages;
mod validator;

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::loader::ModuleLoader;
use ferron_core::providers::Provider;
use ferron_core::registry::{ProviderRegistry, RegistryBuilder, GLOBAL_REGISTRY};
use ferron_core::runtime::Runtime;
use ferron_core::{config_validator_scoped_key, Module};
use ferron_dns::{DnsClient, DnsContext};
use ferron_observability::{
    build_composite_sink, CompositeEventSink, Event, LogAttributeValue, LogEvent, LogLevel,
    MetricAttributeValue, MetricEvent, MetricType, MetricValue,
};
use ferron_tls::TlsContext;
use futures_util::StreamExt;
use instant_acme::ChallengeType;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::config::{parse_acme_config, AcmeConfigOrOnDemand, SniResolverLock};
use crate::on_demand::{check_ask_endpoint, get_cached_domains, OnDemandRequest};
use crate::resolver::{AcmeResolverInner, TlsAcmeResolver};

/// Shared runtime state for the ACME module.
///
/// Configuration-scoped state lives in [`AcmeGeneration`]. A configuration
/// load builds a staging generation; [`TlsAcmeModule::start`] promotes it and
/// only then retires the previous generation. A failed load therefore leaves
/// the active generation and its background task untouched.
pub struct AcmeTaskState {
    /// Generation currently being built by the configuration loader.
    staging: parking_lot::Mutex<Option<Arc<AcmeGeneration>>>,
    /// Generation whose background task is currently running.
    active: arc_swap::ArcSwapOption<AcmeGeneration>,
    /// Handles of the running background task, used when it is retired.
    active_task: parking_lot::Mutex<Option<ActiveTask>>,
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
    /// Event sink for observability.
    pub event_sink: Arc<parking_lot::RwLock<Option<Arc<ferron_observability::CompositeEventSink>>>>,
}

/// One configuration generation's ACME state.
///
/// A reload builds a new generation before the previous one is retired, so the
/// two never share configuration-scoped state during the transition.
struct AcmeGeneration {
    /// Token identifying the configuration load this generation belongs to.
    reload_token: Arc<CancellationToken>,
    /// ACME configs (both eager and dynamically added on-demand).
    configs: Arc<RwLock<Vec<crate::config::AcmeConfig>>>,
    /// On-demand configurations for lazy certificate issuance.
    on_demand_configs: Arc<RwLock<Vec<crate::config::AcmeOnDemandConfigData>>>,
    /// SNI resolvers populated by on-demand issuance.
    sni_resolver_lock: SniResolverLock,
    /// Challenge cache directories (file-backed `cache` paths).
    ///
    /// Peers publish HTTP-01 / TLS-ALPN-01 challenge files here so any node
    /// behind a load balancer can answer CA validation. Populated from each
    /// config's file `cache` path (deduplicated). Empty when only in-memory
    /// caching is used.
    challenge_cache_dirs: Arc<RwLock<Vec<std::path::PathBuf>>>,
}

impl AcmeGeneration {
    fn new(reload_token: Arc<CancellationToken>) -> Self {
        Self {
            reload_token,
            configs: Arc::new(RwLock::new(Vec::new())),
            on_demand_configs: Arc::new(RwLock::new(Vec::new())),
            sni_resolver_lock: Arc::new(RwLock::new(HashMap::new())),
            challenge_cache_dirs: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Registers a file cache directory for distributed challenge fallback (deduplicated).
    fn register_challenge_cache_dir(&self, dir: std::path::PathBuf) {
        let mut dirs = self.challenge_cache_dirs.blocking_write();
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
}

/// Handles for one running ACME background task.
struct ActiveTask {
    generation: Arc<AcmeGeneration>,
    /// Hard cancel, used when draining takes too long.
    cancel: CancellationToken,
    /// Graceful drain: stop starting new orders.
    drain: CancellationToken,
    /// Cancelled when the background task exits.
    done: CancellationToken,
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
            staging: parking_lot::Mutex::new(None),
            active: arc_swap::ArcSwapOption::empty(),
            active_task: parking_lot::Mutex::new(None),
            on_demand_tx: tx,
            on_demand_rx: rx,
            tls_alpn_01_resolvers: Arc::new(RwLock::new(Vec::new())),
            http_01_resolvers: Arc::new(RwLock::new(Vec::new())),
            memory_account_cache: Arc::new(RwLock::new(HashMap::new())),
            event_sink: Arc::new(parking_lot::RwLock::new(None)),
        }
    }

    fn set_event_sink(&self, event_sink: Arc<ferron_observability::CompositeEventSink>) {
        *self.event_sink.write() = Some(event_sink);
    }

    /// The generation for the configuration load currently in progress.
    fn staging_generation(&self) -> Arc<AcmeGeneration> {
        self.staging_generation_for(ferron_core::shutdown::RELOAD_TOKEN.load_full())
    }

    fn staging_generation_for(&self, reload_token: Arc<CancellationToken>) -> Arc<AcmeGeneration> {
        let mut staging = self.staging.lock();
        if let Some(generation) = staging.as_ref() {
            if Arc::ptr_eq(&generation.reload_token, &reload_token) {
                return generation.clone();
            }
        }

        let generation = Arc::new(AcmeGeneration::new(reload_token));
        *staging = Some(generation.clone());
        generation
    }

    /// Promotes the staging generation of the current load attempt.
    ///
    /// If no staging generation was built for this attempt (for example because
    /// all ACME configuration was removed), an empty generation is promoted.
    fn promote_generation(&self) -> Arc<AcmeGeneration> {
        self.promote_generation_for(ferron_core::shutdown::RELOAD_TOKEN.load_full())
    }

    fn promote_generation_for(&self, reload_token: Arc<CancellationToken>) -> Arc<AcmeGeneration> {
        let mut staging = self.staging.lock();
        match staging.take() {
            Some(generation) if Arc::ptr_eq(&generation.reload_token, &reload_token) => generation,
            _ => Arc::new(AcmeGeneration::new(reload_token)),
        }
    }

    /// Installs a new active generation and returns the previous task, if any.
    fn install_active(&self, task: ActiveTask) -> Option<ActiveTask> {
        let mut slot = self.active_task.lock();
        self.active.store(Some(task.generation.clone()));
        slot.replace(task)
    }

    /// Installs an active generation without a background task.
    fn install_empty_active(&self, generation: Arc<AcmeGeneration>) -> Option<ActiveTask> {
        let mut slot = self.active_task.lock();
        self.active.store(Some(generation));
        slot.take()
    }

    /// Challenge cache directories of the active generation.
    async fn active_challenge_cache_dirs(&self) -> Vec<std::path::PathBuf> {
        match self.active.load_full() {
            Some(generation) => generation.challenge_cache_dirs.read().await.clone(),
            None => Vec::new(),
        }
    }

    /// Drops challenge data locks that no live config refers to any more.
    async fn prune_challenge_locks(&self) {
        self.tls_alpn_01_resolvers
            .write()
            .await
            .retain(|lock| Arc::strong_count(lock) > 1);
        self.http_01_resolvers
            .write()
            .await
            .retain(|lock| Arc::strong_count(lock) > 1);
    }
}

/// Retires a previous generation's background task without blocking the caller.
fn spawn_retirement(runtime: &Runtime, state: Arc<AcmeTaskState>, previous: ActiveTask) {
    runtime.spawn_secondary_task(async move {
        // Give the previous generation's in-flight order a chance to finish.
        // A hung order is cancelled after `RELOAD_DRAIN_TIMEOUT`.
        previous.drain.cancel();
        if tokio::time::timeout(RELOAD_DRAIN_TIMEOUT, previous.done.cancelled())
            .await
            .is_err()
        {
            previous.cancel.cancel();
            previous.done.cancelled().await;
        }

        // Drop the generation before pruning so only challenge data still
        // referenced by live configs survives.
        drop(previous.generation);
        state.prune_challenge_locks().await;
    });
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
        let state = self
            .inner
            .get_or_init(|| Arc::new(AcmeTaskState::new()))
            .clone();
        if let Some(event_sink) = self.event_sink.lock().clone() {
            state.set_event_sink(event_sink);
        }
        state
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
/// Implements `Provider<TlsContext>` to handle `tls { provider acme; ... }` blocks.
pub struct TlsAcmeProvider;

impl Provider<TlsContext<'_>> for TlsAcmeProvider {
    fn name(&self) -> &str {
        "acme"
    }

    fn execute(&self, ctx: &mut TlsContext) -> Result<(), Box<dyn std::error::Error>> {
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
        let generation = task_state.staging_generation();

        let acme_result = parse_acme_config(
            ctx.config,
            &domain,
            port,
            task_state.memory_account_cache.clone(),
            task_state.tls_alpn_01_resolvers.clone(),
            task_state.http_01_resolvers.clone(),
            generation.sni_resolver_lock.clone(),
            dns_client,
        )
        .map_err(|e| format!("Failed to parse ACME config: {e}"))?;

        let tls_config = ferron_tls::config::TlsServerConfig::from_config(ctx.config)
            .map_err(|e| std::io::Error::other(format!("Invalid TLS configuration: {e}")))?;
        let tls_config_builder = ferron_tls::builder::build_server_config_builder(
            &tls_config.crypto,
            &tls_config.client_auth,
        )?;

        match acme_result {
            AcmeConfigOrOnDemand::Eager(acme_config) => {
                let certified_key_lock = acme_config.certified_key_lock.clone();
                let challenge_type = acme_config.challenge_type.clone();
                let error_message = acme_config.error_message.clone();
                // Register the file cache dir so peers' challenge files are visible.
                if let Some(dir) = acme_config
                    .account_cache
                    .cache_dir()
                    .or_else(|| acme_config.certificate_cache.cache_dir())
                {
                    generation.register_challenge_cache_dir(dir);
                }

                // Add to configs list
                generation.configs.blocking_write().push(acme_config);

                let tls_alpn_resolvers = if challenge_type == ChallengeType::TlsAlpn01 {
                    Some(task_state.tls_alpn_01_resolvers.clone())
                } else {
                    None
                };

                let alpn_protocols = ctx.alpn.clone().unwrap_or_default();

                let ocsp_config = ferron_tls::config::OcspConfig::from_config(ctx.config);
                let ocsp_handle = crate::resolver::get_ocsp_handle_if_enabled(&ocsp_config);
                let ticketer = ferron_tls::builder::build_ticketer(ctx.config);

                let acme_resolver = TlsAcmeResolver::new(
                    tls_config_builder,
                    AcmeResolverInner::Eager(certified_key_lock),
                    tls_alpn_resolvers,
                    alpn_protocols,
                    ocsp_config,
                    ocsp_handle,
                    ticketer,
                    None,
                    error_message,
                    generation.challenge_cache_dirs.clone(),
                );

                ctx.resolver = Some(Arc::new(acme_resolver));
            }
            AcmeConfigOrOnDemand::OnDemand(on_demand_config) => {
                let sni_resolver_lock = on_demand_config.sni_resolver_lock.clone();
                let challenge_type = on_demand_config.challenge_type.clone();
                let error_message = on_demand_config.error_message.clone();
                if let Some(ref dir) = on_demand_config.cache_path {
                    generation.register_challenge_cache_dir(dir.clone());
                }

                // Store on-demand config for later use by the background task
                generation
                    .on_demand_configs
                    .blocking_write()
                    .push(on_demand_config.clone_for_state());

                let tls_alpn_resolvers = if challenge_type == ChallengeType::TlsAlpn01 {
                    Some(task_state.tls_alpn_01_resolvers.clone())
                } else {
                    None
                };

                let alpn_protocols = ctx.alpn.clone().unwrap_or_default();

                let ocsp_config = ferron_tls::config::OcspConfig::from_config(ctx.config);
                let ocsp_handle = crate::resolver::get_ocsp_handle_if_enabled(&ocsp_config);
                let ticketer = ferron_tls::builder::build_ticketer(ctx.config);

                let acme_resolver = TlsAcmeResolver::new(
                    tls_config_builder,
                    AcmeResolverInner::OnDemand(sni_resolver_lock),
                    tls_alpn_resolvers,
                    alpn_protocols,
                    ocsp_config,
                    ocsp_handle,
                    ticketer,
                    Some((task_state.on_demand_tx.clone(), on_demand_config.port)),
                    error_message,
                    generation.challenge_cache_dirs.clone(),
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
        let generation = self.task_state.promote_generation();
        let configs = generation.configs.clone();
        let configs_guard = configs.blocking_read();
        let configs_count = configs_guard.len();
        let on_demand_empty = generation.on_demand_configs.blocking_read().is_empty();
        let domains: Vec<_> = configs_guard
            .iter()
            .flat_map(|c| c.domains.iter())
            .cloned()
            .collect();
        drop(configs_guard);

        if configs_count == 0 && on_demand_empty {
            // No eager or on-demand configs. Retire any task left over from the
            // previous generation and make the empty generation visible to
            // challenge handling.
            if let Some(previous) = self.task_state.install_empty_active(generation) {
                spawn_retirement(runtime, self.task_state.clone(), previous);
            }
            return Ok(());
        }

        let event_sink = self
            .task_state
            .event_sink
            .read()
            .clone()
            .unwrap_or(Arc::new(CompositeEventSink::new(vec![])));
        emit_log(
            &event_sink,
            LogLevel::Info,
            "ACME background task started",
            &format!(
                "ACME background task started with {} configuration(s) for domains: {}",
                configs_count,
                domains.join(", ")
            ),
            "ferron-tls-acme",
            vec![
                (
                    "ferron.acme.config_count",
                    LogAttributeValue::I64(configs_count as i64),
                ),
                (
                    "ferron.acme.domains",
                    LogAttributeValue::String(domains.join(", ")),
                ),
            ],
        );

        // Clone all state needed for the background task
        let on_demand_configs = generation.on_demand_configs.blocking_read().clone();
        let memory_account_cache = self.task_state.memory_account_cache.clone();
        let on_demand_rx = self.task_state.on_demand_rx.clone();
        let sni_resolver_lock = generation.sni_resolver_lock.clone();
        let tls_alpn_01_resolvers = self.task_state.tls_alpn_01_resolvers.clone();
        let http_01_resolvers = self.task_state.http_01_resolvers.clone();
        let challenge_cache_dirs = generation.challenge_cache_dirs.clone();

        let cancel_token = CancellationToken::new();
        let drain_token = CancellationToken::new();
        let done_token = CancellationToken::new();
        let done_guard = done_token.clone();
        let task_cancel = cancel_token.clone();

        // Install the new generation before retiring the old one, so the old
        // generation stays fully functional if the reload failed.
        let previous = self.task_state.install_active(ActiveTask {
            generation,
            cancel: cancel_token,
            drain: drain_token.clone(),
            done: done_token,
        });

        runtime.spawn_secondary_task(async move {
            // Tells the retirement watcher the task has ended, even by panicking.
            let _done = done_guard.drop_guard();
            task_cancel
                .run_until_cancelled(run_acme_background_task(
                    configs,
                    on_demand_rx,
                    on_demand_configs,
                    memory_account_cache,
                    sni_resolver_lock,
                    tls_alpn_01_resolvers,
                    http_01_resolvers,
                    challenge_cache_dirs,
                    drain_token,
                    event_sink,
                ))
                .await;
        });

        if let Some(previous) = previous {
            spawn_retirement(runtime, self.task_state.clone(), previous);
        }

        Ok(())
    }
}

/// How long an order in progress may run on after a reload before it's cancelled.
const RELOAD_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// How many certificates the background task provisions at once.
const MAX_CONCURRENT_ORDERS: usize = 4;

/// Runs the ACME provisioning loop for both eager and on-demand configs.
///
/// Once `drain` is cancelled (on reload), it starts no new orders and returns
/// after the one in progress.
#[allow(clippy::too_many_arguments)]
async fn run_acme_background_task(
    configs: Arc<RwLock<Vec<crate::config::AcmeConfig>>>,
    on_demand_rx: async_channel::Receiver<OnDemandRequest>,
    on_demand_configs: Vec<crate::config::AcmeOnDemandConfigData>,
    memory_account_cache: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    sni_resolver_lock: Arc<RwLock<HashMap<String, Arc<dyn rustls::server::ResolvesServerCert>>>>,
    tls_alpn_01_resolvers: Arc<RwLock<Vec<crate::challenge::TlsAlpn01DataLock>>>,
    http_01_resolvers: Arc<RwLock<Vec<crate::challenge::Http01DataLock>>>,
    challenge_cache_dirs: Arc<RwLock<Vec<std::path::PathBuf>>>,
    drain: CancellationToken,
    event_sink: Arc<ferron_observability::CompositeEventSink>,
) {
    // Track which (hostname, port) combinations we've already processed
    let mut existing_combinations = std::collections::HashSet::new();

    // Insert cached on-demand config domains
    for config in &on_demand_configs {
        let domains = get_cached_domains(
            config.port,
            config.sni_hostname.as_deref(),
            &config.cache_path,
        )
        .await;

        for domain in domains {
            emit_log(
                &event_sink,
                LogLevel::Info,
                "On-demand certificate pre-loaded",
                &format!(
                    "On-demand certificate pre-loaded for SNI {domain}:{}",
                    config.port
                ),
                "ferron-tls-acme",
                vec![
                    ("tls.sni", LogAttributeValue::String(domain.clone())),
                    ("tls.port", LogAttributeValue::I64(config.port as i64)),
                ],
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

            let acme_config = crate::on_demand::convert_on_demand_config(
                config,
                domain,
                memory_account_cache.clone(),
                &sni_resolver_lock,
                &tls_alpn_01_resolvers,
                &http_01_resolvers,
            )
            .await;

            configs.write().await.push(acme_config);
        }
    }

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
        "ACME provisioning cycle started",
        "ferron-tls-acme",
        Vec::new(),
    );

    {
        use rustls_pki_types::pem::PemObject;

        for config in configs.read().await.iter() {
            // Install TLS certificates from cache early
            let certificate_cache_key = crate::cache::get_certificate_cache_key(config);
            if let Some(serialized_data) =
                config.certificate_cache.get(&certificate_cache_key).await
            {
                if let Ok(data) =
                    serde_json::from_slice::<crate::cache::CertificateCacheData>(&serialized_data)
                {
                    if let Ok(certs) = rustls_pki_types::CertificateDer::pem_slice_iter(
                        data.certificate_chain_pem.as_bytes(),
                    )
                    .collect::<Result<Vec<_>, _>>()
                    {
                        if let Ok(private_key) = rustls_pki_types::PrivateKeyDer::from_pem_slice(
                            data.private_key_pem.as_bytes(),
                        ) {
                            // If installation fails, continue anyway.
                            // This is early and best-effort certificate installation from cache...
                            let _ = crate::provision::cert_install::install_certified_key(
                                config,
                                certs,
                                private_key,
                                &data,
                                &event_sink,
                            )
                            .await;
                        }
                    }
                }
            }
        }
    }

    // Don't spawn on-demand request loop task when config struct is empty
    if !on_demand_configs.is_empty() {
        let event_sink2 = event_sink.clone();
        let configs2 = configs.clone();
        let drain2 = drain.clone();
        tokio::spawn(async move {
            // On-demand request loop; after a reload, the next generation's
            // loop receives the requests.
            while let Some(Ok((sni_hostname, port))) =
                drain2.run_until_cancelled(on_demand_rx.recv()).await
            {
                emit_log(
                    &event_sink2,
                    LogLevel::Info,
                    "On-demand certificate requested",
                    &format!("On-demand certificate requested for SNI {sni_hostname}:{port}"),
                    "ferron-tls-acme",
                    vec![
                        ("tls.sni", LogAttributeValue::String(sni_hostname.clone())),
                        ("tls.port", LogAttributeValue::I64(port as i64)),
                    ],
                );
                emit_metric(
                    &event_sink2,
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
                                    match check_ask_endpoint(
                                        &sni_hostname,
                                        on_demand_data.on_demand_ask.as_deref(),
                                        on_demand_data.on_demand_ask_auth.as_deref(),
                                        on_demand_data.on_demand_ask_no_verification,
                                    )
                                    .await
                                    {
                                        Ok(true) => (),
                                        Ok(false) => {
                                            emit_log(
                                                &event_sink2,
                                                LogLevel::Error,
                                                "Certificate issuance denied",
                                                &format!(
                                                "The TLS certificate cannot be issued for \"{}\" \
                                                hostname",
                                                sni_hostname
                                            ),
                                                "ferron-tls-acme",
                                                vec![(
                                                    "tls.sni",
                                                    LogAttributeValue::String(sni_hostname.clone()),
                                                )],
                                            );

                                            continue;
                                        }
                                        Err(err) => {
                                            emit_log(
                                                &event_sink2,
                                                LogLevel::Error,
                                                "Ask endpoint error",
                                                &format!(
                                                "Error while determining if the TLS certificate \
                                                can be issued for \"{}\" hostname: {err}",
                                                sni_hostname
                                            ),
                                                "ferron-tls-acme",
                                                vec![
                                                    (
                                                        "tls.sni",
                                                        LogAttributeValue::String(
                                                            sni_hostname.clone(),
                                                        ),
                                                    ),
                                                    (
                                                        "error.message",
                                                        LogAttributeValue::String(err.to_string()),
                                                    ),
                                                ],
                                            );

                                            continue;
                                        }
                                    }

                                    let _ = crate::on_demand::add_domain_to_cache(
                                        port,
                                        Some(pattern),
                                        &on_demand_data.cache_path,
                                        &sni_hostname,
                                    )
                                    .await;

                                    // After a reload, leave the hostname to the next
                                    // generation, which loads it from the domain cache
                                    // or receives the next request for it.
                                    if drain2.is_cancelled() {
                                        break;
                                    }

                                    let acme_config = crate::on_demand::convert_on_demand_config(
                                        on_demand_data,
                                        sni_hostname.clone(),
                                        memory_account_cache.clone(),
                                        &sni_resolver_lock,
                                        &tls_alpn_01_resolvers,
                                        &http_01_resolvers,
                                    )
                                    .await;

                                    configs2.write().await.push(acme_config);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    // Sleep for 2ms to ensure configurations are loaded
    tokio::time::sleep(tokio::time::Duration::from_millis(2)).await;

    let mut old_config_count = 0;

    // Main provisioning loop
    loop {
        // Provision certificates for all eager configs
        {
            let mut configs_guard = configs.write().await;
            if old_config_count != configs_guard.len() {
                old_config_count = configs_guard.len();

                emit_log(
                    &event_sink,
                    LogLevel::Debug,
                    "ACME provisioning cycle started",
                    &format!(
                        "ACME provisioning cycle started — checking {} configurations",
                        configs_guard.len()
                    ),
                    "ferron-tls-acme",
                    vec![(
                        "ferron.acme.config_count",
                        LogAttributeValue::I64(configs_guard.len() as i64),
                    )],
                );
            }

            futures_util::stream::iter(
                configs_guard
                    .iter_mut()
                    .filter(|config| !config.domains.is_empty()),
            )
            .for_each_concurrent(MAX_CONCURRENT_ORDERS, |config| async {
                if drain.is_cancelled() {
                    return;
                }
                provision_and_report(config, &event_sink).await;
            })
            .await;
        }

        // Drop expired peer challenge files so the shared dir doesn't grow
        // unboundedly when a node crashes mid-order (TTL is the backstop).
        for dir in challenge_cache_dirs.read().await.clone() {
            crate::challenge_sync::prune_expired_challenges(&dir).await;
        }

        // Stagger cycles with deterministic jitter (pid + time based, no `rand`
        // dependency) so nodes restarted together desynchronize and don't wake
        // in lockstep every 10s (thundering-herd damping; the lock is the
        // primary serialization).
        let jitter_ms = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() % 1000)
            .unwrap_or(0) as u64
            + std::process::id() as u64)
            % 2000;
        let cycle_delay = std::time::Duration::from_millis(10_000 + jitter_ms);
        if drain
            .run_until_cancelled(tokio::time::sleep(cycle_delay))
            .await
            .is_none()
        {
            return;
        }
    }
}

/// Provisions a certificate for one configuration and reports the outcome.
async fn provision_and_report(
    config: &mut crate::config::AcmeConfig,
    event_sink: &Arc<ferron_observability::CompositeEventSink>,
) {
    let domains = config.domains.join(", ");
    let challenge_type = format!("{:?}", config.challenge_type).to_lowercase();

    match crate::provision::provision_certificate(config, event_sink).await {
        Ok(true) => {
            emit_log(
                event_sink,
                LogLevel::Info,
                "ACME certificate issued",
                &format!("ACME certificate issued for domains: {domains}"),
                "ferron-tls-acme",
                vec![("ferron.acme.domains", LogAttributeValue::String(domains))],
            );
            emit_metric(
                event_sink,
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
        Ok(false) => {
            // Certificate has been already provisioned, skipping
        }
        Err(e) => {
            *config.error_message.write() = Some(format!(
                "ACME certificate provisioning error for {domains}: {e}"
            ));
            emit_log(
                event_sink,
                LogLevel::Warn,
                "ACME certificate provisioning error",
                &format!("ACME certificate provisioning error for {domains}: {e}"),
                "ferron-tls-acme",
                vec![
                    ("ferron.acme.domains", LogAttributeValue::String(domains)),
                    ("error.message", LogAttributeValue::String(e.to_string())),
                ],
            );
            emit_metric(
                event_sink,
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

/// Helper to emit log events through the event sink.
pub fn emit_log(
    event_sink: &Arc<ferron_observability::CompositeEventSink>,
    level: LogLevel,
    summary: &'static str,
    message: &str,
    target: &'static str,
    attributes: Vec<(&'static str, LogAttributeValue)>,
) {
    event_sink.emit(Event::Log(LogEvent {
        level,
        message: message.to_string(),
        summary: summary.into(),
        target,
        attributes,
        trace_context: None,
    }));
}

/// Helper to emit metric events through the event sink.
pub(crate) fn emit_metric(
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
        trace_context: None,
    }));
}

/// Module loader for the ACME TLS provider.
#[derive(Clone, Default)]
pub struct TlsAcmeModuleLoader;

impl ModuleLoader for TlsAcmeModuleLoader {
    fn register_providers(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry.with_provider::<TlsContext<'_>, _>(|| Arc::new(TlsAcmeProvider))
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

        let event_sink = build_composite_sink(&registry, &config.global_config, None)?;
        set_event_sink(event_sink);

        // The actual task spawning happens in start()
        let task_state = get_or_init_task_state();
        modules.push(Arc::new(TlsAcmeModule::new(task_state)));

        Ok(())
    }

    fn register_directives(&mut self, registry: &mut ferron_core::directives::DirectiveRegistry) {
        let tls = ferron_core::directives::DirectiveSubblock::custom("tls");
        register_acme_directive(
            registry,
            "challenge",
            "challenge <type>",
            "This directive specifies the ACME challenge type. Supported: http-01, dns-01.",
            tls,
        );
        register_acme_directive(registry, "contact", "contact <email>", "This directive specifies the ACME account contact email address for certificate expiry and account notices.", tls);
        register_acme_directive(
            registry,
            "directory",
            "directory <uri>",
            "This directive specifies the ACME directory URI.",
            tls,
        );
        register_acme_directive(
            registry,
            "profile",
            "profile <name>",
            "This directive specifies the ACME profile name.",
            tls,
        );
        register_acme_directive(
            registry,
            "eab",
            "eab <key_id> <mac_key>",
            "This directive specifies External Account Binding credentials for the ACME account.",
            tls,
        );
        register_acme_directive(
            registry,
            "cache",
            "cache <path>",
            "This directive specifies the directory path for the ACME certificate cache.",
            tls,
        );
        register_acme_directive(
            registry,
            "save",
            "save <path>...",
            "This directive specifies one or more paths to save obtained ACME certificates.",
            tls,
        );
        register_acme_directive(
            registry,
            "post_obtain_command",
            "post_obtain_command <command>",
            "This directive specifies a command to run after obtaining an ACME certificate.",
            tls,
        );
        register_acme_directive(
            registry,
            "no_verification",
            "no_verification [bool]",
            "This directive disables TLS certificate verification for the ACME directory.",
            tls,
        );
        register_acme_directive(registry, "on_demand", "on_demand [bool]", "This directive enables automatic certificate issuance on demand for unknown hostnames.", tls);
        register_acme_directive(registry, "on_demand_ask", "on_demand_ask <url>", "This directive specifies the approval endpoint URL for on-demand certificate requests.", tls);
        register_acme_directive(registry, "on_demand_ask_auth", "on_demand_ask_auth <header>", "This directive sets the Authorization header value for the on-demand approval endpoint.", tls);
        register_acme_directive(
            registry,
            "on_demand_ask_no_verification",
            "on_demand_ask_no_verification [bool]",
            "This directive disables TLS verification for the on-demand approval endpoint.",
            tls,
        );
        register_acme_directive(
            registry,
            "dns",
            "dns [bool] | dns { ... }",
            "This directive enables DNS-01 challenges or specifies a DNS provider block for ACME.",
            tls,
        );
        register_acme_directive(registry, "fallback", "fallback { ... }", "This directive defines fallback ACME provider settings including directory, contact, profile, and eab.", tls);
        register_acme_directive(
            registry,
            "key_algo",
            "key_algo <key_algo>",
            "This directive specifies the private key signature algorithm for the certificate \
            signing request for ACME. Default is ECDSA, unless RSA key size is specified which \
            would set the default to RSA",
            tls,
        );
        register_acme_directive(
            registry,
            "key_size",
            "key_size <key_size>",
            "This directive specifies the RSA private key size for the certificate signing \
            request for ACME. Not supported when using non-RSA key signature algorithms",
            tls,
        );
    }

    fn register_scoped_configuration_validators(
        &mut self,
        registry: &mut std::collections::HashMap<
            ferron_core::config::validator::ConfigurationValidatorScopedKey,
            Box<dyn ferron_core::config::validator::ConfigurationValidator>,
        >,
    ) {
        registry.insert(
            config_validator_scoped_key!("tls", "acme"),
            Box::new(validator::TlsAcmeConfigurationValidator),
        );
    }
}

fn register_acme_directive(
    registry: &mut ferron_core::directives::DirectiveRegistry,
    name: &'static str,
    usage: &'static str,
    description: &'static str,
    parent: ferron_core::directives::DirectiveSubblock,
) {
    use ferron_core::directives::Directive;
    registry.register(
        Directive {
            name,
            usage,
            description,
            applicable_protocols: None,
            global_only: false,
            subblock_link: None,
        },
        parent,
    );
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
///     provider acme
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

    fn token() -> Arc<CancellationToken> {
        Arc::new(CancellationToken::new())
    }

    #[test]
    fn staging_generation_is_reused_within_one_load_attempt() {
        let state = AcmeTaskState::new();
        let reload_token = token();

        let first = state.staging_generation_for(reload_token.clone());
        let second = state.staging_generation_for(reload_token);

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn staging_generation_rotates_for_a_new_load_attempt() {
        let state = AcmeTaskState::new();

        let first = state.staging_generation_for(token());
        let second = state.staging_generation_for(token());

        assert!(!Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&first.configs, &second.configs));
        assert!(!Arc::ptr_eq(
            &first.on_demand_configs,
            &second.on_demand_configs
        ));
        assert!(!Arc::ptr_eq(
            &first.sni_resolver_lock,
            &second.sni_resolver_lock
        ));
        assert!(!Arc::ptr_eq(
            &first.challenge_cache_dirs,
            &second.challenge_cache_dirs
        ));
    }

    #[test]
    fn promotion_takes_the_staging_generation() {
        let state = AcmeTaskState::new();
        let reload_token = token();

        let staging = state.staging_generation_for(reload_token.clone());
        let active = state.promote_generation_for(reload_token);

        assert!(Arc::ptr_eq(&staging, &active));
    }

    #[test]
    fn failed_load_does_not_touch_the_active_generation() {
        let state = AcmeTaskState::new();
        let active = state.promote_generation_for(token());
        state.active.store(Some(active.clone()));

        // A failed load only builds a staging generation. It must not reach the
        // generation whose background task is still running.
        let failed = state.staging_generation_for(token());
        failed.register_challenge_cache_dir(std::path::PathBuf::from("/tmp/failed"));
        assert!(active.challenge_cache_dirs.blocking_read().is_empty());

        let current = state.active.load_full().expect("active generation");
        assert!(Arc::ptr_eq(&current, &active));
    }

    #[test]
    fn promotion_without_staging_yields_an_empty_generation() {
        let state = AcmeTaskState::new();

        // A previous load attempt left a staging generation behind.
        let stale = state.staging_generation_for(token());
        stale.register_challenge_cache_dir(std::path::PathBuf::from("/tmp/stale"));

        // The next attempt removed all ACME configuration, so no provider built
        // a staging generation for its token.
        let promoted = state.promote_generation_for(token());

        assert!(!Arc::ptr_eq(&stale, &promoted));
        assert!(promoted.configs.blocking_read().is_empty());
        assert!(promoted.on_demand_configs.blocking_read().is_empty());
        assert!(promoted.challenge_cache_dirs.blocking_read().is_empty());
    }

    #[test]
    fn installing_a_new_active_task_returns_the_previous_one() {
        let state = AcmeTaskState::new();

        let task = |generation_token| ActiveTask {
            generation: Arc::new(AcmeGeneration::new(generation_token)),
            cancel: CancellationToken::new(),
            drain: CancellationToken::new(),
            done: CancellationToken::new(),
        };

        assert!(state.install_active(task(token())).is_none());
        assert!(state.install_active(task(token())).is_some());
    }

    #[tokio::test]
    async fn prune_challenge_locks_keeps_data_referenced_by_live_configs() {
        let state = AcmeTaskState::new();
        let live: crate::challenge::Http01DataLock = Arc::new(RwLock::new(None));
        state.http_01_resolvers.write().await.push(live.clone());
        state
            .http_01_resolvers
            .write()
            .await
            .push(Arc::new(RwLock::new(None)));

        state.prune_challenge_locks().await;

        let resolvers = state.http_01_resolvers.read().await;
        assert_eq!(resolvers.len(), 1);
        assert!(Arc::ptr_eq(&resolvers[0], &live));
        drop(resolvers);

        drop(live);
        state.prune_challenge_locks().await;
        assert!(state.http_01_resolvers.read().await.is_empty());
    }
}
