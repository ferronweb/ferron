use ferron_core::config::ServerConfigurationBlock;
use ferron_observability::baggage::{BaggageKeyPromotion, SignalSet};

/// Log style for OTLP log records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogStyle {
    /// Legacy behavior: emit `event.message` as the log body verbatim and do not
    /// publish per-event attributes to the OTLP record. The configured `format`
    /// directive (if any) is honored as before.
    #[default]
    Legacy,
    /// Modern (OTEL-friendly) behavior: emit a short `summary` plus typed
    /// `attributes` instead of the human-readable `message`. The `format`
    /// directive is ignored for log records in this mode.
    Modern,
}

/// Parse a `log_style` directive value into a [`LogStyle`]. Returns `None` if
/// the value is not a recognized log style.
pub fn parse_log_style(value: &str) -> Option<LogStyle> {
    match value.to_ascii_lowercase().as_str() {
        "legacy" => Some(LogStyle::Legacy),
        "modern" => Some(LogStyle::Modern),
        _ => None,
    }
}

/// Sampling mode for traces.
#[derive(Debug, Clone)]
pub enum TraceSamplingMode {
    /// Sample every trace.
    AlwaysOn,
    /// Sample no traces.
    AlwaysOff,
    /// Respect the parent span's sampling decision; AlwaysOn for root spans.
    ParentBasedAlwaysOn,
    /// Sample a fixed ratio of traces based on trace ID.
    TraceIdRatioBased { ratio: f64 },
    /// Parent-based with TraceIdRatioBased for root spans.
    ParentBasedTraceIdRatio { ratio: f64 },
    /// Sample based on span attributes set before the span is built.
    AttributeBased { rules: Vec<AttributeSamplingRule> },
}

/// A rule for attribute-based sampling.
#[derive(Debug, Clone)]
pub struct AttributeSamplingRule {
    /// The attribute key to match against.
    pub attribute: String,
    /// How to match the attribute value.
    pub matcher: AttributeMatcher,
}

/// Matcher for attribute-based sampling rules.
#[derive(Debug, Clone)]
pub enum AttributeMatcher {
    /// Exact string match.
    Exact(String),
    /// Prefix match.
    Prefix(String),
    /// Match if the attribute exists (any value).
    Exists,
}

/// Trace sampling configuration.
#[derive(Debug, Clone)]
pub struct TraceSamplingConfig {
    pub mode: TraceSamplingMode,
}

impl Default for TraceSamplingConfig {
    fn default() -> Self {
        Self {
            mode: TraceSamplingMode::ParentBasedAlwaysOn,
        }
    }
}

/// Per-host configuration for a single OTLP signal (logs, metrics, or traces)
pub struct SignalConfig {
    pub endpoint: String,
    pub protocol: String,
    pub authorization: Option<String>,
    pub sampling: TraceSamplingConfig,
}

/// Shared configuration for an OTLP backend instance
pub struct OtlpBackendConfig {
    pub service_name: String,
    pub no_verify: bool,
    pub logs: Option<SignalConfig>,
    pub metrics: Option<SignalConfig>,
    pub traces: Option<SignalConfig>,
    pub baggage_promotions: Vec<BaggageKeyPromotion>,
    pub log_style: LogStyle,
}

impl OtlpBackendConfig {
    /// Parse the OTLP backend configuration from a ServerConfigurationBlock
    pub fn parse_config(config: &ServerConfigurationBlock) -> Self {
        let service_name = config
            .get_value("service_name")
            .and_then(|v| v.as_str())
            .unwrap_or("ferron")
            .to_string();

        let no_verify = config.get_flag("no_verification");

        let logs = SignalConfig::parse_config(config, "logs");
        let metrics = SignalConfig::parse_config(config, "metrics");
        let traces = SignalConfig::parse_config(config, "traces");
        let baggage_promotions = parse_baggage_promotions(config);
        let log_style = config
            .get_value("log_style")
            .and_then(|v| v.as_str())
            .and_then(parse_log_style)
            .unwrap_or_default();

        Self {
            service_name,
            no_verify,
            logs,
            metrics,
            traces,
            baggage_promotions,
            log_style,
        }
    }
}

impl SignalConfig {
    /// Parse a single signal sub-block (logs, metrics, or traces)
    fn parse_config(parent: &ServerConfigurationBlock, name: &str) -> Option<SignalConfig> {
        let entries = parent.directives.get(name)?;
        let entry = entries.first()?;
        let endpoint = entry.args.first().and_then(|v| v.as_str())?.to_string();

        let children = entry.children.as_ref()?;

        let protocol = children
            .get_value("protocol")
            .and_then(|v| v.as_str())
            .unwrap_or("grpc")
            .to_string();

        let authorization = children
            .get_value("authorization")
            .and_then(|v| v.as_str())
            .map(|s: &str| s.to_string());

        let sampling = if name == "traces" {
            parse_trace_sampling(children)
        } else {
            TraceSamplingConfig::default()
        };

        Some(Self {
            endpoint,
            protocol,
            authorization,
            sampling,
        })
    }
}

