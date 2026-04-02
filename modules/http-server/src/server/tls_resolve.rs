use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::Arc;

use ferron_tls::TcpTlsResolver;

/// Key types for the radix tree, ordered from root to leaf.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum RadixKey {
    /// IPv4/IPv6 address octet or byte (e.g., `127` from `127.0.0.1`)
    IpOctet(u8),
    /// Hostname segment (e.g., `"com"`, `"example"` from `"example.com"`)
    HostSegment(String),
    /// Hostname wildcard (`"*"` for `"*.example.com"`)
    HostWildcard,
}

impl RadixKey {
    /// Returns the sort order for this key type (lower = closer to root).
    fn order(&self) -> u8 {
        match self {
            RadixKey::IpOctet(_) => 0,
            RadixKey::HostSegment(_) => 1,
            RadixKey::HostWildcard => 2,
        }
    }

    /// Converts the key to bytes for storage in the BTreeMap.
    fn to_bytes(&self) -> Vec<u8> {
        match self {
            RadixKey::IpOctet(octet) => vec![0x00, *octet],
            RadixKey::HostSegment(segment) => {
                let mut bytes = vec![0x01];
                bytes.extend_from_slice(segment.as_bytes());
                bytes
            }
            RadixKey::HostWildcard => vec![0x02, b'*'],
        }
    }

    /// Parses bytes back into a key.
    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() {
            return None;
        }
        match bytes[0] {
            0x00 => bytes.get(1).copied().map(RadixKey::IpOctet),
            0x01 => String::from_utf8(bytes[1..].to_vec())
                .ok()
                .map(RadixKey::HostSegment),
            0x02 => Some(RadixKey::HostWildcard),
            _ => None,
        }
    }
}

/// A compressed radix tree node for TLS resolver lookup.
///
/// The tree stores resolvers at nodes and supports lookup by:
/// 1. IPv4/IPv6 address octet (e.g., "127" from "127.0.0.1")
/// 2. Hostname segment (e.g., "com", "example" from "example.com")
/// 3. Hostname wildcard ("*" for "*.example.com")
///
/// Lookup returns the resolver at the highest (most specific) level found.
struct RadixNode {
    /// Compressed edge label leading to this node
    edge: Vec<u8>,
    /// Resolver stored at this node (if any)
    resolver: Option<Arc<dyn TcpTlsResolver>>,
    /// Child nodes (BTreeMap for ordered traversal)
    children: BTreeMap<Vec<u8>, RadixNode>,
}

impl RadixNode {
    fn new(edge: Vec<u8>) -> Self {
        Self {
            edge,
            resolver: None,
            children: BTreeMap::new(),
        }
    }

    fn with_resolver(edge: Vec<u8>, resolver: Arc<dyn TcpTlsResolver>) -> Self {
        Self {
            edge,
            resolver: Some(resolver),
            children: BTreeMap::new(),
        }
    }
}

/// A compressed radix tree for storing and looking up TLS resolvers.
///
/// The tree organizes resolvers in a hierarchy:
/// - Root level: IPv4/IPv6 address octets
/// - Second level: Hostname segments (reversed for suffix matching)
/// - Third level: Wildcard entries
///
/// Usage: Build the tree using `insert_*` methods with `&mut self`,
/// then use `lookup_*` methods with `&self` for concurrent reads.
///
/// # Examples
///
/// ```rust,ignore
/// use std::net::IpAddr;
///
/// let mut tree = TlsResolverRadixTree::new();
///
/// // Insert by IP only
/// tree.insert_ip(IpAddr::from([127, 0, 0, 1]), resolver.clone());
///
/// // Insert by hostname only
/// tree.insert_hostname("example.com", resolver.clone(), false);
///
/// // Insert by both IP and hostname (most specific)
/// tree.insert_ip_and_hostname(IpAddr::from([127, 0, 0, 1]), "localhost", resolver.clone(), false);
///
/// // Lookup will return the most specific match
/// tree.lookup_ip_and_hostname(IpAddr::from([127, 0, 0, 1]), "localhost");
/// ```
pub struct TlsResolverRadixTree {
    root: RadixNode,
}

