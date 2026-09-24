//! Central abuse registry: tracks bans, records events, and enforces thresholds.

use cidr::IpCidr;
use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use ferron_http::HttpContext;
use rustc_hash::FxBuildHasher;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use ferron_http::abuse::{AbuseEvent, AbuseEventType, AbuseRecorder, EventResult};

/// Configuration for a per-event-type threshold.
#[derive(Debug, Clone)]
pub struct EventThreshold {
    /// Event type this threshold applies to.
    pub event_type: AbuseEventType,
    /// Number of events required to trigger a ban within the window.
    pub events_count: usize,
    /// Time window in seconds.
    pub window_secs: u64,
}

impl EventThreshold {
    pub fn new(event_type: AbuseEventType, events_count: usize, window_secs: u64) -> Self {
        Self {
            event_type,
            events_count,
            window_secs,
        }
    }
}

/// Configuration for an error rate threshold.
#[derive(Debug, Clone)]
pub struct ErrorRateThresholdConfig {
    /// The underlying event threshold (events count + window).
    pub event_threshold: EventThreshold,
    /// HTTP status codes that count as errors (e.g., 404, 403).
    pub status_codes: Vec<u16>,
}

impl ErrorRateThresholdConfig {
    pub fn new(events_count: usize, window_secs: u64, status_codes: Vec<u16>) -> Self {
        Self {
            event_threshold: EventThreshold::new(
                AbuseEventType::ErrorRate,
                events_count,
                window_secs,
            ),
            status_codes,
        }
    }
}

/// Configuration for the abuse registry.
#[derive(Debug, Clone)]
pub struct AbuseRegistryConfig {
    /// Whether abuse protection is enabled.
    pub enabled: bool,
    /// Duration of bans in seconds.
    pub ban_duration_secs: u64,
    /// Per-event-type thresholds.
    pub thresholds: Vec<EventThreshold>,
    /// Error rate thresholds (track response status codes).
    pub error_rate_thresholds: Vec<ErrorRateThresholdConfig>,
    /// IPs or CIDR ranges that are exempt from bans.
    pub allowlist: Vec<IpCidr>,
}

impl AbuseRegistryConfig {
    pub const DEFAULT_BAN_DURATION_SECS: u64 = 900; // 15 minutes
}

impl Default for AbuseRegistryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ban_duration_secs: Self::DEFAULT_BAN_DURATION_SECS,
            thresholds: vec![
                EventThreshold::new(AbuseEventType::RateLimitExceeded, 5, 300),
                EventThreshold::new(AbuseEventType::BruteForceFailure, 3, 120),
            ],
            error_rate_thresholds: Vec::new(),
            allowlist: Vec::new(),
        }
    }
}

impl typemap_rev::TypeMapKey for AbuseRegistryConfig {
    type Value = Self;
}

/// Metadata for a single ban entry.
#[derive(Debug, Clone)]
struct BanEntry {
    /// Reason for the ban.
    reason: String,
    /// When the ban expires.
    expires_at: Instant,
}

impl BanEntry {
    /// Check if this ban is still active.
    fn is_active(&self) -> bool {
        Instant::now() < self.expires_at
    }

    /// Get the remaining ban duration.
    fn time_remaining(&self) -> Duration {
        self.expires_at
            .checked_duration_since(Instant::now())
            .unwrap_or_default()
    }
}

/// Per-event tracker: stores timestamps of recent events for a given IP + event type.
#[derive(Debug)]
struct EventTracker {
    /// Timestamps of events within the current window (per event type).
    events: Vec<Instant>,
}

impl EventTracker {
    fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Prune events outside the given time window.
    fn prune(&mut self, window: Duration) {
        let cutoff = Instant::now().checked_sub(window).unwrap_or(Instant::now());
        self.events.retain(|&t| t >= cutoff);
    }

    /// Record a new event.
    fn record(&mut self) {
        self.events.push(Instant::now());
    }

    /// Get the count of events in the window.
    fn count(&self) -> usize {
        self.events.len()
    }
}

/// Central abuse registry: tracks bans and event thresholds.
///
/// Manages per-IP ban records with automatic TTL-based expiration,
/// and per-IP-per-event-type event tracking for threshold aggregation.
pub struct AbuseRegistry {
    /// Active bans by IP address.
    bans: DashMap<IpAddr, BanEntry, FxBuildHasher>,
    /// Event trackers per IP and event type (key: "ip:event_type").
    event_trackers: DashMap<String, EventTracker, FxBuildHasher>,
    /// Metrics: total bans triggered.
    bans_triggered: AtomicU64,
    /// Current number of active (unexpired) bans. Maintained on every
    /// insert and every lazy eviction so transition logs and gauges stay
    /// exact without scanning the map.
    active_bans: AtomicU64,
}

