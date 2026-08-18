//! Configuration parsing for the `canary` directive.

use std::time::Duration;

use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::config::ServerConfigurationDirectiveEntry;

/// SameSite attribute mode for the canary affinity cookie.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SameSiteMode {
    Strict,
    #[default]
    Lax,
    None,
}

impl SameSiteMode {
    /// Return the string representation for the `Set-Cookie` header.
    #[inline]
    pub fn as_str(&self) -> &'static str {
        match self {
            SameSiteMode::Strict => "Strict",
            SameSiteMode::Lax => "Lax",
            SameSiteMode::None => "None",
        }
    }
}

/// Cookie attribute configuration for the canary affinity cookie.
///
/// Instead of a browser-session cookie, Ferron emits a persistent cookie by
/// default (see [`CookieConfig::default`]), so the canary assignment survives
/// a browser restart. Every attribute can be overridden through the `cookie`
/// block inside a `canary` directive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CookieConfig {
    /// Cookie TTL (`None` = browser-session cookie).
    pub ttl: Option<Duration>,
    /// Cookie path.
    pub path: String,
    /// Optional cookie domain.
    pub domain: Option<String>,
    /// Whether to set the `Secure` flag.
    pub secure: bool,
    /// Whether to set the `HttpOnly` flag.
    pub httponly: bool,
    /// SameSite attribute.
    pub samesite: SameSiteMode,
}

impl Default for CookieConfig {
    #[inline]
    fn default() -> Self {
        Self {
            ttl: Some(Duration::from_secs(7 * 24 * 3600)),
            path: "/".to_string(),
            domain: None,
            secure: false,
            httponly: true,
            samesite: SameSiteMode::Lax,
        }
    }
}

/// Affinity source for canary variant selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanaryAffinity {
    /// Hash the client IP address.
    Ip,
    /// Hash the value of the named request cookie.
    Cookie(String),
    /// Hash the value of the named request header.
    Header(String),
    /// Hash the resolved value of the named variable.
    Hash(String),
}

/// A canary variant with its configured weight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanaryVariant {
    pub name: String,
    pub weight: u32,
}

/// Parsed `canary` directive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanaryConfig {
    /// The canary name (e.g. `ab_test`).
    pub name: String,
    /// Where the sticky key comes from.
    pub affinity: CanaryAffinity,
    /// Whether Ferron sets the affinity cookie itself when the request has none.
    pub set_cookie: bool,
    /// Cookie attributes for the affinity cookie. Ferron emits a persistent
    /// cookie by default; this controls its lifetime and other attributes.
    pub cookie: CookieConfig,
    /// Configured variants; the ring maps every key to one of them.
    pub variants: Vec<CanaryVariant>,
}

