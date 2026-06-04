use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::Arc;

use crate::server::sni::HostnameRadixTree;

/// A simpler, hostname-centric radix tree used for TLS resolver lookups.
///
/// This struct implements the public API previously provided by the compressed
/// RadixTree but internally delegates hostname matching to
/// `HostnameRadixTree` and keeps IP-only and IP+hostname maps to ensure exact
/// semantics: an ip+hostname entry only matches that exact hostname (or its
/// wildcard), and will not be returned for subdomains unless a wildcard was
/// installed.
#[derive(Debug)]
pub struct RadixTree<T> {
    root_data: Option<T>,
    ip_map: BTreeMap<IpAddr, T>,
    host_tree: HostnameRadixTree<T>,
    ip_host_trees: BTreeMap<IpAddr, HostnameRadixTree<T>>,
}

impl<T: Clone> RadixTree<T> {
    /// Create a new empty tree
    #[inline]
    pub fn new() -> Self {
        Self {
            root_data: None,
            ip_map: BTreeMap::new(),
            host_tree: HostnameRadixTree::new(),
            ip_host_trees: BTreeMap::new(),
        }
    }

    /// Sets the root (default) data.
    #[inline]
    pub fn set_root_data(&mut self, data: T) {
        self.root_data = Some(data);
    }

    /// Gets the root (default) data, if set.
    #[inline]
    pub fn root_data(&self) -> Option<T> {
        self.root_data.clone()
    }

    /// Inserts data for an IP address.
    #[inline]
    pub fn insert_ip(&mut self, ip: IpAddr, data: T) {
        self.ip_map.insert(ip, data);
    }

    /// Inserts data for a hostname (with optional wildcard).
    #[inline]
    pub fn insert_hostname(&mut self, hostname: &str, data: T) {
        self.host_tree.insert(hostname.to_string(), data);
    }

    /// Inserts data for both an IP address and hostname.
    #[inline]
    pub fn insert_ip_and_hostname(&mut self, ip: IpAddr, hostname: &str, data: T) {
        let tree = self
            .ip_host_trees
            .entry(ip)
            .or_insert_with(HostnameRadixTree::new);
        tree.insert(hostname.to_string(), data);
    }

    /// Looks up data by IP address.
    #[inline]
    pub fn lookup_ip(&self, ip: IpAddr) -> Option<T> {
        self.ip_map
            .get(&ip)
            .cloned()
            .or_else(|| self.root_data.clone())
    }

    /// Looks up data by hostname.
    #[inline]
    pub fn lookup_hostname(&self, hostname: &str) -> Option<T> {
        self.host_tree
            .get(hostname)
            .cloned()
            .or_else(|| self.root_data.clone())
    }

    /// Looks up data by both IP address and hostname.
    ///
    /// Order of precedence:
    /// 1. ip-specific hostname exact/wildcard
    /// 2. ip-only entry
    /// 3. hostname-only entry (including wildcard)
    /// 4. root data fallback
    #[inline]
    pub fn lookup_ip_and_hostname(&self, ip: IpAddr, hostname: &str) -> Option<T> {
        // 1) ip-specific hostname tree
        if let Some(tree) = self.ip_host_trees.get(&ip) {
            if let Some(v) = tree.get(hostname) {
                return Some(v.clone());
            }
        }

        // 2) ip-only
        if let Some(v) = self.ip_map.get(&ip) {
            return Some(v.clone());
        }

        // 3) hostname-only (includes root_data fallback)
        self.lookup_hostname(hostname)
    }
}

impl<T: Clone> Default for RadixTree<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