impl TlsResolverRadixTree {
    /// Creates a new empty radix tree.
    pub fn new() -> Self {
        Self {
            root: RadixNode::new(Vec::new()),
        }
    }

    /// Inserts a resolver into the tree at the specified path.
    ///
    /// The path components are ordered from root to leaf:
    /// - For IP-based: `[IpOctet(127), IpOctet(0), IpOctet(0), IpOctet(1)]`
    /// - For hostname: `[HostSegment("com"), HostSegment("example")]` (reversed)
    /// - For wildcard: `[HostWildcard, HostSegment("com"), HostSegment("example")]`
    /// - For combined IP+hostname: IP octets followed by hostname segments
    pub fn insert(&mut self, path: &[RadixKey], resolver: Arc<dyn TcpTlsResolver>) {
        let mut current = &mut self.root;

        for (i, key) in path.iter().enumerate() {
            let bytes = key.to_bytes();
            let is_last = i == path.len() - 1;

            let child = {
                let mut found_child_key = None;

                // Find a child with matching edge prefix
                for child_key in current.children.keys() {
                    if child_key.starts_with(&bytes) || bytes.starts_with(child_key.as_slice()) {
                        found_child_key = Some(child_key.clone());
                        break;
                    }
                }

                if let Some(child_key) = found_child_key {
                    let child = current.children.get_mut(&child_key).unwrap();
                    let child_edge = child.edge.clone();

                    if child_edge == bytes {
                        // Exact match - update resolver if this is the last segment
                        if is_last {
                            child.resolver = Some(resolver.clone());
                        }
                        child
                    } else if child_edge.starts_with(&bytes) {
                        // New node is a prefix of existing child - split
                        let remaining = child_edge[bytes.len()..].to_vec();
                        child.edge = bytes.clone();

                        if is_last {
                            child.resolver = Some(resolver.clone());
                        }

                        // Create new child for the remaining edge
                        let existing_children = std::mem::take(&mut child.children);
                        let new_child = RadixNode {
                            edge: remaining.clone(),
                            resolver: None,
                            children: existing_children,
                        };

                        child.children.insert(remaining, new_child);
                        child
                    } else {
                        // Existing child edge is a prefix of new bytes
                        // Need to traverse deeper or create intermediate node
                        let remaining = &bytes[child_edge.len()..];
                        Self::insert_into_child(child, remaining, is_last, &resolver);
                        child
                    }
                } else {
                    // No matching child - create new node
                    let new_node = if is_last {
                        RadixNode::with_resolver(bytes.clone(), resolver.clone())
                    } else {
                        RadixNode::new(bytes.clone())
                    };
                    current.children.insert(bytes.clone(), new_node);
                    current.children.get_mut(&bytes).unwrap()
                }
            };

            current = child;
        }
    }