/// Outcome of checking an IP against the ban list.
#[derive(Debug, PartialEq, Eq)]
pub enum BanCheck {
    /// The IP is currently banned.
    Banned {
        /// Reason recorded when the ban was triggered.
        reason: String,
        /// Remaining ban duration.
        remaining: Duration,
    },
    /// The IP had a ban that has just expired. The entry was evicted by
    /// this call; only the first observer sees this variant.
    Expired {
        /// Reason recorded when the ban was triggered.
        reason: String,
    },
    /// The IP is not (and was not) banned.
    Clean,
}

impl Default for AbuseRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AbuseRegistry {
    /// Create a new abuse registry with the given configuration.
    pub fn new() -> Self {
        Self {
            bans: DashMap::with_hasher(FxBuildHasher),
            event_trackers: DashMap::with_hasher(FxBuildHasher),
            bans_triggered: AtomicU64::new(0),
            active_bans: AtomicU64::new(0),
        }
    }

    /// Check if an IP is allowlisted and therefore exempt from bans.
    pub fn is_allowlisted(ip: IpAddr, config: &AbuseRegistryConfig) -> bool {
        config.allowlist.iter().any(|cidr| cidr.contains(&ip))
    }

    /// Check if an IP address is currently banned.
    ///
    /// Lazily evicts expired bans on access.
    pub fn is_banned(&self, ip: IpAddr, config: &AbuseRegistryConfig) -> bool {
        matches!(self.check_ban(ip, config), BanCheck::Banned { .. })
    }

    /// Check an IP against the ban list, evicting expired bans.
    ///
    /// Uses the entry API so observing an expiry and evicting it is atomic:
    /// concurrent observers either see the active ban or a clean map, and
    /// only the first observer of an expiry sees [`BanCheck::Expired`].
    pub fn check_ban(&self, ip: IpAddr, config: &AbuseRegistryConfig) -> BanCheck {
        if !config.enabled {
            return BanCheck::Clean;
        }

        let ip = ip.to_canonical();

        if Self::is_allowlisted(ip, config) {
            return BanCheck::Clean;
        }

        // Read-lock fast path (write-lock path below is mutually exclusive to single thread)
        if let Some(entry) = self.bans.get(&ip) {
            if entry.is_active() {
                return BanCheck::Banned {
                    reason: entry.reason.clone(),
                    remaining: entry.time_remaining(),
                };
            }
        } else {
            return BanCheck::Clean;
        }

        match self.bans.entry(ip) {
            Entry::Occupied(slot) => {
                if slot.get().is_active() {
                    BanCheck::Banned {
                        reason: slot.get().reason.clone(),
                        remaining: slot.get().time_remaining(),
                    }
                } else {
                    let entry = slot.remove();
                    self.active_bans.fetch_sub(1, Ordering::Relaxed);
                    BanCheck::Expired {
                        reason: entry.reason,
                    }
                }
            }
            Entry::Vacant(_) => BanCheck::Clean,
        }
    }

    /// Current number of active (unexpired) bans.
    pub fn active_ban_count(&self) -> u64 {
        self.active_bans.load(Ordering::Relaxed)
    }