/// Parse a single `canary` directive entry.
///
/// Returns `None` when the entry is malformed; the configuration validator
/// reports such configurations at load time.
pub fn parse_canary_entry(entry: &ServerConfigurationDirectiveEntry) -> Option<CanaryConfig> {
    let name = entry.args.first()?.as_str()?.to_string();
    let block = entry.children.as_ref()?;

    let mut affinity = CanaryAffinity::Ip;
    if let Some(affinity_entries) = block.directives.get("affinity") {
        if let Some(affinity_entry) = affinity_entries.first() {
            if let Some(keyword) = affinity_entry.args.first().and_then(|a| a.as_str()) {
                match keyword {
                    "ip" => {}
                    "cookie" | "header" | "hash" => {
                        if let Some(value) = affinity_entry.args.get(1).and_then(|a| a.as_str()) {
                            affinity = match keyword {
                                "cookie" => CanaryAffinity::Cookie(value.to_string()),
                                "header" => CanaryAffinity::Header(value.to_lowercase()),
                                _ => CanaryAffinity::Hash(value.to_string()),
                            };
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let mut variants = Vec::new();
    if let Some(variant_entries) = block.directives.get("variant") {
        for variant_entry in variant_entries {
            let Some(variant_name) = variant_entry.args.first().and_then(|a| a.as_str()) else {
                continue;
            };
            let Some(weight) = variant_entry.args.get(1).and_then(|a| a.as_number()) else {
                continue;
            };
            if weight < 1 {
                continue;
            }
            variants.push(CanaryVariant {
                name: variant_name.to_string(),
                weight: weight as u32,
            });
        }
    }

    let set_cookie = block
        .directives
        .get("set_cookie")
        .and_then(|entries| entries.first())
        .map(|entry| entry.get_flag())
        .unwrap_or(false);

    let cookie = parse_cookie_config(block);

    Some(CanaryConfig {
        name,
        affinity,
        set_cookie,
        cookie,
        variants,
    })
}

/// Parse the `cookie { ... }` sub-block of a `canary` directive.
///
/// Starts from the persistent default and overrides only the attributes the
/// user explicitly set, so an omitted `ttl` still yields a long-lived cookie.
fn parse_cookie_config(block: &ferron_core::config::ServerConfigurationBlock) -> CookieConfig {
    let mut cookie = CookieConfig::default();

    let Some(cookie_entries) = block.directives.get("cookie") else {
        return cookie;
    };

    for cookie_entry in cookie_entries {
        let Some(cookie_block) = &cookie_entry.children else {
            continue;
        };

        for (key, entries) in cookie_block.directives.iter() {
            let Some(entry) = entries.first() else {
                continue;
            };
            match key.as_str() {
                "ttl" => {
                    if let Some(val) = entry.args.first().and_then(|v| v.as_duration()) {
                        cookie.ttl = Some(val);
                    }
                }
                "path" => {
                    if let Some(val) = entry.args.first().and_then(|v| v.as_str()) {
                        cookie.path = val.to_string();
                    }
                }
                "domain" => {
                    if let Some(val) = entry.args.first().and_then(|v| v.as_str()) {
                        cookie.domain = Some(val.to_string());
                    }
                }
                "secure" => {
                    cookie.secure = entry.get_flag();
                }
                "httponly" => {
                    cookie.httponly = entry.get_flag();
                }
                "samesite" => {
                    if let Some(val) = entry.args.first().and_then(|v| v.as_str()) {
                        cookie.samesite = match val.to_lowercase().as_str() {
                            "strict" => SameSiteMode::Strict,
                            "lax" => SameSiteMode::Lax,
                            "none" => SameSiteMode::None,
                            _ => SameSiteMode::Lax,
                        };
                    }
                }
                _ => {}
            }
        }
    }

    cookie
}

/// Parse all `canary` directives from the layered configuration.
pub fn parse_canary_config(config: &LayeredConfiguration) -> Vec<CanaryConfig> {
    config
        .get_entries("canary", true)
        .into_iter()
        .filter_map(parse_canary_entry)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    };
    use std::collections::HashMap as StdHashMap;
    use std::sync::Arc;

    fn make_value_string(s: &str) -> ServerConfigurationValue {
        ServerConfigurationValue::String(s.to_string(), None)
    }

    fn make_value_number(n: i64) -> ServerConfigurationValue {
        ServerConfigurationValue::Number(n, None)
    }

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

    fn make_block(
        directives: StdHashMap<String, Vec<ServerConfigurationDirectiveEntry>>,
    ) -> ServerConfigurationBlock {
        ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: StdHashMap::new(),
            span: None,
        }
    }

    #[test]
    fn parses_full_canary_config() {
        let mut block_directives = StdHashMap::new();
        block_directives.insert(
            "affinity".to_string(),
            vec![make_entry(
                vec![make_value_string("cookie"), make_value_string("ab_variant")],
                None,
            )],
        );
        block_directives.insert(
            "variant".to_string(),
            vec![
                make_entry(
                    vec![make_value_string("stable"), make_value_number(90)],
                    None,
                ),
                make_entry(vec![make_value_string("new"), make_value_number(10)], None),
            ],
        );

        let entry = make_entry(
            vec![make_value_string("ab_test")],
            Some(make_block(block_directives)),
        );

        let config = parse_canary_entry(&entry).unwrap();
        assert_eq!(config.name, "ab_test");
        assert_eq!(
            config.affinity,
            CanaryAffinity::Cookie("ab_variant".to_string())
        );
        assert_eq!(config.variants.len(), 2);
        assert_eq!(config.variants[0].name, "stable");
        assert_eq!(config.variants[0].weight, 90);
        assert_eq!(config.variants[1].name, "new");
        assert_eq!(config.variants[1].weight, 10);
    }

    #[test]
    fn parses_ip_affinity_and_header_affinity() {
        let mut block_directives = StdHashMap::new();
        block_directives.insert(
            "affinity".to_string(),
            vec![make_entry(vec![make_value_string("ip")], None)],
        );
        block_directives.insert(
            "variant".to_string(),
            vec![make_entry(
                vec![make_value_string("a"), make_value_number(1)],
                None,
            )],
        );

        let ip_entry = make_entry(
            vec![make_value_string("x")],
            Some(make_block(block_directives.clone())),
        );
        assert_eq!(
            parse_canary_entry(&ip_entry).unwrap().affinity,
            CanaryAffinity::Ip
        );

        block_directives.insert(
            "affinity".to_string(),
            vec![make_entry(
                vec![
                    make_value_string("header"),
                    make_value_string("X-Experiment"),
                ],
                None,
            )],
        );
        let header_entry = make_entry(
            vec![make_value_string("x")],
            Some(make_block(block_directives)),
        );
        assert_eq!(
            parse_canary_entry(&header_entry).unwrap().affinity,
            CanaryAffinity::Header("x-experiment".to_string())
        );
    }

    #[test]
    fn parses_hash_affinity() {
        let mut block_directives = StdHashMap::new();
        block_directives.insert(
            "affinity".to_string(),
            vec![make_entry(
                vec![
                    make_value_string("hash"),
                    make_value_string("request.uri.query.bucket"),
                ],
                None,
            )],
        );
        block_directives.insert(
            "variant".to_string(),
            vec![make_entry(
                vec![make_value_string("a"), make_value_number(1)],
                None,
            )],
        );

        let entry = make_entry(
            vec![make_value_string("x")],
            Some(make_block(block_directives)),
        );
        assert_eq!(
            parse_canary_entry(&entry).unwrap().affinity,
            CanaryAffinity::Hash("request.uri.query.bucket".to_string())
        );
    }

    #[test]
    fn parses_set_cookie_flag() {
        let mut block_directives = StdHashMap::new();
        block_directives.insert(
            "affinity".to_string(),
            vec![make_entry(
                vec![make_value_string("cookie"), make_value_string("ab_variant")],
                None,
            )],
        );
        block_directives.insert("set_cookie".to_string(), vec![make_entry(vec![], None)]);
        block_directives.insert(
            "variant".to_string(),
            vec![make_entry(
                vec![make_value_string("stable"), make_value_number(50)],
                None,
            )],
        );

        let entry = make_entry(
            vec![make_value_string("ab_test")],
            Some(make_block(block_directives)),
        );
        let config = parse_canary_entry(&entry).unwrap();
        assert!(config.set_cookie);

        // Absent `set_cookie` defaults to false.
        let mut block_directives = StdHashMap::new();
        block_directives.insert(
            "variant".to_string(),
            vec![make_entry(
                vec![make_value_string("stable"), make_value_number(50)],
                None,
            )],
        );
        let entry = make_entry(
            vec![make_value_string("ab_test")],
            Some(make_block(block_directives)),
        );
        assert!(!parse_canary_entry(&entry).unwrap().set_cookie);
    }

    #[test]
    fn skips_malformed_entries() {
        // No arguments at all
        assert!(parse_canary_entry(&make_entry(vec![], None)).is_none());

        // No children block
        assert!(parse_canary_entry(&make_entry(vec![make_value_string("x")], None)).is_none());

        // Missing variant weight
        let mut block_directives = StdHashMap::new();
        block_directives.insert(
            "variant".to_string(),
            vec![make_entry(vec![make_value_string("a")], None)],
        );
        let entry = make_entry(
            vec![make_value_string("x")],
            Some(make_block(block_directives)),
        );
        let config = parse_canary_entry(&entry).unwrap();
        assert!(config.variants.is_empty());
    }

    #[test]
    fn skips_non_positive_weights() {
        let mut block_directives = StdHashMap::new();
        block_directives.insert(
            "variant".to_string(),
            vec![
                make_entry(vec![make_value_string("zero"), make_value_number(0)], None),
                make_entry(
                    vec![make_value_string("negative"), make_value_number(-3)],
                    None,
                ),
            ],
        );
        let entry = make_entry(
            vec![make_value_string("x")],
            Some(make_block(block_directives)),
        );
        assert!(parse_canary_entry(&entry).unwrap().variants.is_empty());
    }

    #[test]
    fn defaults_to_persistent_cookie() {
        let mut block_directives = StdHashMap::new();
        block_directives.insert(
            "variant".to_string(),
            vec![make_entry(
                vec![make_value_string("a"), make_value_number(1)],
                None,
            )],
        );
        let entry = make_entry(
            vec![make_value_string("x")],
            Some(make_block(block_directives)),
        );
        let config = parse_canary_entry(&entry).unwrap();
        // The default cookie is persistent (longer than the browser session).
        assert_eq!(
            config.cookie,
            CookieConfig {
                ttl: Some(Duration::from_secs(7 * 24 * 3600)),
                path: "/".to_string(),
                domain: None,
                secure: false,
                httponly: true,
                samesite: SameSiteMode::Lax,
            }
        );
    }

    #[test]
    fn parses_cookie_block() {
        let mut block_directives = StdHashMap::new();
        block_directives.insert(
            "variant".to_string(),
            vec![make_entry(
                vec![make_value_string("a"), make_value_number(1)],
                None,
            )],
        );
        let mut cookie_directives = StdHashMap::new();
        cookie_directives.insert(
            "ttl".to_string(),
            vec![make_entry(vec![make_value_string("1h")], None)],
        );
        cookie_directives.insert(
            "domain".to_string(),
            vec![make_entry(vec![make_value_string("example.com")], None)],
        );
        cookie_directives.insert("secure".to_string(), vec![make_entry(vec![], None)]);
        cookie_directives.insert("httponly".to_string(), vec![make_entry(vec![], None)]);
        cookie_directives.insert(
            "samesite".to_string(),
            vec![make_entry(vec![make_value_string("strict")], None)],
        );
        block_directives.insert(
            "cookie".to_string(),
            vec![make_entry(vec![], Some(make_block(cookie_directives)))],
        );

        let entry = make_entry(
            vec![make_value_string("x")],
            Some(make_block(block_directives)),
        );
        let config = parse_canary_entry(&entry).unwrap();
        assert_eq!(config.cookie.ttl, Some(Duration::from_secs(3600)));
        assert_eq!(config.cookie.domain, Some("example.com".to_string()));
        assert!(config.cookie.secure);
        assert!(config.cookie.httponly);
        assert_eq!(config.cookie.samesite, SameSiteMode::Strict);
    }
}