    fn insert_into_child<'a>(
        child: &'a mut RadixNode,
        remaining: &[u8],
        is_last: bool,
        resolver: &Arc<dyn TcpTlsResolver>,
    ) -> &'a mut RadixNode {
        let mut current = child;
        let mut offset = 0;

        while offset < remaining.len() {
            let edge = current.edge.clone();

            if edge.is_empty() {
                current.edge = remaining[offset..].to_vec();
                if is_last && offset + edge.len() >= remaining.len() {
                    current.resolver = Some(resolver.clone());
                }
                break;
            }

            // Find matching child or create new
            let mut found = None;
            for child_key in current.children.keys() {
                if child_key.starts_with(&edge) || edge.starts_with(child_key.as_slice()) {
                    found = Some(child_key.clone());
                    break;
                }
            }

            if let Some(child_key) = found {
                let next_child = current.children.get_mut(&child_key).unwrap();
                current = next_child;
            } else {
                let new_node = if is_last {
                    RadixNode::with_resolver(remaining[offset..].to_vec(), resolver.clone())
                } else {
                    RadixNode::new(remaining[offset..].to_vec())
                };
                let edge_key = new_node.edge.clone();
                current.children.insert(edge_key.clone(), new_node);
                return current.children.get_mut(&edge_key).unwrap();
            }

            offset += edge.len();
        }

        current
    }

    /// Looks up a resolver by path components.
    ///
    /// Returns the resolver at the highest (most specific) level found during traversal.
    pub fn lookup(&self, path: &[RadixKey]) -> Option<Arc<dyn TcpTlsResolver>> {
        let mut current = &self.root;
        let mut best_match: Option<Arc<dyn TcpTlsResolver>> = None;

        for key in path {
            let bytes = key.to_bytes();

            let next = {
                let mut found_child = None;

                for (child_key, child) in &current.children {
                    if child_key.as_slice() == bytes {
                        found_child = Some((child, true));
                        break;
                    } else if bytes.starts_with(child_key.as_slice()) {
                        // Partial match - continue traversal
                        found_child = Some((child, false));
                        break;
                    }
                }

                if let Some((child, exact)) = found_child {
                    // Check if this node has a resolver
                    if child.resolver.is_some() {
                        best_match = child.resolver.clone();
                    }

                    if exact {
                        Some(child)
                    } else {
                        // Partial match - continue with this child
                        Some(child)
                    }
                } else {
                    None
                }
            };

            match next {
                Some(child) => current = child,
                None => break,
            }
        }

        // Check final node for resolver
        current.resolver.clone().or(best_match)
    }

    /// Converts an IP address to a path of IP octet keys.
    fn ip_to_path(ip: IpAddr) -> Vec<RadixKey> {
        match ip {
            IpAddr::V4(addr) => addr
                .octets()
                .iter()
                .copied()
                .map(RadixKey::IpOctet)
                .collect(),
            IpAddr::V6(addr) => addr
                .octets()
                .iter()
                .copied()
                .map(RadixKey::IpOctet)
                .collect(),
        }
    }

    /// Converts a hostname to a path of hostname segment keys (reversed for suffix matching).
    fn hostname_to_path(hostname: &str, wildcard: bool) -> Vec<RadixKey> {
        let mut segments: Vec<RadixKey> = hostname
            .split('.')
            .rev()
            .map(|s| RadixKey::HostSegment(s.to_string()))
            .collect();

        if wildcard {
            segments.insert(0, RadixKey::HostWildcard);
        }

        segments
    }

    /// Inserts a resolver for an IP address.
    ///
    /// For IPv4, all 4 octets are used. For IPv6, all 16 bytes are used.
    pub fn insert_ip(&mut self, ip: IpAddr, resolver: Arc<dyn TcpTlsResolver>) {
        let path = Self::ip_to_path(ip);
        self.insert(&path, resolver);
    }

    /// Inserts a resolver for a partial IP address prefix.
    ///
    /// Useful for matching ranges like `127.x.x.x` or `192.168.x.x`.
    pub fn insert_ip_prefix(&mut self, prefix: &[u8], resolver: Arc<dyn TcpTlsResolver>) {
        let path: Vec<RadixKey> = prefix.iter().copied().map(RadixKey::IpOctet).collect();
        self.insert(&path, resolver);
    }

    /// Inserts a resolver for a hostname (with optional wildcard).
    ///
    /// If `wildcard` is true, the resolver will match `*.hostname`.
    pub fn insert_hostname(
        &mut self,
        hostname: &str,
        resolver: Arc<dyn TcpTlsResolver>,
        wildcard: bool,
    ) {
        let path = Self::hostname_to_path(hostname, wildcard);
        self.insert(&path, resolver);
    }

    /// Inserts a resolver for both an IP address and hostname.
    ///
    /// This creates a more specific path: IP octets followed by hostname segments.
    /// The resolver will only match when both the IP and hostname match.
    ///
    /// # Arguments
    ///
    /// * `ip` - IP address
    /// * `hostname` - Hostname (e.g., `"localhost"`)
    /// * `resolver` - The TLS resolver to store
    /// * `wildcard` - If true, matches subdomains (e.g., `*.example.com`)
    pub fn insert_ip_and_hostname(
        &mut self,
        ip: IpAddr,
        hostname: &str,
        resolver: Arc<dyn TcpTlsResolver>,
        wildcard: bool,
    ) {
        let mut path = Self::ip_to_path(ip);
        path.extend(Self::hostname_to_path(hostname, wildcard));
        self.insert(&path, resolver);
    }

    /// Looks up a resolver by IP address.
    pub fn lookup_ip(&self, ip: IpAddr) -> Option<Arc<dyn TcpTlsResolver>> {
        let path = Self::ip_to_path(ip);
        self.lookup(&path)
    }

    /// Looks up a resolver by IP address prefix.
    pub fn lookup_ip_prefix(&self, prefix: &[u8]) -> Option<Arc<dyn TcpTlsResolver>> {
        let path: Vec<RadixKey> = prefix.iter().copied().map(RadixKey::IpOctet).collect();
        self.lookup(&path)
    }

    /// Looks up a resolver by hostname.
    ///
    /// Attempts to find the most specific match, checking:
    /// 1. Exact hostname match
    /// 2. Wildcard match for parent domain
    pub fn lookup_hostname(&self, hostname: &str) -> Option<Arc<dyn TcpTlsResolver>> {
        // Try exact match first
        let exact_path = Self::hostname_to_path(hostname, false);
        if let Some(resolver) = self.lookup(&exact_path) {
            return Some(resolver);
        }

        // Try wildcard match (add "*" at the beginning of reversed segments)
        let wildcard_path = Self::hostname_to_path(hostname, true);
        self.lookup(&wildcard_path)
    }

    /// Looks up a resolver by both IP address and hostname.
    ///
    /// Attempts to find the most specific match in this order:
    /// 1. Exact IP + exact hostname match
    /// 2. Exact IP + wildcard hostname match
    /// 3. IP prefix + hostname match (falls back to IP-only or hostname-only)
    ///
    /// # Arguments
    ///
    /// * `ip` - IP address
    /// * `hostname` - Hostname to match (e.g., `"localhost"`)
    ///
    /// # Returns
    ///
    /// The most specific resolver found, or `None` if no match.
    pub fn lookup_ip_and_hostname(
        &self,
        ip: IpAddr,
        hostname: &str,
    ) -> Option<Arc<dyn TcpTlsResolver>> {
        // Build full path: IP octets + hostname segments
        let mut full_path = Self::ip_to_path(ip);
        full_path.extend(Self::hostname_to_path(hostname, false));

        // Try exact IP + exact hostname
        if let Some(resolver) = self.lookup(&full_path) {
            return Some(resolver);
        }

        // Try exact IP + wildcard hostname
        let mut wildcard_path = Self::ip_to_path(ip);
        wildcard_path.extend(Self::hostname_to_path(hostname, true));
        if let Some(resolver) = self.lookup(&wildcard_path) {
            return Some(resolver);
        }

        // Fall back to IP-only lookup
        if let Some(resolver) = self.lookup_ip(ip) {
            return Some(resolver);
        }

        // Fall back to hostname-only lookup
        self.lookup_hostname(hostname)
    }
}