    /// Insert or refresh a ban, maintaining the active-ban count.
    ///
    /// Refreshing an already-active ban keeps the counter exact by only
    /// counting transitions from inactive to active.
    fn insert_ban(&self, ip: IpAddr, entry: BanEntry) {
        match self.bans.entry(ip) {
            Entry::Occupied(mut slot) => {
                let was_active = slot.get().is_active();
                slot.insert(entry);
                if !was_active {
                    self.active_bans.fetch_add(1, Ordering::Relaxed);
                }
            }
            Entry::Vacant(slot) => {
                slot.insert(entry);
                self.active_bans.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Record an abuse event and check thresholds.
    ///
    /// Returns `EventResult::BanTriggered` if the event caused a threshold to be met,
    /// otherwise returns `EventResult::Recorded`.
    pub fn record_event(&self, event: &AbuseEvent, config: &AbuseRegistryConfig) -> EventResult {
        if !config.enabled {
            return EventResult::Recorded;
        }

        // Allowlisted IPs are never tracked or banned
        if Self::is_allowlisted(event.ip.to_canonical(), config) {
            return EventResult::Recorded;
        }

        // Check if IP is already banned
        if self.is_banned(event.ip.to_canonical(), config) {
            return EventResult::Recorded;
        }

        // Handle error rate events separately since they need status code matching
        if event.event_type == AbuseEventType::ErrorRate {
            return self.record_error_rate_event(event, config);
        }

        // Opportunistically evict trackers whose events are all older than the
        // largest configured window. Prevents unbounded memory growth from
        // many distinct IP+event_type combinations.
        let max_window_secs = config
            .thresholds
            .iter()
            .map(|t| t.window_secs)
            .max()
            .unwrap_or(3600);
        let eviction_window = Duration::from_secs(max_window_secs);
        self.evict_stale_trackers_with_window(eviction_window);

        let key = format!("{}:{}", event.ip.to_canonical(), event.event_type.as_str());
        let mut tracker = self
            .event_trackers
            .entry(key.clone())
            .or_insert_with(EventTracker::new);

        // Find matching threshold (cloned to avoid holding the config lock)
        let threshold = match config
            .thresholds
            .iter()
            .find(|t| t.event_type == event.event_type)
            .cloned()
        {
            Some(t) => t,
            None => return EventResult::Recorded, // No threshold for this event type
        };

        // Prune old events outside the window
        let window = Duration::from_secs(threshold.window_secs);
        tracker.prune(window);

        // Record the new event
        tracker.record();

        // Check if threshold is met
        if tracker.count() >= threshold.events_count {
            let ban_duration = Duration::from_secs(config.ban_duration_secs);
            let ban_entry = BanEntry {
                reason: event.reason.clone(),
                expires_at: Instant::now() + ban_duration,
            };

            self.insert_ban(event.ip.to_canonical(), ban_entry);
            self.bans_triggered.fetch_add(1, Ordering::Relaxed);

            // Clear the tracker after ban. We must drop the RefMut first to avoid
            // a deadlock (DashMap::remove takes a shard write lock, which the
            // RefMut also holds), then remove the entry. The window between drop
            // and remove is safe because a concurrent thread hitting the same key
            // will see `count() >= threshold` and trigger another ban, but the
            // worst case is a redundant ban insert (same IP, slightly extended
            // expiry) and a double-count of `bans_triggered`, both benign.
            //
            // To reduce the window, we clear events first (making the count 0 for
            // any concurrent reader), then drop, then remove.
            tracker.events.clear();
            drop(tracker);
            self.event_trackers.remove(&key);

            EventResult::BanTriggered
        } else {
            EventResult::Recorded
        }
    }

    /// Record an error rate event, checking against configured error rate thresholds.
    fn record_error_rate_event(
        &self,
        event: &AbuseEvent,
        config: &AbuseRegistryConfig,
    ) -> EventResult {
        let status_code = match event.status_code {
            Some(sc) => sc,
            None => return EventResult::Recorded,
        };

        let mut overall_result = EventResult::Recorded;

        for error_threshold in &config.error_rate_thresholds {
            if !error_threshold.status_codes.contains(&status_code) {
                continue;
            }

            let eviction_window = Duration::from_secs(error_threshold.event_threshold.window_secs);
            self.evict_stale_trackers_with_window(eviction_window);

            let key = format!(
                "{}:{}:{}",
                event.ip.to_canonical(),
                event.event_type.as_str(),
                error_threshold
                    .status_codes
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            let mut tracker = self
                .event_trackers
                .entry(key.clone())
                .or_insert_with(EventTracker::new);

            let window = Duration::from_secs(error_threshold.event_threshold.window_secs);
            tracker.prune(window);
            tracker.record();
            if tracker.count() >= error_threshold.event_threshold.events_count {
                let ban_duration = Duration::from_secs(config.ban_duration_secs);
                let ban_entry = BanEntry {
                    reason: event.reason.clone(),
                    expires_at: Instant::now() + ban_duration,
                };

                self.insert_ban(event.ip.to_canonical(), ban_entry);
                self.bans_triggered.fetch_add(1, Ordering::Relaxed);

                tracker.events.clear();
                drop(tracker);
                self.event_trackers.remove(&key);

                overall_result = EventResult::BanTriggered;
                break;
            }
        }

        overall_result
    }

    /// Get the total number of bans triggered since startup.
    #[cfg(test)]
    pub fn total_bans_triggered(&self) -> u64 {
        self.bans_triggered.load(Ordering::Relaxed)
    }

    /// Evict stale event trackers to prevent unbounded memory growth.
    ///
    /// A tracker is removed when all of its events are older than
    /// `max_window`. Should be called periodically (e.g., every minute) or
    /// lazily. Without an upper bound on the eviction window, default
    /// thresholds (up to 5 minutes) are used as a safe fallback.
    #[cfg(test)]
    pub fn evict_stale_trackers(&self) {
        self.evict_stale_trackers_with_window(Duration::from_secs(3600));
    }

    /// Evict event trackers whose events are all older than `max_window`.
    /// Passing a conservative window ensures we keep at least as much
    /// state as any configured threshold needs.
    pub fn evict_stale_trackers_with_window(&self, max_window: Duration) {
        let cutoff = Instant::now()
            .checked_sub(max_window)
            .unwrap_or(Instant::now());
        self.event_trackers
            .retain(|_, tracker| tracker.events.iter().any(|&t| t >= cutoff));
    }
}

impl AbuseRecorder for AbuseRegistry {
    #[inline]
    fn record_event(&self, event: &AbuseEvent, ctx: &HttpContext) -> EventResult {
        let result = if let Some(config) = ctx.extensions.get::<AbuseRegistryConfig>() {
            self.record_event(event, config)
        } else {
            EventResult::Recorded
        };

        if result == EventResult::BanTriggered {
            ctx.events.emit(ferron_observability::Event::Log(
                ferron_observability::LogEvent {
                    level: ferron_observability::LogLevel::Warn,
                    message: format!("Ban triggered: IP {} - {}", event.ip, event.reason),
                    summary: "Ban triggered".into(),
                    target: "ferron-http-abuseban",
                    attributes: vec![
                        (
                            "client.address",
                            ferron_observability::LogAttributeValue::String(
                                event.ip.to_canonical().to_string(),
                            ),
                        ),
                        (
                            "ferron.abuseban.reason",
                            ferron_observability::LogAttributeValue::String(event.reason.clone()),
                        ),
                    ],
                    trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                },
            ));

            ctx.events.emit(ferron_observability::Event::Metric(
                ferron_observability::MetricEvent {
                    name: "ferron.abuseban.triggered",
                    attributes: vec![(
                        "ferron.abuseban.reason",
                        ferron_observability::MetricAttributeValue::String(event.reason.clone()),
                    )],
                    ty: ferron_observability::MetricType::Counter,
                    value: ferron_observability::MetricValue::U64(1),
                    unit: Some("{request}"),
                    description: Some("Requests that triggered an IP ban."),
                    trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                },
            ));

            emit_active_bans_gauge(ctx, self.active_ban_count());
        }

        result
    }

    fn is_banned(&self, ip: IpAddr, ctx: &HttpContext) -> bool {
        let Some(config) = ctx.extensions.get::<AbuseRegistryConfig>() else {
            return false;
        };
        match self.check_ban(ip, config) {
            BanCheck::Banned { .. } => true,
            // Only the first observer of an expiry sees this variant, so the
            // transition is logged exactly once per ban lifecycle.
            BanCheck::Expired { reason } => {
                emit_ban_expired(ctx, ip.to_canonical(), &reason, self.active_ban_count());
                false
            }
            BanCheck::Clean => false,
        }
    }
}

/// Emit the ban-expired transition log, expiry counter, and active-ban gauge.
///
/// Callers must only invoke this for bans they evicted themselves (see
/// [`AbuseRegistry::check_ban`]) so each expiry is reported exactly once.
pub(crate) fn emit_ban_expired(ctx: &HttpContext, ip: IpAddr, reason: &str, active_bans: u64) {
    ctx.events.emit(ferron_observability::Event::Log(
        ferron_observability::LogEvent {
            level: ferron_observability::LogLevel::Info,
            message: format!("Ban expired: IP {ip} - {reason}"),
            summary: "Ban expired".into(),
            target: "ferron-http-abuseban",
            attributes: vec![
                (
                    "client.address",
                    ferron_observability::LogAttributeValue::String(ip.to_string()),
                ),
                (
                    "ferron.abuseban.reason",
                    ferron_observability::LogAttributeValue::String(reason.to_string()),
                ),
            ],
            trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
        },
    ));

    ctx.events.emit(ferron_observability::Event::Metric(
        ferron_observability::MetricEvent {
            name: "ferron.abuseban.expired",
            attributes: vec![(
                "ferron.abuseban.reason",
                ferron_observability::MetricAttributeValue::String(reason.to_string()),
            )],
            ty: ferron_observability::MetricType::Counter,
            value: ferron_observability::MetricValue::U64(1),
            unit: Some("{ban}"),
            description: Some("IP bans that expired."),
            trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
        },
    ));

    emit_active_bans_gauge(ctx, active_bans);
}

/// Emit the current active-ban count gauge.
fn emit_active_bans_gauge(ctx: &HttpContext, active_bans: u64) {
    ctx.events.emit(ferron_observability::Event::Metric(
        ferron_observability::MetricEvent {
            name: "ferron.abuseban.active_bans",
            attributes: vec![],
            ty: ferron_observability::MetricType::Gauge,
            value: ferron_observability::MetricValue::U64(active_bans),
            unit: Some("{ban}"),
            description: Some("Current number of active IP bans."),
            trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;
    use std::sync::Arc;
    use std::thread;

    use ferron_http::abuse::{AbuseEvent, AbuseEventType, EventResult};

    fn make_test_config() -> AbuseRegistryConfig {
        AbuseRegistryConfig {
            enabled: true,
            ban_duration_secs: 60,
            thresholds: vec![EventThreshold::new(
                AbuseEventType::RateLimitExceeded,
                3,
                10,
            )],
            error_rate_thresholds: Vec::new(),
            allowlist: Vec::new(),
        }
    }

    fn test_ip() -> IpAddr {
        "192.168.1.1".parse().unwrap()
    }

    #[test]
    fn empty_registry_has_no_bans() {
        let registry = AbuseRegistry::new();
        assert!(!registry.is_banned(test_ip(), &make_test_config()));
        assert_eq!(
            registry.check_ban(test_ip(), &make_test_config()),
            BanCheck::Clean
        );
    }

    #[test]
    fn events_below_threshold_not_banned() {
        let registry = AbuseRegistry::new();
        let event = AbuseEvent::new(
            AbuseEventType::RateLimitExceeded,
            test_ip(),
            "Too fast".into(),
            50,
        );

        assert_eq!(
            registry.record_event(&event, &make_test_config()),
            EventResult::Recorded
        );
        assert!(!registry.is_banned(test_ip(), &make_test_config()));
        assert_eq!(
            registry.record_event(&event, &make_test_config()),
            EventResult::Recorded
        );
        assert!(!registry.is_banned(test_ip(), &make_test_config()));
    }

    #[test]
    fn threshold_reached_triggers_ban() {
        let registry = AbuseRegistry::new();
        let event = AbuseEvent::new(
            AbuseEventType::RateLimitExceeded,
            test_ip(),
            "Too fast".into(),
            50,
        );

        registry.record_event(&event, &make_test_config());
        registry.record_event(&event, &make_test_config());
        let result = registry.record_event(&event, &make_test_config());

        assert_eq!(result, EventResult::BanTriggered);
        assert!(registry.is_banned(test_ip(), &make_test_config()));
        match registry.check_ban(test_ip(), &make_test_config()) {
            BanCheck::Banned { reason, .. } => assert_eq!(reason, "Too fast"),
            other => panic!("expected Banned, got {other:?}"),
        }
        assert_eq!(registry.total_bans_triggered(), 1);
    }

    #[test]
    fn disabled_registry_never_bans() {
        let config = AbuseRegistryConfig {
            enabled: false,
            ..make_test_config()
        };
        let registry = AbuseRegistry::new();
        let event = AbuseEvent::new(
            AbuseEventType::RateLimitExceeded,
            test_ip(),
            "Too fast".into(),
            50,
        );

        for _ in 0..10 {
            registry.record_event(&event, &config);
        }

        assert!(!registry.is_banned(test_ip(), &config));
        assert_eq!(registry.total_bans_triggered(), 0);
    }

    #[test]
    fn different_ips_tracked_separately() {
        let registry = AbuseRegistry::new();
        let event1 = AbuseEvent::new(
            AbuseEventType::RateLimitExceeded,
            "192.168.1.1".parse().unwrap(),
            "Too fast".into(),
            50,
        );
        let event2 = AbuseEvent::new(
            AbuseEventType::RateLimitExceeded,
            "192.168.1.2".parse().unwrap(),
            "Too fast".into(),
            50,
        );

        registry.record_event(&event1, &make_test_config());
        registry.record_event(&event1, &make_test_config());
        registry.record_event(&event1, &make_test_config());

        assert!(registry.is_banned(event1.ip, &make_test_config()));
        assert!(!registry.is_banned(event2.ip, &make_test_config()));
    }

    #[test]
    fn different_event_types_tracked_separately() {
        let config = AbuseRegistryConfig {
            enabled: true,
            ban_duration_secs: 60,
            thresholds: vec![
                EventThreshold::new(AbuseEventType::RateLimitExceeded, 2, 10),
                EventThreshold::new(AbuseEventType::BruteForceFailure, 3, 10),
            ],
            error_rate_thresholds: Vec::new(),
            allowlist: Vec::new(),
        };
        let registry = AbuseRegistry::new();
        let ip = test_ip();
        let rate_limit_event = AbuseEvent::new(
            AbuseEventType::RateLimitExceeded,
            ip,
            "Rate limited".into(),
            50,
        );
        let brute_force_event = AbuseEvent::new(
            AbuseEventType::BruteForceFailure,
            ip,
            "Brute force".into(),
            50,
        );

        // 2 rate limit events (at threshold)
        registry.record_event(&rate_limit_event, &config);
        assert_eq!(
            registry.record_event(&rate_limit_event, &config),
            EventResult::BanTriggered
        );
        assert!(registry.is_banned(ip, &config));

        // After ban, new events shouldn't trigger further tracking
        assert_eq!(
            registry.record_event(&brute_force_event, &config),
            EventResult::Recorded
        );
    }

    #[test]
    fn concurrent_event_recording() {
        let registry = Arc::new(AbuseRegistry::new());
        let mut handles = Vec::new();

        for i in 0..4 {
            let reg = registry.clone();
            handles.push(thread::spawn(move || {
                let ip: IpAddr = format!("192.168.1.{}", i + 1).parse().unwrap();
                let event = AbuseEvent::new(
                    AbuseEventType::RateLimitExceeded,
                    ip,
                    "Concurrent".into(),
                    50,
                );
                for _ in 0..3 {
                    AbuseRegistry::record_event(&reg, &event, &make_test_config());
                }
                AbuseRegistry::is_banned(&reg, ip, &make_test_config())
            }));
        }

        let results: Vec<bool> = handles
            .into_iter()
            .map(|h: std::thread::JoinHandle<bool>| h.join().unwrap())
            .collect();
        assert_eq!(results.len(), 4);
        assert!(results.iter().all(|&b| b), "all IPs should be banned");
    }

    #[test]
    fn empty_thresholds_list_no_ban() {
        let config = AbuseRegistryConfig {
            enabled: true,
            ban_duration_secs: 60,
            thresholds: vec![],
            error_rate_thresholds: Vec::new(),
            allowlist: Vec::new(),
        };
        let registry = AbuseRegistry::new();
        let event = AbuseEvent::new(
            AbuseEventType::RateLimitExceeded,
            test_ip(),
            "No threshold".into(),
            50,
        );

        for _ in 0..10 {
            assert_eq!(
                registry.record_event(&event, &config),
                EventResult::Recorded
            );
        }
        assert!(!registry.is_banned(test_ip(), &config));
    }

    #[test]
    fn no_matching_threshold_for_event_type() {
        let config = AbuseRegistryConfig {
            enabled: true,
            ban_duration_secs: 60,
            thresholds: vec![EventThreshold::new(
                AbuseEventType::RateLimitExceeded,
                2,
                10,
            )],
            error_rate_thresholds: Vec::new(),
            allowlist: Vec::new(),
        };
        let registry = AbuseRegistry::new();
        let event = AbuseEvent::new(
            AbuseEventType::BruteForceFailure,
            test_ip(),
            "No matching threshold".into(),
            50,
        );

        for _ in 0..10 {
            assert_eq!(
                registry.record_event(&event, &config),
                EventResult::Recorded
            );
        }
        assert!(!registry.is_banned(test_ip(), &config));
    }

    #[test]
    fn evict_stale_trackers_cleans_up() {
        let registry = AbuseRegistry::new();
        let event = AbuseEvent::new(
            AbuseEventType::RateLimitExceeded,
            test_ip(),
            "Tracker test".into(),
            50,
        );

        registry.record_event(&event, &make_test_config());
        // Trigger a ban so the tracker is removed
        registry.record_event(&event, &make_test_config());
        registry.record_event(&event, &make_test_config());

        // The tracker should have been cleaned up when the ban was triggered
        registry.evict_stale_trackers();
        // No crash and no stale trackers
    }

    #[test]
    fn record_event_on_already_banned_ip_returns_recorded() {
        let registry = AbuseRegistry::new();
        let event = AbuseEvent::new(
            AbuseEventType::RateLimitExceeded,
            test_ip(),
            "Already banned".into(),
            50,
        );

        registry.record_event(&event, &make_test_config());
        registry.record_event(&event, &make_test_config());
        registry.record_event(&event, &make_test_config());

        // IP is now banned
        let result = registry.record_event(&event, &make_test_config());
        assert_eq!(result, EventResult::Recorded);
        assert_eq!(registry.total_bans_triggered(), 1);
    }

    #[test]
    fn concurrent_same_ip_race_prevents_double_ban() {
        // Multiple threads racing on the same IP should still result in a ban,
        // but bans_triggered should not be inflated by more than a small margin.
        let registry = Arc::new(AbuseRegistry::new());
        let config = make_test_config();
        let mut handles = Vec::new();

        // Launch 20 threads all hitting the same IP simultaneously.
        // Threshold is 3 events. Each thread records 3 events.
        for _ in 0..20 {
            let reg = registry.clone();
            let cfg = config.clone();
            handles.push(thread::spawn(move || {
                let ip = test_ip();
                let event = AbuseEvent::new(
                    AbuseEventType::RateLimitExceeded,
                    ip,
                    "Concurrent race".into(),
                    50,
                );
                for _ in 0..3 {
                    AbuseRegistry::record_event(&reg, &event, &cfg);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // The IP must be banned
        assert!(
            AbuseRegistry::is_banned(&registry, test_ip(), &config),
            "IP should be banned after concurrent events"
        );
        // bans_triggered should be close to 1. Due to the race window, it may
        // be slightly higher (e.g., 2-3), but should be much less than 20.
        let triggered = registry.total_bans_triggered();
        assert!(
            (1..=5).contains(&triggered),
            "bans_triggered should be 1-5, got {triggered}"
        );
    }

    #[test]
    fn error_rate_threshold_triggers_ban_on_matching_status_code() {
        let config = AbuseRegistryConfig {
            enabled: true,
            ban_duration_secs: 60,
            thresholds: vec![],
            error_rate_thresholds: vec![ErrorRateThresholdConfig::new(3, 60, vec![404, 403])],
            allowlist: Vec::new(),
        };
        let registry = AbuseRegistry::new();
        let ip = test_ip();

        let event = AbuseEvent::with_status_code(
            AbuseEventType::ErrorRate,
            ip,
            "Error rate: 404 responses".into(),
            40,
            404,
        );

        assert_eq!(
            registry.record_event(&event, &config),
            EventResult::Recorded
        );
        assert_eq!(
            registry.record_event(&event, &config),
            EventResult::Recorded
        );
        assert_eq!(
            registry.record_event(&event, &config),
            EventResult::BanTriggered
        );
        assert!(registry.is_banned(ip, &config));
    }

    #[test]
    fn error_rate_threshold_ignores_non_matching_status_code() {
        let config = AbuseRegistryConfig {
            enabled: true,
            ban_duration_secs: 60,
            thresholds: vec![],
            error_rate_thresholds: vec![ErrorRateThresholdConfig::new(3, 60, vec![404])],
            allowlist: Vec::new(),
        };
        let registry = AbuseRegistry::new();
        let ip = test_ip();

        // 500 doesn't match the configured 404
        let event = AbuseEvent::with_status_code(
            AbuseEventType::ErrorRate,
            ip,
            "Error rate: 500 responses".into(),
            40,
            500,
        );

        for _ in 0..10 {
            assert_eq!(
                registry.record_event(&event, &config),
                EventResult::Recorded
            );
        }
        assert!(!registry.is_banned(ip, &config));
    }

    #[test]
    fn error_rate_threshold_no_status_code_returns_recorded() {
        let config = AbuseRegistryConfig {
            enabled: true,
            ban_duration_secs: 60,
            thresholds: vec![],
            error_rate_thresholds: vec![ErrorRateThresholdConfig::new(3, 60, vec![404])],
            allowlist: Vec::new(),
        };
        let registry = AbuseRegistry::new();
        let ip = test_ip();

        // Event without status_code
        let event = AbuseEvent::new(
            AbuseEventType::ErrorRate,
            ip,
            "Error rate: no status".into(),
            40,
        );

        for _ in 0..10 {
            assert_eq!(
                registry.record_event(&event, &config),
                EventResult::Recorded
            );
        }
        assert!(!registry.is_banned(ip, &config));
    }

    #[test]
    fn error_rate_threshold_multiple_status_codes_count_together() {
        let config = AbuseRegistryConfig {
            enabled: true,
            ban_duration_secs: 60,
            thresholds: vec![],
            error_rate_thresholds: vec![ErrorRateThresholdConfig::new(3, 60, vec![404, 403])],
            allowlist: Vec::new(),
        };
        let registry = AbuseRegistry::new();
        let ip = test_ip();

        let event_404 = AbuseEvent::with_status_code(
            AbuseEventType::ErrorRate,
            ip,
            "Error rate: 404 responses".into(),
            40,
            404,
        );
        let event_403 = AbuseEvent::with_status_code(
            AbuseEventType::ErrorRate,
            ip,
            "Error rate: 403 responses".into(),
            40,
            403,
        );

        // Mix of 404 and 403 events
        assert_eq!(
            registry.record_event(&event_404, &config),
            EventResult::Recorded
        );
        assert_eq!(
            registry.record_event(&event_403, &config),
            EventResult::Recorded
        );
        assert_eq!(
            registry.record_event(&event_404, &config),
            EventResult::BanTriggered
        );
        assert!(registry.is_banned(ip, &config));
    }

    #[test]
    fn error_rate_threshold_empty_config_no_ban() {
        let config = AbuseRegistryConfig {
            enabled: true,
            ban_duration_secs: 60,
            thresholds: vec![],
            error_rate_thresholds: Vec::new(),
            allowlist: Vec::new(),
        };
        let registry = AbuseRegistry::new();
        let ip = test_ip();

        let event = AbuseEvent::with_status_code(
            AbuseEventType::ErrorRate,
            ip,
            "Error rate: 404 responses".into(),
            40,
            404,
        );

        for _ in 0..10 {
            assert_eq!(
                registry.record_event(&event, &config),
                EventResult::Recorded
            );
        }
        assert!(!registry.is_banned(ip, &config));
    }

    #[test]
    fn error_rate_threshold_disabled_never_bans() {
        let config = AbuseRegistryConfig {
            enabled: false,
            ban_duration_secs: 60,
            thresholds: vec![],
            error_rate_thresholds: vec![ErrorRateThresholdConfig::new(1, 60, vec![404])],
            allowlist: Vec::new(),
        };
        let registry = AbuseRegistry::new();
        let ip = test_ip();

        let event = AbuseEvent::with_status_code(
            AbuseEventType::ErrorRate,
            ip,
            "Error rate: 404 responses".into(),
            40,
            404,
        );

        for _ in 0..10 {
            assert_eq!(
                registry.record_event(&event, &config),
                EventResult::Recorded
            );
        }
        assert!(!registry.is_banned(ip, &config));
    }

    fn ban_test_ip(registry: &AbuseRegistry, config: &AbuseRegistryConfig) {
        let event = AbuseEvent::new(
            AbuseEventType::RateLimitExceeded,
            test_ip(),
            "Too fast".into(),
            50,
        );
        for _ in 0..3 {
            registry.record_event(&event, config);
        }
        assert!(registry.is_banned(test_ip(), config));
    }

    #[test]
    fn check_ban_reports_active_ban() {
        let config = AbuseRegistryConfig {
            ban_duration_secs: 60,
            ..make_test_config()
        };
        let registry = AbuseRegistry::new();
        ban_test_ip(&registry, &config);

        match registry.check_ban(test_ip(), &config) {
            BanCheck::Banned { reason, remaining } => {
                assert_eq!(reason, "Too fast");
                assert!(remaining.as_secs() <= 60);
            }
            other => panic!("expected Banned, got {other:?}"),
        }
        // Observing an active ban must not evict it.
        assert!(registry.is_banned(test_ip(), &config));
        assert_eq!(registry.active_ban_count(), 1);
    }

    #[test]
    fn check_ban_reports_expiry_exactly_once() {
        let config = AbuseRegistryConfig {
            ban_duration_secs: 0,
            ..make_test_config()
        };
        let registry = AbuseRegistry::new();
        let event = AbuseEvent::new(
            AbuseEventType::RateLimitExceeded,
            test_ip(),
            "Too fast".into(),
            50,
        );
        for _ in 0..3 {
            registry.record_event(&event, &config);
        }

        // Zero-duration bans are already expired on first observation.
        match registry.check_ban(test_ip(), &config) {
            BanCheck::Expired { reason } => assert_eq!(reason, "Too fast"),
            other => panic!("expected Expired, got {other:?}"),
        }
        // The entry was evicted: later observers see a clean map, so an
        // expiry transition can only ever be reported once.
        assert_eq!(registry.check_ban(test_ip(), &config), BanCheck::Clean);
        assert!(!registry.is_banned(test_ip(), &config));
        assert_eq!(registry.active_ban_count(), 0);
    }

    #[test]
    fn active_ban_count_tracks_lifecycle() {
        let config = make_test_config();
        let registry = AbuseRegistry::new();
        assert_eq!(registry.active_ban_count(), 0);

        ban_test_ip(&registry, &config);
        assert_eq!(registry.active_ban_count(), 1);

        // Recording further events while banned is a no-op for the ban
        // set, so the counter must stay exact.
        let event = AbuseEvent::new(
            AbuseEventType::RateLimitExceeded,
            test_ip(),
            "Too fast".into(),
            50,
        );
        registry.record_event(&event, &config);
        assert_eq!(registry.active_ban_count(), 1);
    }

    #[test]
    fn check_ban_respects_disabled_and_allowlist() {
        let registry = AbuseRegistry::new();
        ban_test_ip(&registry, &make_test_config());

        let disabled = AbuseRegistryConfig {
            enabled: false,
            ..make_test_config()
        };
        assert_eq!(registry.check_ban(test_ip(), &disabled), BanCheck::Clean);

        let allowlisted = AbuseRegistryConfig {
            allowlist: vec!["192.168.1.0/24".parse().unwrap()],
            ..make_test_config()
        };
        assert_eq!(registry.check_ban(test_ip(), &allowlisted), BanCheck::Clean);
        // Neither path may evict or report the still-active ban.
        assert_eq!(registry.active_ban_count(), 1);
    }
}
