use std::time::Duration;

use http::header::HeaderName;
use http::{HeaderMap, HeaderValue};

use crate::policy::CacheScope;

pub const LS_CACHE_CONTROL: HeaderName = HeaderName::from_static("x-litespeed-cache-control");
pub const LS_TAG: HeaderName = HeaderName::from_static("x-litespeed-tag");
pub const LS_PURGE: HeaderName = HeaderName::from_static("x-litespeed-purge");
pub const LS_VARY: HeaderName = HeaderName::from_static("x-litespeed-vary");
pub const LS_CACHE: HeaderName = HeaderName::from_static("x-litespeed-cache");
pub const LS_COOKIE: HeaderName = HeaderName::from_static("lsc-cookie");

#[derive(Clone, Debug, Default)]
pub struct LiteSpeedCacheControl {
    pub public: bool,
    pub private: bool,
    pub shared: bool,
    pub no_cache: bool,
    pub no_store: bool,
    pub no_vary: bool,
    pub max_age: Option<Duration>,
    pub s_maxage: Option<Duration>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LiteSpeedVary {
    pub cookies: Vec<String>,
    pub value: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopedTag {
    pub scope: CacheScope,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PurgeSelector {
    All,
    Tag(String),
    Url(String),
    UrlPath(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurgeOperation {
    pub scope: CacheScope,
    pub selectors: Vec<PurgeSelector>,
    pub stale: bool,
}

pub fn parse_litespeed_cache_control(headers: &HeaderMap) -> Option<LiteSpeedCacheControl> {
    let mut parsed = LiteSpeedCacheControl::default();
    let mut seen = false;

    for value in headers.get_all(&LS_CACHE_CONTROL) {
        let Some(text) = value.to_str().ok() else {
            continue;
        };
        for part in text.split(',') {
            let directive = part.trim();
            if directive.is_empty() {
                continue;
            }
            seen = true;
            match directive {
                "public" => parsed.public = true,
                "private" => parsed.private = true,
                "shared" => parsed.shared = true,
                "no-cache" => parsed.no_cache = true,
                "no-store" => parsed.no_store = true,
                "no-vary" => parsed.no_vary = true,
                _ => {
                    if let Some((name, value)) = directive.split_once('=') {
                        match name.trim() {
                            "max-age" => {
                                if let Ok(seconds) = value.trim().parse::<u64>() {
                                    parsed.max_age = Some(Duration::from_secs(seconds));
                                }
                            }
                            "s-maxage" => {
                                if let Ok(seconds) = value.trim().parse::<u64>() {
                                    parsed.s_maxage = Some(Duration::from_secs(seconds));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    seen.then_some(parsed)
}

pub fn parse_litespeed_vary(headers: &HeaderMap) -> LiteSpeedVary {
    let mut vary = LiteSpeedVary::default();

    for value in headers.get_all(&LS_VARY) {
        let Some(text) = value.to_str().ok() else {
            continue;
        };
        for part in text.split(',') {
            let directive = part.trim();
            if let Some(cookie) = directive.strip_prefix("cookie=") {
                let cookie = cookie.trim();
                if !cookie.is_empty() && !vary.cookies.iter().any(|name| name == cookie) {
                    vary.cookies.push(cookie.to_string());
                }
            } else if let Some(value) = directive.strip_prefix("value=") {
                let value = value.trim();
                if !value.is_empty() {
                    vary.value = Some(value.to_string());
                }
            }
        }
    }

    vary.cookies.sort_unstable();
    vary
}

pub fn parse_litespeed_tags(headers: &HeaderMap, response_scope: CacheScope) -> Vec<ScopedTag> {
    let mut tags = Vec::new();

    for value in headers.get_all(&LS_TAG) {
        let Some(text) = value.to_str().ok() else {
            continue;
        };
        for part in text.split(',') {
            let token = part.trim();
            if token.is_empty() {
                continue;
            }

            let (scope, name) = if let Some(name) = token.strip_prefix("public:") {
                (CacheScope::Public, name.trim())
            } else {
                (response_scope, token)
            };

            if !name.is_empty()
                && !tags
                    .iter()
                    .any(|existing: &ScopedTag| existing.scope == scope && existing.name == name)
            {
                tags.push(ScopedTag {
                    scope,
                    name: name.to_string(),
                });
            }
        }
    }

    tags
}

pub fn parse_litespeed_purge(headers: &HeaderMap) -> Vec<PurgeOperation> {
    let mut operations = Vec::new();

    for value in headers.get_all(&LS_PURGE) {
        let Some(text) = value.to_str().ok() else {
            continue;
        };
        for segment in text.split(';') {
            let mut scope = CacheScope::Public;
            let mut selectors = Vec::new();
            let mut stale = false;

            for token in segment.split(',') {
                let token = token.trim();
                if token.is_empty() {
                    continue;
                }

                match token {
                    "public" => scope = CacheScope::Public,
                    "private" => scope = CacheScope::Private,
                    "stale" => stale = true,
                    "*" => selectors.push(PurgeSelector::All),
                    _ => {
                        if let Some(tag) = token.strip_prefix("tag=") {
                            selectors.push(PurgeSelector::Tag(tag.trim().to_string()));
                        } else if let Some(url) = token.strip_prefix("url=") {
                            selectors.push(PurgeSelector::Url(url.trim().to_string()));
                        } else {
                            selectors.push(PurgeSelector::Tag(token.to_string()));
                        }
                    }
                }
            }

            if !selectors.is_empty() {
                operations.push(PurgeOperation {
                    scope,
                    selectors,
                    stale,
                });
            }
        }
    }

    operations
}

pub fn collect_lsc_cookies(headers: &HeaderMap) -> Vec<HeaderValue> {
    headers.get_all(&LS_COOKIE).iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vary() {
        let mut headers = HeaderMap::new();
        headers.insert(
            &LS_VARY,
            HeaderValue::from_static("cookie=currency,value=mobile"),
        );
        let vary = parse_litespeed_vary(&headers);
        assert_eq!(vary.cookies, vec!["currency".to_string()]);
        assert_eq!(vary.value.as_deref(), Some("mobile"));
    }

    #[test]
    fn parses_purge() {
        let mut headers = HeaderMap::new();
        headers.insert(
            &LS_PURGE,
            HeaderValue::from_static("public,tag=listing;private,url=/account"),
        );
        let purge = parse_litespeed_purge(&headers);
        assert_eq!(purge.len(), 2);
        assert_eq!(purge[0].scope, CacheScope::Public);
        assert_eq!(purge[1].scope, CacheScope::Private);
    }

    #[test]
    fn empty_header_values_produce_defaults() {
        let headers = HeaderMap::new();
        assert!(parse_litespeed_cache_control(&headers).is_none());
        assert_eq!(parse_litespeed_vary(&headers).cookies.len(), 0);
        assert_eq!(parse_litespeed_tags(&headers, CacheScope::Public).len(), 0);
        assert_eq!(parse_litespeed_purge(&headers).len(), 0);
    }

    #[test]
    fn purge_empty_tokens_between_delimiters() {
        let mut headers = HeaderMap::new();
        headers.insert(&LS_PURGE, HeaderValue::from_static("url=/path,,tag=foo"));
        let purge = parse_litespeed_purge(&headers);
        assert_eq!(purge.len(), 1);
        assert_eq!(purge[0].selectors.len(), 2);
    }

    #[test]
    fn purge_double_semicolon() {
        let mut headers = HeaderMap::new();
        headers.insert(&LS_PURGE, HeaderValue::from_static("tag=a;;tag=b"));
        let purge = parse_litespeed_purge(&headers);
        // Empty segment between ;; produces no operation
        assert_eq!(purge.len(), 2);
    }

    #[test]
    fn purge_empty_tag_value() {
        let mut headers = HeaderMap::new();
        headers.insert(&LS_PURGE, HeaderValue::from_static("tag="));
        let purge = parse_litespeed_purge(&headers);
        assert_eq!(purge.len(), 1);
        assert_eq!(purge[0].selectors.len(), 1);
        match &purge[0].selectors[0] {
            PurgeSelector::Tag(t) => assert_eq!(t, ""),
            _ => panic!("expected Tag selector"),
        }
    }

    #[test]
    fn purge_mixed_scope_on_same_segment() {
        let mut headers = HeaderMap::new();
        headers.insert(
            &LS_PURGE,
            HeaderValue::from_static("private,public,tag=foo"),
        );
        let purge = parse_litespeed_purge(&headers);
        // Last scope wins for the segment: public
        assert_eq!(purge.len(), 1);
        assert_eq!(purge[0].scope, CacheScope::Public);
    }

    #[test]
    fn purge_bare_token_falls_back_to_tag() {
        let mut headers = HeaderMap::new();
        headers.insert(&LS_PURGE, HeaderValue::from_static("some-unknown-token"));
        let purge = parse_litespeed_purge(&headers);
        assert_eq!(purge.len(), 1);
        assert_eq!(purge[0].selectors.len(), 1);
        match &purge[0].selectors[0] {
            PurgeSelector::Tag(t) => assert_eq!(t, "some-unknown-token"),
            _ => panic!("expected Tag selector for bare token"),
        }
    }

    #[test]
    fn tags_dedup_by_scope_and_name() {
        let mut headers = HeaderMap::new();
        headers.insert(&LS_TAG, HeaderValue::from_static("tag-a, tag-a, tag-b"));
        let tags = parse_litespeed_tags(&headers, CacheScope::Public);
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].name, "tag-a");
        assert_eq!(tags[1].name, "tag-b");
    }

    #[test]
    fn tags_public_prefix_overrides_scope() {
        let mut headers = HeaderMap::new();
        headers.insert(
            &LS_TAG,
            HeaderValue::from_static("public:shared-tag, private-tag"),
        );
        let tags = parse_litespeed_tags(&headers, CacheScope::Private);
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].scope, CacheScope::Public);
        assert_eq!(tags[0].name, "shared-tag");
        assert_eq!(tags[1].scope, CacheScope::Private);
        assert_eq!(tags[1].name, "private-tag");
    }

    #[test]
    fn tags_empty_token_skipped() {
        let mut headers = HeaderMap::new();
        headers.insert(&LS_TAG, HeaderValue::from_static("tag-a,,tag-b"));
        let tags = parse_litespeed_tags(&headers, CacheScope::Public);
        assert_eq!(tags.len(), 2);
    }

    #[test]
    fn vary_cookies_dedup_and_sorted() {
        let mut headers = HeaderMap::new();
        headers.insert(
            &LS_VARY,
            HeaderValue::from_static("cookie=z, cookie=a, cookie=a"),
        );
        let vary = parse_litespeed_vary(&headers);
        assert_eq!(vary.cookies, vec!["a", "z"]);
    }

    #[test]
    fn vary_empty_cookie_value_skipped() {
        let mut headers = HeaderMap::new();
        headers.insert(&LS_VARY, HeaderValue::from_static("cookie="));
        let vary = parse_litespeed_vary(&headers);
        assert!(vary.cookies.is_empty());
    }

    #[test]
    fn vary_value_no_trim_issue() {
        let mut headers = HeaderMap::new();
        headers.insert(&LS_VARY, HeaderValue::from_static("value= desktop"));
        let vary = parse_litespeed_vary(&headers);
        // value= is stripped and remaining content is trimmed then stored
        assert_eq!(vary.value.as_deref(), Some("desktop"));
    }

    #[test]
    fn lscache_control_parses_multiple_headers() {
        let mut headers = HeaderMap::new();
        headers.append(
            &LS_CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=120"),
        );
        headers.append(&LS_CACHE_CONTROL, HeaderValue::from_static("no-vary"));
        let cc = parse_litespeed_cache_control(&headers).expect("should parse");
        assert!(cc.public);
        assert!(cc.no_vary);
        assert_eq!(cc.max_age, Some(Duration::from_secs(120)));
    }

    #[test]
    fn purge_with_stale_flag() {
        let mut headers = HeaderMap::new();
        headers.insert(
            &LS_PURGE,
            HeaderValue::from_static("public,stale,tag=listing"),
        );
        let purge = parse_litespeed_purge(&headers);
        assert_eq!(purge.len(), 1);
        assert!(purge[0].stale);
        assert_eq!(purge[0].scope, CacheScope::Public);
    }

    #[test]
    fn purge_all_selector_with_tag() {
        let mut headers = HeaderMap::new();
        headers.insert(&LS_PURGE, HeaderValue::from_static("*,tag=foo"));
        let purge = parse_litespeed_purge(&headers);
        assert_eq!(purge.len(), 1);
        assert_eq!(purge[0].selectors.len(), 2);
    }
}