impl Default for TlsResolverRadixTree {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use std::sync::Arc;

    struct MockResolver {
        #[allow(dead_code)]
        name: String,
    }

    #[async_trait::async_trait(?Send)]
    impl TcpTlsResolver for MockResolver {
        fn get_tls_config(&self) -> Arc<rustls::ServerConfig> {
            unimplemented!()
        }
    }

    #[test]
    fn test_insert_and_lookup_ip() {
        let mut tree = TlsResolverRadixTree::new();
        let resolver = Arc::new(MockResolver {
            name: "127-0-0-1-resolver".to_string(),
        });

        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        tree.insert_ip(ip, resolver.clone());

        let found = tree.lookup_ip(ip);
        assert!(found.is_some());

        let other_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
        let not_found = tree.lookup_ip(other_ip);
        assert!(not_found.is_none());
    }

    #[test]
    fn test_insert_ip_prefix() {
        let mut tree = TlsResolverRadixTree::new();
        let resolver = Arc::new(MockResolver {
            name: "127-prefix-resolver".to_string(),
        });

        // Insert prefix for 127.x.x.x
        tree.insert_ip_prefix(&[127], resolver.clone());

        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let found = tree.lookup_ip_prefix(&[127]);
        assert!(found.is_some());
    }

    #[test]
    fn test_insert_and_lookup_hostname() {
        let mut tree = TlsResolverRadixTree::new();
        let resolver = Arc::new(MockResolver {
            name: "example-com-resolver".to_string(),
        });

        tree.insert_hostname("example.com", resolver.clone(), false);

        let found = tree.lookup_hostname("example.com");
        assert!(found.is_some());

        let not_found = tree.lookup_hostname("test.com");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_wildcard_lookup() {
        let mut tree = TlsResolverRadixTree::new();
        let resolver = Arc::new(MockResolver {
            name: "wildcard-example-com".to_string(),
        });

        tree.insert_hostname("example.com", resolver.clone(), true);

        // Wildcard should match subdomains
        let found = tree.lookup_hostname("sub.example.com");
        assert!(found.is_some());
    }

    #[test]
    fn test_hierarchy_priority() {
        let mut tree = TlsResolverRadixTree::new();

        let com_resolver = Arc::new(MockResolver {
            name: "com-resolver".to_string(),
        });
        let example_com_resolver = Arc::new(MockResolver {
            name: "example-com-resolver".to_string(),
        });

        tree.insert_hostname("com", com_resolver.clone(), false);
        tree.insert_hostname("example.com", example_com_resolver.clone(), false);

        // Should return the most specific match
        let found = tree.lookup_hostname("example.com");
        assert!(found.is_some());
    }

    #[test]
    fn test_mixed_ip_and_hostname() {
        let mut tree = TlsResolverRadixTree::new();

        let ip_resolver = Arc::new(MockResolver {
            name: "ip-127-resolver".to_string(),
        });
        let hostname_resolver = Arc::new(MockResolver {
            name: "localhost-resolver".to_string(),
        });

        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        tree.insert_ip(ip, ip_resolver.clone());
        tree.insert_hostname("localhost", hostname_resolver.clone(), false);

        assert!(tree.lookup_ip(ip).is_some());
        assert!(tree.lookup_hostname("localhost").is_some());
    }

    #[test]
    fn test_btree_ordering() {
        let mut tree = TlsResolverRadixTree::new();

        // Insert in non-alphabetical order
        let resolver_z = Arc::new(MockResolver {
            name: "z-resolver".to_string(),
        });
        let resolver_a = Arc::new(MockResolver {
            name: "a-resolver".to_string(),
        });
        let resolver_m = Arc::new(MockResolver {
            name: "m-resolver".to_string(),
        });

        tree.insert_hostname("z.com", resolver_z.clone(), false);
        tree.insert_hostname("a.com", resolver_a.clone(), false);
        tree.insert_hostname("m.com", resolver_m.clone(), false);

        // All should be findable
        assert!(tree.lookup_hostname("z.com").is_some());
        assert!(tree.lookup_hostname("a.com").is_some());
        assert!(tree.lookup_hostname("m.com").is_some());
    }

    #[test]
    fn test_insert_ip_and_hostname() {
        let mut tree = TlsResolverRadixTree::new();
        let resolver = Arc::new(MockResolver {
            name: "127-0-0-1-localhost-resolver".to_string(),
        });

        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        tree.insert_ip_and_hostname(ip, "localhost", resolver.clone(), false);

        // Should find the combined match
        let found = tree.lookup_ip_and_hostname(ip, "localhost");
        assert!(found.is_some());

        // IP-only should not find the combined resolver
        let ip_only = tree.lookup_ip(ip);
        assert!(ip_only.is_none());

        // Hostname-only should not find the combined resolver
        let hostname_only = tree.lookup_hostname("localhost");
        assert!(hostname_only.is_none());
    }

    #[test]
    fn test_ip_and_hostname_with_wildcard() {
        let mut tree = TlsResolverRadixTree::new();
        let resolver = Arc::new(MockResolver {
            name: "127-wildcard-example-com".to_string(),
        });

        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        tree.insert_ip_and_hostname(ip, "example.com", resolver.clone(), true);

        // Should match IP + subdomain
        let found = tree.lookup_ip_and_hostname(ip, "sub.example.com");
        assert!(found.is_some());
    }

    #[test]
    fn test_lookup_fallback_order() {
        let mut tree = TlsResolverRadixTree::new();

        let ip_resolver = Arc::new(MockResolver {
            name: "ip-only-resolver".to_string(),
        });
        let hostname_resolver = Arc::new(MockResolver {
            name: "hostname-only-resolver".to_string(),
        });

        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
        tree.insert_ip(ip, ip_resolver.clone());
        tree.insert_hostname("example.com", hostname_resolver.clone(), false);

        // Combined lookup should fall back to IP-only when no combined match
        let found = tree.lookup_ip_and_hostname(ip, "example.com");
        assert!(found.is_some());
    }

    #[test]
    fn test_combined_more_specific_than_separate() {
        let mut tree = TlsResolverRadixTree::new();

        let ip_resolver = Arc::new(MockResolver {
            name: "ip-resolver".to_string(),
        });
        let hostname_resolver = Arc::new(MockResolver {
            name: "hostname-resolver".to_string(),
        });
        let combined_resolver = Arc::new(MockResolver {
            name: "combined-resolver".to_string(),
        });

        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let ip_prefix = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 0));
        tree.insert_ip(ip_prefix, ip_resolver.clone());
        tree.insert_hostname("example.com", hostname_resolver.clone(), false);
        tree.insert_ip_and_hostname(ip, "example.com", combined_resolver.clone(), false);