// Type alias for the common TLS resolver use case
pub type TlsResolverRadixTree = RadixTree<Arc<dyn ferron_tls::TcpTlsResolver>>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_insert_and_lookup_ip() {
        let mut tree = RadixTree::new();
        tree.insert_ip(
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            "127-resolver".to_string(),
        );

        let found = tree.lookup_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "127-resolver");

        let not_found = tree.lookup_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));
        assert!(not_found.is_none());
    }

    #[test]
    fn test_insert_and_lookup_hostname() {
        let mut tree = RadixTree::new();
        tree.insert_hostname("example.com", "example-com".to_string());

        let found = tree.lookup_hostname("example.com");
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "example-com");

        let not_found = tree.lookup_hostname("test.com");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_wildcard_lookup() {
        let mut tree = RadixTree::new();
        tree.insert_hostname("*.example.com", "wildcard-example-com".to_string());

        let found = tree.lookup_hostname("sub.example.com");
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "wildcard-example-com");
    }

    #[test]
    fn test_hierarchy_priority() {
        let mut tree = RadixTree::new();

        tree.insert_hostname("com", "com-resolver".to_string());
        tree.insert_hostname("example.com", "example-com-resolver".to_string());

        let found = tree.lookup_hostname("example.com");
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "example-com-resolver");
    }

    #[test]
    fn test_mixed_ip_and_hostname() {
        let mut tree = RadixTree::new();

        tree.insert_ip(
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            "ip-127".to_string(),
        );
        tree.insert_hostname("localhost", "localhost".to_string());

        assert!(tree
            .lookup_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)))
            .is_some());
        assert!(tree.lookup_hostname("localhost").is_some());
    }

    #[test]
    fn test_btree_ordering() {
        let mut tree = RadixTree::new();

        tree.insert_hostname("z.com", "z-resolver".to_string());
        tree.insert_hostname("a.com", "a-resolver".to_string());
        tree.insert_hostname("m.com", "m-resolver".to_string());

        assert!(tree.lookup_hostname("z.com").is_some());
        assert!(tree.lookup_hostname("a.com").is_some());
        assert!(tree.lookup_hostname("m.com").is_some());
    }

    #[test]
    fn test_insert_ip_and_hostname() {
        let mut tree = RadixTree::new();
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        tree.insert_ip_and_hostname(ip, "localhost", "combined".to_string());

        let found = tree.lookup_ip_and_hostname(ip, "localhost");
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "combined");

        let ip_only = tree.lookup_ip(ip);
        assert!(ip_only.is_none());

        let hostname_only = tree.lookup_hostname("localhost");
        assert!(hostname_only.is_none());
    }

    #[test]
    fn test_ip_and_hostname_with_wildcard() {
        let mut tree = RadixTree::new();
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        tree.insert_ip_and_hostname(ip, "*.example.com", "wildcard".to_string());

        let found = tree.lookup_ip_and_hostname(ip, "sub.example.com");
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "wildcard");
    }

    #[test]
    fn test_ip_and_hostname_without_wildcard() {
        let mut tree = RadixTree::new();
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        tree.insert_ip_and_hostname(ip, "example.com", "exact".to_string());

        let found = tree.lookup_ip_and_hostname(ip, "example.com");
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "exact");

        let not_found = tree.lookup_ip_and_hostname(ip, "sub.example.com");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_lookup_fallback_order() {
        let mut tree = RadixTree::new();

        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
        tree.insert_ip(ip, "ip-only".to_string());
        tree.insert_hostname("example.com", "hostname-only".to_string());

        let found = tree.lookup_ip_and_hostname(ip, "example.com");
        assert!(found.is_some());
    }

    #[test]
    fn test_combined_more_specific_than_separate() {
        let mut tree = RadixTree::new();

        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let ip_prefix = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 0));
        tree.insert_ip(ip_prefix, "ip-resolver".to_string());
        tree.insert_hostname("example.com", "hostname-resolver".to_string());
        tree.insert_ip_and_hostname(ip, "example.com", "combined-resolver".to_string());

        let found = tree.lookup_ip_and_hostname(ip, "example.com");
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "combined-resolver");
    }

    #[test]
    fn test_ipv6_support() {
        let mut tree = RadixTree::new();
        let ip = IpAddr::V6("::1".parse().unwrap());
        tree.insert_ip(ip, "ipv6-localhost".to_string());

        let found = tree.lookup_ip(ip);
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "ipv6-localhost");
    }

    #[test]
    fn test_root_data() {
        let mut tree = RadixTree::new();

        assert!(tree.root_data().is_none());

        tree.set_root_data("root-resolver".to_string());
        assert!(tree.root_data().is_some());
        assert_eq!(tree.root_data().unwrap(), "root-resolver");

        let ip = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        let found = tree.lookup_ip(ip);
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "root-resolver");

        let found = tree.lookup_hostname("nonexistent.com");
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "root-resolver");
    }

    #[test]
    fn test_root_data_with_specific_matches() {
        let mut tree = RadixTree::new();
        tree.set_root_data("root-resolver".to_string());

        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        tree.insert_ip(ip, "specific-resolver".to_string());

        let found = tree.lookup_ip(ip);
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "specific-resolver");

        let other_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
        let found = tree.lookup_ip(other_ip);
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "root-resolver");
    }

    #[test]
    fn test_lookup_ip_and_hostname_with_root_fallback() {
        let mut tree = RadixTree::new();
        tree.set_root_data("root-resolver".to_string());

        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let found = tree.lookup_ip_and_hostname(ip, "example.com");
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "root-resolver");
    }
}
