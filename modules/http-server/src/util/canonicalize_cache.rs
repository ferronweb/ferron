/// Small bounded cache for routing-only canonicalization results.
/// Key: raw request path string. Value: (routing_view, original)
/// Bounded by approximate byte weight to avoid unbounded memory growth.

#[derive(Clone)]
struct CanonicalizeCacheWeighter;

impl quick_cache::Weighter<String, (String, String)> for CanonicalizeCacheWeighter {
    fn weight(&self, key: &String, val: &(String, String)) -> u64 {
        (key.len() + val.0.len() + val.1.len()) as u64
    }
}

static CANONICALIZE_CACHE: std::sync::LazyLock<
    quick_cache::sync::Cache<String, (String, String), CanonicalizeCacheWeighter>,
> = std::sync::LazyLock::new(|| {
    // Initial capacity 2048 entries, max weight ~4 MiB
    quick_cache::sync::Cache::with_weighter(2048, 4 * 1024 * 1024, CanonicalizeCacheWeighter)
});

/// Routing-only canonicalization with a bounded global cache.
/// Returns (routing_view, original) on success.
pub fn canonicalize_path_routing_cached(
    raw_path: &str,
) -> Result<(String, String), crate::util::canonicalize_url::CanonicalizationError> {
    // Allocate a temporary key for lookup/insert. This cost is typically
    // smaller than full canonicalization work and is bounded by the length
    // of the request path.
    let key = raw_path.to_string();

    if let Some(v) = CANONICALIZE_CACHE.get(&key) {
        return Ok(v);
    }

    let res = crate::util::canonicalize_url::canonicalize_path_routing(raw_path)?;
    CANONICALIZE_CACHE.insert(key, res.clone());
    Ok(res)
}