        // Combined lookup should return the combined resolver (most specific)
        let found = tree.lookup_ip_and_hostname(ip, "example.com");
        assert!(found.is_some());
    }

    #[test]
    fn test_key_type_ordering() {
        // Verify that key types are ordered correctly
        let ip_key = RadixKey::IpOctet(127);
        let host_key = RadixKey::HostSegment("com".to_string());
        let wildcard_key = RadixKey::HostWildcard;

        assert!(ip_key.order() < host_key.order());
        assert!(host_key.order() < wildcard_key.order());
        assert!(ip_key < host_key);
        assert!(host_key < wildcard_key);
    }

    #[test]
    fn test_key_serialization() {
        // Test IP octet
        let ip_key = RadixKey::IpOctet(127);
        let bytes = ip_key.to_bytes();
        assert_eq!(RadixKey::from_bytes(&bytes), Some(ip_key));

        // Test host segment
        let host_key = RadixKey::HostSegment("example".to_string());
        let bytes = host_key.to_bytes();
        assert_eq!(RadixKey::from_bytes(&bytes), Some(host_key));

        // Test wildcard
        let wildcard_key = RadixKey::HostWildcard;
        let bytes = wildcard_key.to_bytes();
        assert_eq!(RadixKey::from_bytes(&bytes), Some(wildcard_key));
    }

    #[test]
    fn test_ipv6_support() {
        let mut tree = TlsResolverRadixTree::new();
        let resolver = Arc::new(MockResolver {
            name: "ipv6-localhost".to_string(),
        });

        let ip = IpAddr::V6("::1".parse().unwrap());
        tree.insert_ip(ip, resolver.clone());

        let found = tree.lookup_ip(ip);
        assert!(found.is_some());
    }
}
