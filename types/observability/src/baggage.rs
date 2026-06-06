/// Bitflag-style set of telemetry signal types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SignalSet(u8);

impl SignalSet {
    pub const TRACES: SignalSet = SignalSet(1);
    pub const LOGS: SignalSet = SignalSet(2);
    pub const METRICS: SignalSet = SignalSet(4);
    pub const ALL: SignalSet = SignalSet(7);

    pub const fn empty() -> Self {
        SignalSet(0)
    }

    pub const fn contains(self, other: SignalSet) -> bool {
        (self.0 & other.0) == other.0
    }

    pub const fn insert(self, other: SignalSet) -> Self {
        SignalSet(self.0 | other.0)
    }
}

/// Configuration for promoting a single W3C Baggage key into telemetry attributes.
#[derive(Debug, Clone)]
pub struct BaggageKeyPromotion {
    /// The W3C Baggage key to extract.
    pub baggage_key: String,
    /// The OpenTelemetry attribute name to use. Defaults to `baggage_key` if None.
    pub attribute_name: Option<String>,
    /// Which signals to emit this attribute on. Defaults to `ALL` if not set.
    pub signals: Option<SignalSet>,
    /// Maximum distinct values for metrics before hashing. None means no cap.
    pub max_distinct: Option<usize>,
}

impl BaggageKeyPromotion {
    /// The effective attribute name (falls back to the baggage key).
    pub fn effective_attribute_name(&self) -> &str {
        self.attribute_name.as_deref().unwrap_or(&self.baggage_key)
    }

    /// Whether this promotion applies to the given signal set.
    pub fn applies_to(&self, signal: SignalSet) -> bool {
        match self.signals {
            Some(s) => s.contains(signal),
            None => true, // default: all signals
        }
    }
}

/// A single extracted baggage key-value pair ready to become an attribute.
#[derive(Debug, Clone)]
pub struct ExtractedBaggageAttr {
    pub attribute_name: String,
    pub value: String,
}

/// Tracks distinct baggage values per key and hashes overflow to prevent
/// high-cardinality metric explosion.
///
/// Not `Sync` — create one per provider cache / processing task.
pub struct DistinctValueTracker {
    /// Maps baggage key -> set of seen values.
    seen: std::collections::HashMap<String, std::collections::HashSet<String>>,
}

impl DistinctValueTracker {
    pub fn new() -> Self {
        Self {
            seen: std::collections::HashMap::new(),
        }
    }

    /// Canonicalize a baggage value for use as a metric label.
    ///
    /// If `max_distinct` is set and the number of distinct values for this key
    /// has reached the cap, the value is replaced with a deterministic hash.
    /// All values also go through control-character sanitization.
    pub fn canonicalize(&mut self, key: &str, value: &str, max_distinct: Option<usize>) -> String {
        let sanitized = sanitize_value(value);

        if let Some(cap) = max_distinct {
            let entry = self.seen.entry(key.to_string()).or_default();
            if !entry.contains(&sanitized) {
                if entry.len() >= cap {
                    // Cap reached: hash new values
                    return hash_value(&sanitized);
                }
                entry.insert(sanitized.clone());
            }
        }

        sanitized
    }
}

impl Default for DistinctValueTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Sanitize a value: trim, replace control characters.
fn sanitize_value(s: &str) -> String {
    let s = s.trim();
    s.chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

/// Hash a value deterministically.
fn hash_value(s: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    format!("hash_{:x}", hasher.finish())
}

/// Parse a raw W3C Baggage header value into key-value pairs.
///
/// Handles URL encoding, comma separation, and optional metadata (stripped).
/// Returns an empty Vec for None or empty input.
pub fn parse_baggage_string(raw: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for item in raw.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        // Split off metadata (after ';')
        let kv_part = if let Some(idx) = item.find(';') {
            &item[..idx]
        } else {
            item
        };
        let Some((key, value)) = kv_part.split_once('=') else {
            continue;
        };
        let key = key.trim().to_string();
        let value = match urlencoding::decode(value.trim()) {
            Ok(v) => v.into_owned(),
            Err(_) => continue,
        };
        if !key.is_empty() {
            result.push((key, value));
        }
    }
    result
}

