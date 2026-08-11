mod helpers;
mod key;
mod outcome;
mod purge;
mod response_helpers;
mod run_helpers;
mod served;
#[cfg(test)]
mod tests;

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use ahash::AHashMap;
use async_trait::async_trait;
use dashmap::DashMap;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::HttpContext;
use http::HeaderMap;
use typemap_rev::TypeMapKey;

use crate::config::{
    parse_cache_config, parse_max_entries, resolve_persist_config, CacheConfig, CacheZoneId,
};
use crate::policy::{CacheScope, RequestCachePolicy};
use crate::store::persist::writer::{
    restore_zone, sanitize_zone_label, PersistManager, RestoreStop,
};
use crate::store::{CacheStore, LookupEntry, StoreStats};

use self::helpers::active_config_generation;

pub(super) struct RequestStateKey;

impl TypeMapKey for RequestStateKey {
    type Value = RequestState;
}

pub(super) struct RequestState {
    config: Arc<CacheConfig>,
    zone_id: CacheZoneId,
    base_key: String,
    request_headers: HeaderMap,
    request_cookies: AHashMap<String, String>,
    private_key: Option<String>,
    purge_url: String,
    request_policy: RequestCachePolicy,
    has_authorization: bool,
    head_only: bool,
    lookup_result: LookupResult,
    store: Arc<CacheStore>,
    /// When present, notifies coalesced waiters when the leader completes.
    _inflight_guard: Option<InflightGuard>,
}

pub(super) enum LookupResult {
    Hit,
    StaleWhileRevalidate {
        entry: Box<LookupEntry>,
        stats: StoreStats,
        /// Key for inflight coalescing on expired-entry misses.
        inflight_key: Option<String>,
        scope: Option<CacheScope>,
        items: usize,
    },
    Revalidate {
        entry: Box<LookupEntry>,
        cache_key: String,
        stats: StoreStats,
    },
    Miss {
        stats: StoreStats,
        /// Key for inflight coalescing on expired-entry misses.
        inflight_key: Option<String>,
    },
    Bypass,
}

/// RAII guard that calls `complete_fetch` when dropped, notifying coalesced waiters.
pub(super) struct InflightGuard {
    store: Arc<CacheStore>,
    cache_key: String,
}

impl Drop for InflightGuard {
    #[inline]
    fn drop(&mut self) {
        self.store.complete_fetch(&self.cache_key);
    }
}

/// Tracks the config generation at which a zone's `max_entries` was last applied.
struct ZoneGeneration {
    generation: AtomicU64,
}

/// Pipeline stage for HTTP response caching.
pub struct HttpCacheStage {
    /// Cache stores keyed by `CacheZoneId`.
    zones: Arc<DashMap<CacheZoneId, Arc<CacheStore>>>,
    /// Config generation at which each zone's `max_entries` was last applied.
    zone_generations: Arc<DashMap<CacheZoneId, ZoneGeneration>>,
    /// Parsed cache configs keyed by hostname, cleared on config reload.
    configs: Arc<DashMap<String, Arc<CacheConfig>>>,
    /// Config generation at which `configs` was last filled.
    config_generation: Arc<AtomicU64>,
    /// Process-wide persistence manager (journal writer thread).
    persist: Arc<PersistManager>,
}

impl HttpCacheStage {
    #[inline]
    pub fn new(persist: Arc<PersistManager>) -> Self {
        Self {
            zones: Arc::new(DashMap::new()),
            zone_generations: Arc::new(DashMap::new()),
            configs: Arc::new(DashMap::new()),
            config_generation: Arc::new(AtomicU64::new(0)),
            persist,
        }
    }

    /// Get the cache config for the request's hostname, parsing it once per
    /// hostname per configuration generation. The configuration can differ
    /// per host within a generation, so the cache is keyed by hostname
    /// rather than by generation alone.
    #[inline]
    fn get_config(&self, ctx: &HttpContext) -> Arc<CacheConfig> {
        let current_gen = active_config_generation();
        if self.config_generation.load(Ordering::Relaxed) != current_gen {
            self.configs.clear();
            self.config_generation.store(current_gen, Ordering::Relaxed);
        }
        let hostname = ctx
            .hostname
            .clone()
            .unwrap_or_else(|| "_default".to_string());
        if let Some(config) = self.configs.get(&hostname) {
            // Fast path (read lock instead of write lock)
            return config.clone();
        }
        self.configs
            .entry(hostname)
            .or_insert_with(|| Arc::new(parse_cache_config(&ctx.configuration)))
            .clone()
    }

