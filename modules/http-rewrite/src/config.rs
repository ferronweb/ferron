//! Configuration parsing for `rewrite` directives.
//!
//! Parses `rewrite <regex> <replacement> { ... }` entries from layered
//! configuration into typed `RewriteRule` structures.

use std::path::{Path, PathBuf};
use std::time::Duration;

use fancy_regex::{Regex, RegexBuilder};
use ferron_core::config::{
    layer::LayeredConfiguration, ServerConfigurationBlock, ServerConfigurationValue,
};

/// A TTL cache for file/directory metadata lookups.
struct MetadataCache {
    cache: parking_lot::Mutex<std::collections::HashMap<PathBuf, (bool, bool, std::time::Instant)>>,
    ttl: Duration,
}

impl MetadataCache {
    fn new(ttl: Duration) -> Self {
        Self {
            cache: parking_lot::Mutex::new(std::collections::HashMap::new()),
            ttl,
        }
    }

    fn get(&self, path: &Path) -> Option<(bool, bool)> {
        let guard = self.cache.lock();
        guard.get(path).and_then(|(is_file, is_dir, ts)| {
            if ts.elapsed() < self.ttl {
                Some((*is_file, *is_dir))
            } else {
                None
            }
        })
    }

    fn insert(&self, path: PathBuf, is_file: bool, is_dir: bool) {
        let mut guard = self.cache.lock();
        // Periodic cleanup
        if guard.len() > 10_000 {
            let ttl = self.ttl;
            guard.retain(|_, (_, _, ts)| ts.elapsed() < ttl);
        }
        guard.insert(path, (is_file, is_dir, std::time::Instant::now()));
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
    pub regex: Regex,
    /// Replacement string (may contain capture group references like `$1`).
    pub replacement: String,
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
pub fn parse_rewrite_config(config: &LayeredConfiguration) -> Vec<RewriteRule> {
    let mut rules = Vec::new();
    let entries = config.get_entries("rewrite", true);

    for entry in entries {
        if let Some(rule) = parse_rewrite_entry(entry) {
            rules.push(rule);
        }
    }

    rules
}

/// Parse a single `rewrite` directive entry into a `RewriteRule`.
fn parse_rewrite_entry(
    entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
) -> Option<RewriteRule> {
    if entry.args.len() < 2 {
        return None;
    }

    let regex_str = entry.args[0].as_str()?;
    let replacement = entry.args[1].as_str()?.to_string();

    let regex = RegexBuilder::new(regex_str)
        .case_insensitive(cfg!(windows))
        .build()
        .ok()?;

    // Parse optional block options
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

    Some(RewriteRule {
        regex,
        replacement,
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
fn resolve_path_metadata(url_path: &str, root: &str) -> (PathBuf, Option<(bool, bool)>) {
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
    let result = std::fs::metadata(&joined);
    let meta = result.ok().map(|m| (m.is_file(), m.is_dir()));
    if let Some((is_file, is_dir)) = meta {
        cache.insert(joined.clone(), is_file, is_dir);
    }

    (joined, meta)
}

/// Result of applying rewrite rules.
#[derive(Debug, PartialEq)]
pub enum RewriteResult {
    /// No rules matched the URL.
    NoMatch,
    /// URL was successfully rewritten to the given value.
    Rewritten(String),
    /// A rule matched but produced an invalid URL (missing leading `/`).
    InvalidRewrite,
}

/// Apply rewrite rules to a URL, returning the result.
pub fn apply_rewrite_rules(url: &str, rules: &[RewriteRule], root: Option<&str>) -> RewriteResult {
    let mut rewritten = url.to_string();
    let mut any_rule_matched = false;

    for rule in rules {
        // Normalize double slashes if not allowed
        if !rule.allow_double_slashes {
            while rewritten.contains("//") {
                rewritten = rewritten.replace("//", "/");
            }
        }

        // Check file/directory constraints
        if !rule.is_file || !rule.is_directory {
            if let Some(root) = root {
                let (joined, metadata) = resolve_path_metadata(&rewritten, root);

                let (is_file, is_directory) = match metadata {
                    Some((f, d)) => (f, d),
                    None => (false, false),
                };

                // Skip if constraint says "don't apply for files" and it IS a file
                if !rule.is_file && is_file {
                    continue;
                }
                // Skip if constraint says "don't apply for directories" and it IS a directory
                if !rule.is_directory && is_directory {
                    continue;
                }

                // Suppress unused variable warning
                let _ = joined;
            }
        }

        let old = rewritten.clone();
        rewritten = rule
            .regex
            .replace(&rewritten, &rule.replacement)
            .to_string();

        // Validate rewritten URL starts with '/'
        if !rewritten.starts_with('/') {
            return RewriteResult::InvalidRewrite;
        }

        if old != rewritten {
            any_rule_matched = true;
        }

        if rule.last && old != rewritten {
            break;
        }
    }

    if any_rule_matched {
        RewriteResult::Rewritten(rewritten)
    } else {
        RewriteResult::NoMatch
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{ServerConfigurationDirectiveEntry, ServerConfigurationValue};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_entry(
        args: Vec<ServerConfigurationValue>,
        children: Option<ServerConfigurationBlock>,
    ) -> ServerConfigurationDirectiveEntry {
        ServerConfigurationDirectiveEntry {
            args,
            children,
            span: None,
        }
    }

    fn make_value_string(s: &str) -> ServerConfigurationValue {
        ServerConfigurationValue::String(s.to_string(), None)
    }

    fn make_value_bool(b: bool) -> ServerConfigurationValue {
        ServerConfigurationValue::Boolean(b, None)
    }

    fn make_options_block(
        directory: Option<bool>,
        file: Option<bool>,
        last: Option<bool>,
        allow_double_slashes: Option<bool>,
    ) -> ServerConfigurationBlock {
        let mut directives = HashMap::new();
        if let Some(v) = directory {
            directives.insert(
                "directory".to_string(),
                vec![make_entry(vec![make_value_bool(v)], None)],
            );
        }
        if let Some(v) = file {
            directives.insert(
                "file".to_string(),
                vec![make_entry(vec![make_value_bool(v)], None)],
            );
        }
        if let Some(v) = last {
            directives.insert(
                "last".to_string(),
                vec![make_entry(vec![make_value_bool(v)], None)],
            );
        }
        if let Some(v) = allow_double_slashes {
            directives.insert(
                "allow_double_slashes".to_string(),
                vec![make_entry(vec![make_value_bool(v)], None)],
            );
        }
        ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        }
    }

    #[test]
    fn parses_simple_rewrite_rule() {
        let entry = make_entry(
            vec![
                make_value_string("^/old/(.*)"),
                make_value_string("/new/$1"),
            ],
            None,
        );
        let rule = parse_rewrite_entry(&entry).unwrap();
        assert_eq!(rule.replacement, "/new/$1");
        assert!(rule.is_directory);
        assert!(rule.is_file);
        assert!(!rule.last);
        assert!(!rule.allow_double_slashes);
    }

    #[test]
    fn parses_rewrite_rule_with_options() {
        let opts = make_options_block(None, None, Some(true), None);
        let entry = make_entry(
            vec![
                make_value_string("^/api/v1/(.*)"),
                make_value_string("/api/v2/$1"),
            ],
            Some(opts),
        );
        let rule = parse_rewrite_entry(&entry).unwrap();
        assert!(rule.last);
        assert!(!rule.allow_double_slashes);
    }

    #[test]
    fn applies_rewrite_rule() {
        let rules = vec![RewriteRule {
            regex: Regex::new("^/old/(.*)").unwrap(),
            replacement: "/new/$1".to_string(),
            is_directory: true,
            is_file: true,
            last: false,
            allow_double_slashes: false,
        }];
        let result = apply_rewrite_rules("/old/path?query=1", &rules, None);
        assert_eq!(
            result,
            RewriteResult::Rewritten("/new/path?query=1".to_string())
        );
    }

    #[test]
    fn applies_last_flag_stops_further_rules() {
        let rules = vec![
            RewriteRule {
                regex: Regex::new("^/a/(.*)").unwrap(),
                replacement: "/b/$1".to_string(),
                is_directory: true,
                is_file: true,
                last: true,
                allow_double_slashes: false,
            },
            RewriteRule {
                regex: Regex::new("^/b/(.*)").unwrap(),
                replacement: "/c/$1".to_string(),
                is_directory: true,
                is_file: true,
                last: false,
                allow_double_slashes: false,
            },
        ];
        let result = apply_rewrite_rules("/a/test", &rules, None);
        // First rule matches and is last, so second rule doesn't apply
        assert_eq!(result, RewriteResult::Rewritten("/b/test".to_string()));
    }

    #[test]
    fn chained_rules_without_last() {
        let rules = vec![
            RewriteRule {
                regex: Regex::new("^/a/(.*)").unwrap(),
                replacement: "/b/$1".to_string(),
                is_directory: true,
                is_file: true,
                last: false,
                allow_double_slashes: false,
            },
            RewriteRule {
                regex: Regex::new("^/b/(.*)").unwrap(),
                replacement: "/c/$1".to_string(),
                is_directory: true,
                is_file: true,
                last: false,
                allow_double_slashes: false,
            },
        ];
        let result = apply_rewrite_rules("/a/test", &rules, None);
        // Both rules chain
        assert_eq!(result, RewriteResult::Rewritten("/c/test".to_string()));
    }

    #[test]
    fn returns_invalid_for_bad_replacement() {
        let rules = vec![RewriteRule {
            regex: Regex::new(".*").unwrap(),
            replacement: "no-leading-slash".to_string(),
            is_directory: true,
            is_file: true,
            last: false,
            allow_double_slashes: false,
        }];
        let result = apply_rewrite_rules("/anything", &rules, None);
        assert!(matches!(result, RewriteResult::InvalidRewrite));
    }
}