/// Extract promoted baggage attributes from a raw baggage string.
///
/// Only returns key-value pairs for keys that are in `promotions` and match the
/// given `signal`. Values are URL-decoded.
pub fn extract_promoted_keys(
    raw: &str,
    promotions: &[BaggageKeyPromotion],
    signal: SignalSet,
) -> Vec<ExtractedBaggageAttr> {
    if promotions.is_empty() {
        return Vec::new();
    }
    let parsed = parse_baggage_string(raw);
    let mut result = Vec::new();
    for (key, value) in &parsed {
        for promo in promotions {
            if promo.baggage_key == *key && promo.applies_to(signal) {
                result.push(ExtractedBaggageAttr {
                    attribute_name: promo.effective_attribute_name().to_string(),
                    value: value.clone(),
                });
                break;
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_baggage() {
        let result = parse_baggage_string("key1=value1,key2=value2");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], ("key1".to_string(), "value1".to_string()));
        assert_eq!(result[1], ("key2".to_string(), "value2".to_string()));
    }

    #[test]
    fn parse_baggage_with_metadata() {
        let result = parse_baggage_string("key1=value1;properties,key2=value2");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].1, "value1");
    }

    #[test]
    fn parse_baggage_url_encoded() {
        let result = parse_baggage_string("key1=hello%20world");
        assert_eq!(result[0].1, "hello world");
    }

    #[test]
    fn parse_empty_baggage() {
        assert!(parse_baggage_string("").is_empty());
        assert!(parse_baggage_string("  ").is_empty());
    }

    #[test]
    fn parse_baggage_malformed() {
        // Missing value
        let result = parse_baggage_string("key1=");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, "");
    }

    #[test]
    fn parse_baggage_no_equals() {
        let result = parse_baggage_string("noequalssign");
        assert!(result.is_empty());
    }

    #[test]
    fn extract_promotes_matching_keys() {
        let promotions = vec![
            BaggageKeyPromotion {
                baggage_key: "tenant.id".to_string(),
                attribute_name: None,
                signals: None,
                max_distinct: None,
            },
            BaggageKeyPromotion {
                baggage_key: "user.role".to_string(),
                attribute_name: Some("ferron.user_role".to_string()),
                signals: None,
                max_distinct: None,
            },
        ];
        let extracted = extract_promoted_keys(
            "tenant.id=acme,user.role=admin,other=skip",
            &promotions,
            SignalSet::ALL,
        );
        assert_eq!(extracted.len(), 2);
        assert_eq!(extracted[0].attribute_name, "tenant.id");
        assert_eq!(extracted[0].value, "acme");
        assert_eq!(extracted[1].attribute_name, "ferron.user_role");
        assert_eq!(extracted[1].value, "admin");
    }

    #[test]
    fn extract_respects_signal_filter() {
        let promotions = vec![BaggageKeyPromotion {
            baggage_key: "tenant.id".to_string(),
            attribute_name: None,
            signals: Some(SignalSet::TRACES),
            max_distinct: None,
        }];
        let extracted_logs = extract_promoted_keys("tenant.id=acme", &promotions, SignalSet::LOGS);
        assert!(extracted_logs.is_empty());

        let extracted_traces =
            extract_promoted_keys("tenant.id=acme", &promotions, SignalSet::TRACES);
        assert_eq!(extracted_traces.len(), 1);
    }

    #[test]
    fn signal_set_basics() {
        assert!(SignalSet::ALL.contains(SignalSet::TRACES));
        assert!(SignalSet::ALL.contains(SignalSet::LOGS));
        assert!(SignalSet::ALL.contains(SignalSet::METRICS));
        assert!(!SignalSet::TRACES.contains(SignalSet::LOGS));
        assert!(SignalSet::TRACES
            .insert(SignalSet::LOGS)
            .contains(SignalSet::LOGS));
    }

    #[test]
    fn effective_attribute_name_fallback() {
        let promo = BaggageKeyPromotion {
            baggage_key: "key".to_string(),
            attribute_name: None,
            signals: None,
            max_distinct: None,
        };
        assert_eq!(promo.effective_attribute_name(), "key");

        let promo2 = BaggageKeyPromotion {
            baggage_key: "key".to_string(),
            attribute_name: Some("custom.name".to_string()),
            signals: None,
            max_distinct: None,
        };
        assert_eq!(promo2.effective_attribute_name(), "custom.name");
    }

    #[test]
    fn tracker_no_cap_passes_through() {
        let mut tracker = DistinctValueTracker::new();
        assert_eq!(tracker.canonicalize("key", "value1", None), "value1");
        assert_eq!(tracker.canonicalize("key", "value2", None), "value2");
    }

    #[test]
    fn tracker_respects_cap() {
        let mut tracker = DistinctValueTracker::new();
        assert_eq!(tracker.canonicalize("key", "v1", Some(2)), "v1");
        assert_eq!(tracker.canonicalize("key", "v2", Some(2)), "v2");
        // Third distinct value should be hashed
        let result = tracker.canonicalize("key", "v3", Some(2));
        assert!(result.starts_with("hash_"));
    }

    #[test]
    fn tracker_same_value_not_hashed() {
        let mut tracker = DistinctValueTracker::new();
        assert_eq!(tracker.canonicalize("key", "v1", Some(1)), "v1");
        // Same value again should not be hashed
        assert_eq!(tracker.canonicalize("key", "v1", Some(1)), "v1");
    }

    #[test]
    fn tracker_sanitizes_control_chars() {
        let mut tracker = DistinctValueTracker::new();
        assert_eq!(
            tracker.canonicalize("key", "hello\x00world", None),
            "hello?world"
        );
    }

    #[test]
    fn tracker_independent_per_key() {
        let mut tracker = DistinctValueTracker::new();
        assert_eq!(tracker.canonicalize("key1", "v1", Some(1)), "v1");
        assert_eq!(tracker.canonicalize("key2", "v1", Some(1)), "v1");
        // key2's v1 should not be affected by key1's cap
        assert_eq!(tracker.canonicalize("key2", "v1", Some(1)), "v1");
    }
}
