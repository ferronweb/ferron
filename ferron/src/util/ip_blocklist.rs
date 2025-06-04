use std::collections::HashSet;
use std::net::{IpAddr, Ipv6Addr};

/// The IP blocklist
pub struct IpBlockList {
  blocked_ips: HashSet<IpAddr>,
}

impl IpBlockList {
  /// Creates a new empty block list
  pub fn new() -> Self {
    Self {
      blocked_ips: HashSet::new(),
    }
  }

  /// Loads the block list from a vector of IP address strings
  pub fn load_from_vec(&mut self, ip_list: Vec<String>) {
    for ip_str in ip_list {
      match ip_str.as_str() {
        "localhost" => {
          self
            .blocked_ips
            .insert(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1).into());
        }
        _ => {
          if let Ok(ip) = ip_str.parse::<IpAddr>() {
            self.blocked_ips.insert(ip.to_canonical());
          }
        }
      }
    }
  }

  /// Checks if an IP address is blocked
  pub fn is_blocked(&self, ip: IpAddr) -> bool {
    self.blocked_ips.contains(&ip.to_canonical())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_ip_block_list() {
    let mut block_list = IpBlockList::new();
    block_list.load_from_vec(vec!["192.168.1.1".into(), "10.0.0.1".into()]);

    assert!(block_list.is_blocked("192.168.1.1".parse().unwrap()));
    assert!(block_list.is_blocked("10.0.0.1".parse().unwrap()));
    assert!(!block_list.is_blocked("8.8.8.8".parse().unwrap()));
  }
}
