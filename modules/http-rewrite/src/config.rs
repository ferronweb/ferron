//! Configuration parsing for `rewrite` directives.
//!
//! Parses `rewrite <regex> <replacement> { ... }` entries from layered
//! configuration into typed `RewriteRule` structures.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationValue};
use regex::{Regex, RegexBuilder};

use crate::RewriteEngine;

/// A TTL cache for file/directory metadata lookups.
struct MetadataCache {
    cache: dashmap::DashMap<PathBuf, (bool, bool, std::time::Instant)>,
    ttl: Duration,
}

impl MetadataCache {
    fn new(ttl: Duration) -> Self {
        Self {
            cache: dashmap::DashMap::new(),
            ttl,
        }
    }

    fn get(&self, path: &Path) -> Option<(bool, bool)> {
        self.cache.get(path).and_then(|r| {
            let (is_file, is_dir, ts) = r.value();
            if ts.elapsed() < self.ttl {
                Some((*is_file, *is_dir))
            } else {
                None
            }
        })
    }

    fn insert(&self, path: PathBuf, is_file: bool, is_dir: bool) {
        // Periodic cleanup
        if self.cache.len() > 10_000 {
            let ttl = self.ttl;
            self.cache.retain(|_, (_, _, ts)| ts.elapsed() < ttl);
        }
        self.cache
            .insert(path, (is_file, is_dir, std::time::Instant::now()));
    }
}

/// Global shared metadata cache to avoid repeated filesystem lookups.
fn metadata_cache() -> &'static MetadataCache {
    static CACHE: std::sync::OnceLock<MetadataCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| MetadataCache::new(Duration::from_millis(100)))
}

/// A single URL rewrite rule parsed from configuration.
#[derive(Debug, Clone)]
pub struct RewriteRule {
    /// Compiled regex for matching the request URL.
    pub regex: Arc<Regex>,
    /// Replacement string (may contain capture group references like `$1`).
    pub replacement: String,
    /// Optional operator-chosen identifier, surfaced in observability output.
    /// Must be a plain (non-interpolated) string so metric labels stay bounded.
    pub name: Option<String>,
    /// Whether the rule applies when the path corresponds to a directory.
    pub is_directory: bool,
    /// Whether the rule applies when the path corresponds to a file.
    pub is_file: bool,
    /// Whether this is the last rule to apply when it matches.
    pub last: bool,
    /// Whether double slashes are allowed in the rewritten URL.
    pub allow_double_slashes: bool,
}

/// Default values for rewrite rule options.
impl RewriteRule {
    const DEFAULT_DIRECTORY: bool = true;
    const DEFAULT_FILE: bool = true;
    const DEFAULT_LAST: bool = false;
    const DEFAULT_ALLOW_DOUBLE_SLASHES: bool = false;
}

/// Parse all `rewrite` directives from the layered configuration.
///
/// Each `rewrite <regex> <replacement> { ... }` becomes a `RewriteRule`.
/// If no `rewrite` entries are present, returns an empty vec.
pub fn parse_rewrite_config(
    config: &LayeredConfiguration,
    engine: &RewriteEngine,
) -> Vec<RewriteRule> {
    let mut rules = Vec::new();
    let entries = config.get_entries("rewrite", true);

    for entry in entries {
        if let Some(rule) = parse_rewrite_entry(entry, engine) {
            rules.push(rule);
        }
    }

    rules
}