/// Parse the `sampling` directive from a traces signal block.
///
/// Expected format:
/// ```text
/// sampling "parentbased_traceidratio" {
///     ratio 0.1
/// }
/// ```
fn parse_trace_sampling(children: &ServerConfigurationBlock) -> TraceSamplingConfig {
    let Some(sampling_entries) = children.directives.get("sampling") else {
        return TraceSamplingConfig::default();
    };
    let Some(entry) = sampling_entries.first() else {
        return TraceSamplingConfig::default();
    };

    let mode = match entry.args.first().and_then(|v| v.as_str()) {
        Some("always_on") => TraceSamplingMode::AlwaysOn,
        Some("always_off") => TraceSamplingMode::AlwaysOff,
        Some("parentbased_always_on") => TraceSamplingMode::ParentBasedAlwaysOn,
        Some("traceidratio") => {
            let ratio = entry
                .children
                .as_ref()
                .and_then(|c| c.get_value("ratio"))
                .and_then(|v| v.as_float())
                .unwrap_or(1.0);
            TraceSamplingMode::TraceIdRatioBased { ratio }
        }
        Some("parentbased_traceidratio") => {
            let ratio = entry
                .children
                .as_ref()
                .and_then(|c| c.get_value("ratio"))
                .and_then(|v| v.as_float())
                .unwrap_or(1.0);
            TraceSamplingMode::ParentBasedTraceIdRatio { ratio }
        }
        Some("attribute_based") => {
            let rules = entry
                .children
                .as_ref()
                .map(parse_attribute_sampling_rules)
                .unwrap_or_default();
            TraceSamplingMode::AttributeBased { rules }
        }
        _ => return TraceSamplingConfig::default(),
    };

    TraceSamplingConfig { mode }
}

/// Parse attribute sampling rules from a `rules { ... }` block.
fn parse_attribute_sampling_rules(
    children: &ServerConfigurationBlock,
) -> Vec<AttributeSamplingRule> {
    let Some(rules_entries) = children.directives.get("rules") else {
        return Vec::new();
    };
    let Some(rules_block) = rules_entries.first().and_then(|e| e.children.as_ref()) else {
        return Vec::new();
    };

    let Some(rule_entries) = rules_block.directives.get("rule") else {
        return Vec::new();
    };

    let mut rules = Vec::new();
    for entry in rule_entries {
        let Some(match_type) = entry.args.first().and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(attribute) = entry.args.get(1).and_then(|v| v.as_str()) else {
            continue;
        };

        let matcher = match match_type {
            "exact" => {
                let Some(value) = entry.args.get(2).and_then(|v| v.as_str()) else {
                    continue;
                };
                AttributeMatcher::Exact(value.to_string())
            }
            "prefix" => {
                let Some(value) = entry.args.get(2).and_then(|v| v.as_str()) else {
                    continue;
                };
                AttributeMatcher::Prefix(value.to_string())
            }
            "exists" => AttributeMatcher::Exists,
            _ => continue,
        };

        rules.push(AttributeSamplingRule {
            attribute: attribute.to_string(),
            matcher,
        });
    }

    rules
}

/// Parse the `baggage` directive from the OTLP config block.
///
/// Expected format:
/// ```text
/// baggage {
///     key "tenant.id" {
///         attribute "tenant.id"
///         signals traces logs
///         max_distinct 1000
///     }
/// }
/// ```
fn parse_baggage_promotions(config: &ServerConfigurationBlock) -> Vec<BaggageKeyPromotion> {
    let Some(baggage_entries) = config.directives.get("baggage") else {
        return Vec::new();
    };
    let Some(baggage_block) = baggage_entries.first().and_then(|e| e.children.as_ref()) else {
        return Vec::new();
    };

    let Some(key_entries) = baggage_block.directives.get("key") else {
        return Vec::new();
    };

    let mut promotions = Vec::new();
    for key_entry in key_entries {
        let Some(baggage_key) = key_entry.args.first().and_then(|v| v.as_str()) else {
            continue;
        };

        let children = key_entry.children.as_ref();

        let attribute_name = children
            .and_then(|c| c.get_value("attribute"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let signals = children.and_then(parse_signal_set);

        let max_distinct = children
            .and_then(|c| c.get_value("max_distinct"))
            .and_then(|v| v.as_number())
            .map(|n| n as usize);

        promotions.push(BaggageKeyPromotion {
            baggage_key: baggage_key.to_string(),
            attribute_name,
            signals,
            max_distinct,
        });
    }

    promotions
}

/// Parse a `signals` directive value into a SignalSet.
/// The value can be a single signal name or multiple args.
fn parse_signal_set(children: &ServerConfigurationBlock) -> Option<SignalSet> {
    let entries = children.directives.get("signals")?;
    let entry = entries.first()?;
    if entry.args.is_empty() {
        return None;
    }
    let mut set = SignalSet::empty();
    for arg in &entry.args {
        if let Some(name) = arg.as_str() {
            match name {
                "traces" => set = set.insert(SignalSet::TRACES),
                "logs" => set = set.insert(SignalSet::LOGS),
                "metrics" => set = set.insert(SignalSet::METRICS),
                _ => {}
            }
        }
    }
    if set == SignalSet::empty() {
        None
    } else {
        Some(set)
    }
}
