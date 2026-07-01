//! Central abuse registry: tracks bans, records events, and enforces thresholds.

use cidr::IpCidr;
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
        if !config.enabled {
            return false;
        }

        // Allowlisted IPs are never banned
        if Self::is_allowlisted(ip, config) {
            return false;
        }

        if let Some(entry) = self.bans.get(&ip) {
            if entry.is_active() {
                return true;
            }

            // Ban expired, remove it.
            drop(entry);
            self.bans.remove(&ip);
        }

        false
    }

    /// Get the remaining ban duration for an IP, if banned.
    pub fn ban_time_remaining(&self, ip: IpAddr, config: &AbuseRegistryConfig) -> Option<Duration> {
        if !config.enabled {
            return None;
        }

        self.bans.get(&ip).and_then(|entry| {
            if entry.is_active() {
                Some(entry.time_remaining())
            } else {
                None
            }
        })
    }

    /// Get the reason for the current ban on an IP.
    pub fn ban_reason(&self, ip: IpAddr, config: &AbuseRegistryConfig) -> Option<String> {
        if !config.enabled {
            return None;
        }

        self.bans.get(&ip).and_then(|entry| {
            if entry.is_active() {
                Some(entry.reason.clone())
            } else {
                None
            }
        })
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
        if Self::is_allowlisted(event.ip, config) {
            return EventResult::Recorded;
        }

        // Check if IP is already banned
        if self.is_banned(event.ip, config) {
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

        let key = format!("{}:{}", event.ip, event.event_type.as_str());
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

            self.bans.insert(event.ip, ban_entry);
            self.bans_triggered.fetch_add(1, Ordering::Relaxed);

            // Clear the tracker after ban. We must drop the RefMut first to avoid
            // a deadlock (DashMap::remove takes a shard write lock, which the
            // RefMut also holds), then remove the entry. The window between drop
            // and remove is safe because a concurrent thread hitting the same key
            // will see `count() >= threshold` and trigger another ban, but the
            // worst case is a redundant ban insert (same IP, slightly extended
            // expiry) and a double-count of `bans_triggered` — both benign.
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
            // Check if this status code matches the threshold's configured codes
            if !error_threshold.status_codes.contains(&status_code) {
                continue;
            }

            // Opportunistically evict stale trackers
            let eviction_window = Duration::from_secs(error_threshold.event_threshold.window_secs);
            self.evict_stale_trackers_with_window(eviction_window);

            // Use a key that includes the threshold index for independent tracking
            // per error_rate_threshold block
            let key = format!(
                "{}:{}:{}",
                event.ip,
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

            // Prune old events outside the window
            let window = Duration::from_secs(error_threshold.event_threshold.window_secs);
            tracker.prune(window);

            // Record the new event
            tracker.record();

            // Check if threshold is met
            if tracker.count() >= error_threshold.event_threshold.events_count {
                let ban_duration = Duration::from_secs(config.ban_duration_secs);
                let ban_entry = BanEntry {
                    reason: event.reason.clone(),
                    expires_at: Instant::now() + ban_duration,
                };

                self.bans.insert(event.ip, ban_entry);
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

    /// Get the current number of active bans.
    pub fn active_ban_count(&self) -> usize {
        self.bans.retain(|_, entry| entry.is_active());
        self.bans.len()
    }

    /// Get the total number of bans triggered since startup.
    pub fn total_bans_triggered(&self) -> u64 {
        self.bans_triggered.load(Ordering::Relaxed)
    }

    /// Evict stale event trackers to prevent unbounded memory growth.
    ///
    /// A tracker is removed when all of its events are older than
    /// `max_window`. Should be called periodically (e.g., every minute) or
    /// lazily. Without an upper bound on the eviction window, default
    /// thresholds (up to 5 minutes) are used as a safe fallback.
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
            // Log ban rejection
            ctx.events.emit(ferron_observability::Event::Log(
                ferron_observability::LogEvent {
                    level: ferron_observability::LogLevel::Warn,
                    message: format!(
                        "Ban triggered: IP {} - {}",
                        ctx.remote_address.ip(),
                        event.reason
                    ),
                    summary: "Ban triggered".into(),
                    target: "ferron-http-abuseban",
                    attributes: vec![
                        (
                            "client.address",
                            ferron_observability::LogAttributeValue::String(
                                ctx.remote_address.ip().to_string(),
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

            // Emit metric for ban rejection
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
        }

        result
    }

    fn is_banned(&self, ip: IpAddr, ctx: &HttpContext) -> bool {
        if let Some(config) = ctx.extensions.get::<AbuseRegistryConfig>() {
            self.is_banned(ip, config)
        } else {
            false
        }
    }
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
        assert!(registry
            .ban_reason(test_ip(), &make_test_config())
            .is_none());
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
        assert_eq!(
            registry.ban_reason(test_ip(), &make_test_config()),
            Some("Too fast".to_string())
        );
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
    fn ban_with_zero_duration_expires_immediately() {
        let config = AbuseRegistryConfig {
            enabled: true,
            ban_duration_secs: 0,
            thresholds: vec![EventThreshold::new(
                AbuseEventType::RateLimitExceeded,
                1,
                10,
            )],
            error_rate_thresholds: Vec::new(),
            allowlist: Vec::new(),
        };
        let registry = AbuseRegistry::new();
        let event = AbuseEvent::new(
            AbuseEventType::RateLimitExceeded,
            test_ip(),
            "Instant ban".into(),
            50,
        );

        assert_eq!(
            registry.record_event(&event, &config),
            EventResult::BanTriggered
        );
        // With zero duration, the ban should already be expired
        assert!(!registry.is_banned(test_ip(), &config));
        assert!(registry.ban_time_remaining(test_ip(), &config).is_none());
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
    fn ban_time_remaining_returns_reasonable_duration() {
        let config = AbuseRegistryConfig {
            enabled: true,
            ban_duration_secs: 3600,
            thresholds: vec![EventThreshold::new(
                AbuseEventType::RateLimitExceeded,
                1,
                10,
            )],
            error_rate_thresholds: Vec::new(),
            allowlist: Vec::new(),
        };
        let registry = AbuseRegistry::new();
        let event = AbuseEvent::new(
            AbuseEventType::RateLimitExceeded,
            test_ip(),
            "Long ban".into(),
            50,
        );

        registry.record_event(&event, &config);
        let remaining = registry.ban_time_remaining(test_ip(), &config);
        assert!(remaining.is_some());
        let secs = remaining.unwrap().as_secs();
        // Should be close to 3600 seconds (allow slight clock drift)
        assert!(secs > 3590 && secs <= 3600, "expected ~3600s, got {secs}s");
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
}