/// Parse a single `rewrite` directive entry into a `RewriteRule`.
fn parse_rewrite_entry(
    entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
    engine: &RewriteEngine,
) -> Option<RewriteRule> {
    if entry.args.len() < 2 {
        return None;
    }

    let regex_str = entry.args[0].as_str()?;
    let replacement = entry.args[1].as_str()?.to_string();

    // Cached regex for performance
    let regex = if let Some(cached) = engine.compiled_regexes.get(regex_str) {
        cached.clone()
    } else {
        let regex = Arc::new(
            RegexBuilder::new(regex_str)
                .case_insensitive(cfg!(windows))
                .build()
                .ok()?,
        );
        engine
            .compiled_regexes
            .insert(regex_str.to_string(), regex.clone());
        regex
    };

    let (is_directory, is_file, last, allow_double_slashes) =
        if let Some(children) = &entry.children {
            parse_rewrite_options(children)
        } else {
            (
                RewriteRule::DEFAULT_DIRECTORY,
                RewriteRule::DEFAULT_FILE,
                RewriteRule::DEFAULT_LAST,
                RewriteRule::DEFAULT_ALLOW_DOUBLE_SLASHES,
            )
        };

    // `name` must be a plain string (interpolated values are rejected by the
    // validator), so it is safe to reuse as a metric label.
    let name = entry
        .children
        .as_ref()
        .and_then(|children| children.get_value("name"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    Some(RewriteRule {
        regex,
        replacement,
        name,
        is_directory,
        is_file,
        last,
        allow_double_slashes,
    })
}

/// Parse optional block options inside a `rewrite { ... }` block.
fn parse_rewrite_options(block: &ServerConfigurationBlock) -> (bool, bool, bool, bool) {
    let is_directory = block
        .get_value("directory")
        .and_then(|v| match v {
            ServerConfigurationValue::Boolean(b, _) => Some(*b),
            _ => None,
        })
        .unwrap_or(RewriteRule::DEFAULT_DIRECTORY);

    let is_file = block
        .get_value("file")
        .and_then(|v| match v {
            ServerConfigurationValue::Boolean(b, _) => Some(*b),
            _ => None,
        })
        .unwrap_or(RewriteRule::DEFAULT_FILE);

    let last = block
        .get_value("last")
        .and_then(|v| match v {
            ServerConfigurationValue::Boolean(b, _) => Some(*b),
            _ => None,
        })
        .unwrap_or(RewriteRule::DEFAULT_LAST);

    let allow_double_slashes = block
        .get_value("allow_double_slashes")
        .and_then(|v| match v {
            ServerConfigurationValue::Boolean(b, _) => Some(*b),
            _ => None,
        })
        .unwrap_or(RewriteRule::DEFAULT_ALLOW_DOUBLE_SLASHES);

    (is_directory, is_file, last, allow_double_slashes)
}

/// Check whether `rewrite_log` is enabled in the layered configuration.
pub fn is_rewrite_log_enabled(config: &LayeredConfiguration) -> bool {
    config
        .get_value("rewrite_log", true)
        .and_then(|v| match v {
            ServerConfigurationValue::Boolean(b, _) => Some(*b),
            _ => None,
        })
        .unwrap_or(false)
}

/// Resolve the filesystem path from a URL path and the configured root directory.
/// Returns the joined path and (is_file, is_directory) metadata.
async fn resolve_path_metadata(url_path: &str, root: &str) -> (PathBuf, Option<(bool, bool)>) {
    let mut relative = url_path.trim_start_matches('/');
    // Strip query string
    if let Some(pos) = relative.find('?') {
        relative = &relative[..pos];
    }

    let joined = Path::new(root).join(relative);
    let cache = metadata_cache();
    if let Some(meta) = cache.get(&joined) {
        return (joined, Some(meta));
    }

    // Spawn a blocking metadata lookup
    let result = zincio::fs::metadata(&joined).await;
    let meta = result.ok().map(|m| (m.is_file(), m.is_dir()));
    if let Some((is_file, is_dir)) = meta {
        cache.insert(joined.clone(), is_file, is_dir);
    }

    (joined, meta)
}

/// One rewrite rule firing within a request: which rule ran and what it did.
#[derive(Debug, PartialEq, Clone)]
pub struct RewriteStep {
    /// 0-based position of the rule among the evaluated rules.
    pub rule_index: usize,
    /// URL before this rule ran.
    pub from: String,
    /// URL after this rule ran.
    pub to: String,
}

/// Result of applying rewrite rules.
#[derive(Debug, PartialEq)]
pub enum RewriteResult {
    /// No rules matched the URL.
    NoMatch,
    /// URL was successfully rewritten, with one step per rule that fired.
    Rewritten {
        url: String,
        steps: Vec<RewriteStep>,
    },
    /// A rule matched but produced an invalid URL (missing leading `/`),
    /// with the 0-based position of the offending rule.
    InvalidRewrite { rule_index: usize },
}

/// Apply rewrite rules to a URL, returning the result.
pub async fn apply_rewrite_rules(
    url: &str,
    rules: &[RewriteRule],
    root: Option<&str>,
) -> RewriteResult {
    let mut rewritten = url.to_string();
    let mut steps = Vec::new();

    for (rule_index, rule) in rules.iter().enumerate() {
        if !rule.allow_double_slashes {
            while rewritten.contains("//") {
                rewritten = rewritten.replace("//", "/");
            }
        }

        if !rule.is_file || !rule.is_directory {
            if let Some(root) = root {
                let (_joined, metadata) = resolve_path_metadata(&rewritten, root).await;
                let (is_file, is_directory) = metadata.unwrap_or((false, false));

                if (!rule.is_file && is_file) || (!rule.is_directory && is_directory) {
                    continue;
                }
            }
        }

        let old = rewritten.clone();
        rewritten = rule
            .regex
            .replace(&rewritten, &rule.replacement)
            .to_string();

        if !rewritten.starts_with('/') {
            return RewriteResult::InvalidRewrite { rule_index };
        }

        let matched_this_rule = old != rewritten;
        if matched_this_rule {
            steps.push(RewriteStep {
                rule_index,
                from: old,
                to: rewritten.clone(),
            });
        }

        if rule.last && matched_this_rule {
            break;
        }
    }

    if steps.is_empty() {
        RewriteResult::NoMatch
    } else {
        RewriteResult::Rewritten {
            url: rewritten,
            steps,
        }
    }
}
