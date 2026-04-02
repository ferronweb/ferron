use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::Arc;

/// Key types for the radix tree, ordered from root to leaf.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum RadixKey {
    /// IPv4 address octet (e.g., `127` from `127.0.0.1`)
    IpV4Octet(u8),
    /// IPv6 address octet (e.g., first byte from `2001:db8::`)
    IpV6Octet(u8),
    /// Hostname segment (e.g., `"com"`, `"example"` from `"example.com"`)
    HostSegment(String),
    /// Hostname wildcard (`"*"` for `"*.example.com"`)
    HostWildcard,
}

impl RadixKey {
    /// Returns the sort order for this key type (lower = closer to root).
    #[inline]
    #[allow(dead_code)]
    fn order(&self) -> u8 {
        match self {
            RadixKey::IpV4Octet(_) => 0,
            RadixKey::IpV6Octet(_) => 1,
            RadixKey::HostSegment(_) => 2,
            RadixKey::HostWildcard => 3,
        }
    }

    /// Converts the key to bytes for storage in the BTreeMap.
    fn to_bytes(&self) -> Vec<u8> {
        match self {
            RadixKey::IpV4Octet(octet) => vec![0x00, *octet],
            RadixKey::IpV6Octet(octet) => vec![0x01, *octet],
            RadixKey::HostSegment(segment) => {
                let mut bytes = vec![0x02];
                bytes.extend_from_slice(segment.as_bytes());
                bytes
            }
            RadixKey::HostWildcard => vec![0x03, b'*'],
        }
    }

    /// Parses bytes back into a key.
    #[allow(dead_code)]
    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() {
            return None;
        }
        match bytes[0] {
            0x00 => bytes.get(1).copied().map(RadixKey::IpV4Octet),
            0x01 => bytes.get(1).copied().map(RadixKey::IpV6Octet),
            0x02 => String::from_utf8(bytes[1..].to_vec())
                .ok()
                .map(RadixKey::HostSegment),
            0x03 => Some(RadixKey::HostWildcard),
            _ => None,
        }
    }
}

/// A compressed radix tree node for generic data lookup.
struct RadixNode<T> {
    /// Compressed edge label leading to this node
    edge: Vec<u8>,
    /// Data stored at this node (if any)
    data: Option<T>,
    /// Child nodes (BTreeMap for ordered traversal)
    children: BTreeMap<Vec<u8>, RadixNode<T>>,
}

impl<T> RadixNode<T> {
    #[inline]
    fn new(edge: Vec<u8>) -> Self {
        Self {
            edge,
            data: None,
            children: BTreeMap::new(),
        }
    }

    #[inline]
    fn with_data(edge: Vec<u8>, data: T) -> Self {
        Self {
            edge,
            data: Some(data),
            children: BTreeMap::new(),
        }
    }
}

/// A compressed radix tree for storing and looking up generic data.
///
/// The tree organizes data in a hierarchy:
/// - Root level: Default/fallback data
/// - First level: IPv4/IPv6 address octets
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
/// use std::sync::Arc;
///
/// // With Arc<dyn Trait>
/// let mut tree: RadixTree<Arc<dyn TcpTlsResolver>> = RadixTree::new();
/// tree.set_root_resolver(default_resolver.clone());
/// tree.insert_ip(IpAddr::from([127, 0, 0, 1]), resolver.clone());
///
/// // With simple values
/// let mut tree: RadixTree<String> = RadixTree::new();
/// tree.set_root_data("default".to_string());
/// tree.insert_hostname("example.com", "example".to_string(), false);
/// ```
pub struct RadixTree<T> {
    root: RadixNode<T>,
}

impl<T: Clone> RadixTree<T> {
    /// Creates a new empty radix tree.
    #[inline]
    pub fn new() -> Self {
        Self {
            root: RadixNode::new(Vec::new()),
        }
    }

    /// Sets the root (default) data.
    ///
    /// This data is returned as a fallback when no other match is found.
    pub fn set_root_data(&mut self, data: T) {
        self.root.data = Some(data);
    }

    /// Gets the root (default) data, if set.
    #[inline]
    pub fn root_data(&self) -> Option<T> {
        self.root.data.clone()
    }

    /// Clears the root (default) data.
    #[inline]
    pub fn clear_root_data(&mut self) {
        self.root.data = None;
    }