    /// Get or create a `CacheStore` for the given zone, updating `max_entries`
    /// only when the configuration generation changes (not on every request).
    async fn get_or_create_zone(
        &self,
        zone_id: &CacheZoneId,
        configuration: &ferron_core::config::layer::LayeredConfiguration,
    ) -> Arc<CacheStore> {
        let persist_config = resolve_persist_config(zone_id, configuration);
        let store_ent = if let Some(e) = self.zones.get(zone_id) {
            e
        } else {
            match self.zones.entry(zone_id.clone()) {
                dashmap::Entry::Occupied(oe) => oe.into_ref().downgrade(),
                dashmap::Entry::Vacant(ve) => {
                    let store = Arc::new(CacheStore::new(crate::config::DEFAULT_MAX_CACHE_ENTRIES));
                    if let Some(dir) = persist_config.dir {
                        let label = zone_id.label().to_string();
                        let zone_dir = dir.join(sanitize_zone_label(&label));
                        // Replay the on-disk state before registering the zone
                        // with the writer: once registered, the writer can
                        // compact (and truncate) the journal at any time.
                        self.restore_zone_from_disk(&store, &label, &zone_dir).await;
                        let persist = self.persist.register_zone(
                            label,
                            zone_dir,
                            persist_config.include_private,
                            persist_config.interval,
                        );
                        store.attach_persistence(persist);
                    }
                    ve.insert(store).downgrade()
                }
            }
        };
        let store = store_ent.clone();

        let current_gen = active_config_generation();

        let should_update = match self.zone_generations.get(zone_id) {
            Some(entry) => entry.generation.load(Ordering::Relaxed) != current_gen,
            None => true,
        };

        if should_update {
            let new_max = match zone_id {
                CacheZoneId::Named(name) => {
                    crate::config::parse_global_zone_max_entries(configuration, name)
                        .unwrap_or(crate::config::DEFAULT_MAX_CACHE_ENTRIES)
                }
                CacheZoneId::Global => parse_max_entries(configuration),
                CacheZoneId::Host(_) => parse_max_entries(configuration),
            };
            store.set_max_entries(new_max);
            self.zone_generations
                .entry(zone_id.clone())
                .or_insert_with(|| ZoneGeneration {
                    generation: AtomicU64::new(current_gen),
                })
                .generation
                .store(current_gen, Ordering::Relaxed);
        }

        store
    }

    /// Replay a zone's snapshot and journal into a freshly created store.
    async fn restore_zone_from_disk(&self, store: &Arc<CacheStore>, label: &str, zone_dir: &Path) {
        let stats = restore_zone(
            zone_dir,
            |key, entry| store.restore_entry(key, entry),
            |key| store.restore_delete(&key),
        )
        .await;
        self.persist.emit_log(
            ferron_observability::LogLevel::Debug,
            format!(
                "cache persistence: restored zone `{label}`: {} records, {} entries, {} tombstones",
                stats.records, stats.puts, stats.deletes,
            ),
            "Cache entries restored from disk at startup",
            vec![(
                "ferron.cache.zone",
                ferron_observability::LogAttributeValue::String(label.to_string()),
            )],
        );
        match stats.stopped {
            None => {}
            Some(RestoreStop::SnapshotIo | RestoreStop::JournalIo) => {
                self.persist.emit_log(
                    ferron_observability::LogLevel::Warn,
                    format!("cache persistence: could not read on-disk state for `{label}`"),
                    "Could not read the persistence files on disk",
                    vec![(
                        "ferron.cache.zone",
                        ferron_observability::LogAttributeValue::String(label.to_string()),
                    )],
                );
            }
            Some(RestoreStop::SnapshotCorrupt | RestoreStop::JournalCorrupt) => {
                self.persist.emit_log(
                    ferron_observability::LogLevel::Warn,
                    format!(
                        "cache persistence: corrupted record in `{label}`; replay stopped, newer records were ignored"
                    ),
                    "Corrupted record in the persistence files; replay stopped",
                    vec![(
                        "ferron.cache.zone",
                        ferron_observability::LogAttributeValue::String(label.to_string()),
                    )],
                );
            }
            Some(RestoreStop::SnapshotTruncated | RestoreStop::JournalTruncated) => {
                self.persist.emit_log(
                    ferron_observability::LogLevel::Debug,
                    format!(
                        "cache persistence: truncated tail in `{label}`; treating as a clean crash stop"
                    ),
                    "Truncated tail in the persistence files, treated as a clean stop",
                    vec![(
                        "ferron.cache.zone",
                        ferron_observability::LogAttributeValue::String(label.to_string()),
                    )],
                );
            }
        }
    }
}

impl Default for HttpCacheStage {
    #[inline]
    fn default() -> Self {
        Self::new(PersistManager::new())
    }
}

#[async_trait(?Send)]
impl Stage<HttpContext> for HttpCacheStage {
    #[inline]
    fn name(&self) -> &str {
        "cache"
    }

    #[inline]
    fn constraints(&self) -> Vec<StageConstraint> {
        vec![
            StageConstraint::After("https_redirect".to_string()),
            StageConstraint::After("rewrite".to_string()),
            StageConstraint::After("rate_limit".to_string()),
            StageConstraint::After("http_response".to_string()),
            StageConstraint::After("abuse_protection".to_string()),
            StageConstraint::After("basicauth".to_string()),
            StageConstraint::Before("forward_proxy".to_string()),
            StageConstraint::Before("reverse_proxy".to_string()),
            StageConstraint::Before("static_file".to_string()),
        ]
    }

    #[inline]
    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        config.is_some_and(|config| config.has_directive("cache"))
    }

    #[inline]
    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        run_helpers::run_forward(self, ctx).await
    }

    #[inline]
    async fn run_inverse(&self, ctx: &mut HttpContext) -> Result<(), PipelineError> {
        run_helpers::run_inverse_handler(self, ctx).await
    }
}