    /// Inserts data into the tree at the specified path.
    ///
    /// The path components are ordered from root to leaf:
    /// - For IP-based: `[IpOctet(127), IpOctet(0), IpOctet(0), IpOctet(1)]`
    /// - For hostname: `[HostSegment("com"), HostSegment("example")]` (reversed)
    /// - For wildcard: `[HostWildcard, HostSegment("com"), HostSegment("example")]`
    /// - For combined IP+hostname: IP octets followed by hostname segments
    pub fn insert(&mut self, path: &[RadixKey], data: T) {
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
                        // Exact match - update data if this is the last segment
                        if is_last {
                            child.data = Some(data.clone());
                        }
                        child
                    } else if child_edge.starts_with(&bytes) {
                        // New node is a prefix of existing child - split
                        let remaining = child_edge[bytes.len()..].to_vec();
                        child.edge = bytes.clone();

                        if is_last {
                            child.data = Some(data.clone());
                        }

                        // Create new child for the remaining edge
                        let existing_children = std::mem::take(&mut child.children);
                        let new_child = RadixNode {
                            edge: remaining.clone(),
                            data: None,
                            children: existing_children,
                        };

                        child.children.insert(remaining, new_child);
                        child
                    } else {
                        // Existing child edge is a prefix of new bytes
                        // Need to traverse deeper or create intermediate node
                        let remaining = &bytes[child_edge.len()..];
                        Self::insert_into_child(child, remaining, is_last, &data);
                        child
                    }
                } else {
                    // No matching child - create new node
                    let new_node = if is_last {
                        RadixNode::with_data(bytes.clone(), data.clone())
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
        child: &'a mut RadixNode<T>,
        remaining: &[u8],
        is_last: bool,
        data: &T,
    ) -> &'a mut RadixNode<T> {
        let mut current = child;
        let mut offset = 0;

        while offset < remaining.len() {
            let edge = current.edge.clone();

            if edge.is_empty() {
                current.edge = remaining[offset..].to_vec();
                if is_last && offset + edge.len() >= remaining.len() {
                    current.data = Some(data.clone());
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
                    RadixNode::with_data(remaining[offset..].to_vec(), data.clone())
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

    /// Looks up data by path components.
    ///
    /// Returns the data at the highest (most specific) level found during traversal.
    /// If no match is found and root data is set, returns the root data as fallback.
    pub fn lookup(&self, path: &[RadixKey]) -> Option<T> {
        let mut current = &self.root;
        // Start with root data as the initial best match (fallback)
        let mut best_match: Option<T> = self.root.data.clone();

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
                    // Check if this node has data
                    if child.data.is_some() {
                        best_match = child.data.clone();
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

        // Check final node for data
        current.data.clone().or(best_match)
    }

    /// Converts an IP address to a path of IP octet keys.
    fn ip_to_path(ip: IpAddr) -> Vec<RadixKey> {
        match ip {
            IpAddr::V4(addr) => addr
                .octets()
                .iter()
                .copied()
                .map(RadixKey::IpV4Octet)
                .collect(),
            IpAddr::V6(addr) => addr
                .octets()
                .iter()
                .copied()
                .map(RadixKey::IpV6Octet)
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

    /// Inserts data for an IP address.
    ///
    /// For IPv4, all 4 octets are used. For IPv6, all 16 bytes are used.
    pub fn insert_ip(&mut self, ip: IpAddr, data: T) {
        let path = Self::ip_to_path(ip);
        self.insert(&path, data);
    }

    /// Inserts data for a partial IPv4 address prefix.
    ///
    /// Useful for matching ranges like `127.x.x.x` or `192.168.x.x`.
    pub fn insert_ipv4_prefix(&mut self, prefix: &[u8], data: T) {
        let path: Vec<RadixKey> = prefix.iter().copied().map(RadixKey::IpV4Octet).collect();
        self.insert(&path, data);
    }

    /// Inserts data for a partial IPv6 address prefix.
    ///
    /// Useful for matching ranges like `2001:db8::/32`.
    pub fn insert_ipv6_prefix(&mut self, prefix: &[u8], data: T) {
        let path: Vec<RadixKey> = prefix.iter().copied().map(RadixKey::IpV6Octet).collect();
        self.insert(&path, data);
    }

    /// Inserts data for a hostname (with optional wildcard).
    ///
    /// If `wildcard` is true, the data will match `*.hostname`.
    pub fn insert_hostname(&mut self, hostname: &str, data: T, wildcard: bool) {
        let path = Self::hostname_to_path(hostname, wildcard);
        self.insert(&path, data);
    }

    /// Inserts data for both an IP address and hostname.
    ///
    /// This creates a more specific path: IP octets followed by hostname segments.
    /// The data will only match when both the IP and hostname match.
    ///
    /// # Arguments
    ///
    /// * `ip` - IP address
    /// * `hostname` - Hostname (e.g., `"localhost"`)
    /// * `data` - The data to store
    /// * `wildcard` - If true, matches subdomains (e.g., `*.example.com`)
    pub fn insert_ip_and_hostname(&mut self, ip: IpAddr, hostname: &str, data: T, wildcard: bool) {
        let mut path = Self::ip_to_path(ip);
        path.extend(Self::hostname_to_path(hostname, wildcard));
        self.insert(&path, data);
    }

    /// Looks up data by IP address.
    pub fn lookup_ip(&self, ip: IpAddr) -> Option<T> {
        let path = Self::ip_to_path(ip);
        self.lookup(&path)
    }

    /// Looks up data by IPv4 address prefix.
    pub fn lookup_ipv4_prefix(&self, prefix: &[u8]) -> Option<T> {
        let path: Vec<RadixKey> = prefix.iter().copied().map(RadixKey::IpV4Octet).collect();
        self.lookup(&path)
    }

    /// Looks up data by IPv6 address prefix.
    pub fn lookup_ipv6_prefix(&self, prefix: &[u8]) -> Option<T> {
        let path: Vec<RadixKey> = prefix.iter().copied().map(RadixKey::IpV6Octet).collect();
        self.lookup(&path)
    }

    /// Looks up data by hostname.
    ///
    /// Attempts to find the most specific match, checking:
    /// 1. Exact hostname match
    /// 2. Wildcard match for parent domain
    pub fn lookup_hostname(&self, hostname: &str) -> Option<T> {
        // Try exact match first
        let exact_path = Self::hostname_to_path(hostname, false);
        if let Some(data) = self.lookup(&exact_path) {
            return Some(data);
        }

        // Try wildcard match (add "*" at the beginning of reversed segments)
        let wildcard_path = Self::hostname_to_path(hostname, true);
        self.lookup(&wildcard_path)
    }

    /// Looks up data by both IP address and hostname.
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
    /// The most specific data found, or `None` if no match.
    pub fn lookup_ip_and_hostname(&self, ip: IpAddr, hostname: &str) -> Option<T> {
        // Build full path: IP octets + hostname segments
        let mut full_path = Self::ip_to_path(ip);
        full_path.extend(Self::hostname_to_path(hostname, false));

        // Try exact IP + exact hostname
        if let Some(data) = self.lookup(&full_path) {
            return Some(data);
        }

        // Try exact IP + wildcard hostname
        let mut wildcard_path = Self::ip_to_path(ip);
        wildcard_path.extend(Self::hostname_to_path(hostname, true));
        if let Some(data) = self.lookup(&wildcard_path) {
            return Some(data);
        }

        // Fall back to IP-only lookup
        if let Some(data) = self.lookup_ip(ip) {
            return Some(data);
        }

        // Fall back to hostname-only lookup
        self.lookup_hostname(hostname)
    }
}

impl<T: Clone> Default for RadixTree<T> {
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
    fn test_insert_ipv4_prefix() {
        let mut tree = RadixTree::new();
        tree.insert_ipv4_prefix(&[127], "127-prefix".to_string());

        let found = tree.lookup_ipv4_prefix(&[127]);
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "127-prefix");
    }

    #[test]
    fn test_insert_ipv6_prefix() {
        let mut tree = RadixTree::new();
        tree.insert_ipv6_prefix(&[0x20, 0x01], "2001-prefix".to_string());

        let found = tree.lookup_ipv6_prefix(&[0x20, 0x01]);
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "2001-prefix");
    }

    #[test]
    fn test_ipv4_ipv6_separation() {
        let mut tree = RadixTree::new();

        // Insert same octet value for both IPv4 and IPv6
        tree.insert_ipv4_prefix(&[127], "ipv4-127".to_string());
        tree.insert_ipv6_prefix(&[127], "ipv6-127".to_string());

        // They should be stored separately
        let ipv4_found = tree.lookup_ipv4_prefix(&[127]);
        assert!(ipv4_found.is_some());
        assert_eq!(ipv4_found.unwrap(), "ipv4-127");

        let ipv6_found = tree.lookup_ipv6_prefix(&[127]);
        assert!(ipv6_found.is_some());
        assert_eq!(ipv6_found.unwrap(), "ipv6-127");
    }

    #[test]
    fn test_insert_and_lookup_hostname() {
        let mut tree = RadixTree::new();
        tree.insert_hostname("example.com", "example-com".to_string(), false);

        let found = tree.lookup_hostname("example.com");
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "example-com");

        let not_found = tree.lookup_hostname("test.com");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_wildcard_lookup() {
        let mut tree = RadixTree::new();
        tree.insert_hostname("example.com", "wildcard-example-com".to_string(), true);

        let found = tree.lookup_hostname("sub.example.com");
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "wildcard-example-com");
    }

    #[test]
    fn test_hierarchy_priority() {
        let mut tree = RadixTree::new();

        tree.insert_hostname("com", "com-resolver".to_string(), false);
        tree.insert_hostname("example.com", "example-com-resolver".to_string(), false);

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
        tree.insert_hostname("localhost", "localhost".to_string(), false);

        assert!(tree
            .lookup_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)))
            .is_some());
        assert!(tree.lookup_hostname("localhost").is_some());
    }

    #[test]
    fn test_btree_ordering() {
        let mut tree = RadixTree::new();

        tree.insert_hostname("z.com", "z-resolver".to_string(), false);
        tree.insert_hostname("a.com", "a-resolver".to_string(), false);
        tree.insert_hostname("m.com", "m-resolver".to_string(), false);

        assert!(tree.lookup_hostname("z.com").is_some());
        assert!(tree.lookup_hostname("a.com").is_some());
        assert!(tree.lookup_hostname("m.com").is_some());
    }

    #[test]
    fn test_insert_ip_and_hostname() {
        let mut tree = RadixTree::new();
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        tree.insert_ip_and_hostname(ip, "localhost", "combined".to_string(), false);

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
        tree.insert_ip_and_hostname(ip, "example.com", "wildcard".to_string(), true);

        let found = tree.lookup_ip_and_hostname(ip, "sub.example.com");
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "wildcard");
    }

    #[test]
    fn test_lookup_fallback_order() {
        let mut tree = RadixTree::new();

        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
        tree.insert_ip(ip, "ip-only".to_string());
        tree.insert_hostname("example.com", "hostname-only".to_string(), false);

        let found = tree.lookup_ip_and_hostname(ip, "example.com");
        assert!(found.is_some());
    }

    #[test]
    fn test_combined_more_specific_than_separate() {
        let mut tree = RadixTree::new();

        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let ip_prefix = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 0));
        tree.insert_ip(ip_prefix, "ip-resolver".to_string());
        tree.insert_hostname("example.com", "hostname-resolver".to_string(), false);
        tree.insert_ip_and_hostname(ip, "example.com", "combined-resolver".to_string(), false);

        let found = tree.lookup_ip_and_hostname(ip, "example.com");
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "combined-resolver");
    }

    #[test]
    fn test_key_type_ordering() {
        let ipv4_key = RadixKey::IpV4Octet(127);
        let ipv6_key = RadixKey::IpV6Octet(127);
        let host_key = RadixKey::HostSegment("com".to_string());
        let wildcard_key = RadixKey::HostWildcard;

        assert!(ipv4_key.order() < ipv6_key.order());
        assert!(ipv6_key.order() < host_key.order());
        assert!(host_key.order() < wildcard_key.order());
        assert!(ipv4_key < ipv6_key);
        assert!(ipv6_key < host_key);
        assert!(host_key < wildcard_key);
    }

    #[test]
    fn test_key_serialization() {
        // Test IPv4 octet
        let ipv4_key = RadixKey::IpV4Octet(127);
        let bytes = ipv4_key.to_bytes();
        assert_eq!(RadixKey::from_bytes(&bytes), Some(ipv4_key));

        // Test IPv6 octet
        let ipv6_key = RadixKey::IpV6Octet(0x20);
        let bytes = ipv6_key.to_bytes();
        assert_eq!(RadixKey::from_bytes(&bytes), Some(ipv6_key));

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
    fn test_same_value_ipv4_ipv6_distinction() {
        // Verify that same octet value is distinguished between IPv4 and IPv6
        let ipv4_key = RadixKey::IpV4Octet(127);
        let ipv6_key = RadixKey::IpV6Octet(127);

        assert_ne!(ipv4_key.to_bytes(), ipv6_key.to_bytes());
        assert_ne!(ipv4_key, ipv6_key);
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
    fn test_clear_root_data() {
        let mut tree = RadixTree::new();
        tree.set_root_data("root-resolver".to_string());
        assert!(tree.root_data().is_some());

        tree.clear_root_data();
        assert!(tree.root_data().is_none());

        let ip = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        let found = tree.lookup_ip(ip);
        assert!(found.is_none());
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
