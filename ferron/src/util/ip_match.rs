use std::net::{IpAddr, Ipv6Addr};

pub fn ip_match(ip1: &str, ip2: IpAddr) -> bool {
  let ip1_processed: IpAddr = match ip1 {
    "localhost" => Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1).into(),
    _ => match ip1.parse() {
      Ok(ip_processed) => ip_processed,
      Err(_) => return false,
    },
  };

  ip1_processed == ip2
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::net::{IpAddr, Ipv6Addr};

  #[test]
  fn test_ip_match_with_valid_ipv6() {
    let ip1 = "2001:0db8:85a3:0000:0000:8a2e:0370:7334";
    let ip2 = ip1.parse::<IpAddr>().unwrap();
    assert!(ip_match(ip1, ip2));
  }

  #[test]
  fn test_ip_match_with_valid_ipv4() {
    let ip1 = "192.168.1.1";
    let ip2 = ip1.parse::<IpAddr>().unwrap();
    assert!(ip_match(ip1, ip2));
  }

  #[test]
  fn test_ip_match_with_localhost() {
    let ip1 = "localhost";
    let ip2 = Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1).into();
    assert!(ip_match(ip1, ip2));
  }

  #[test]
  fn test_ip_match_with_invalid_ip() {
    let ip1 = "invalid_ip";
    let ip2 = "192.168.1.1".parse::<IpAddr>().unwrap();
    assert!(!ip_match(ip1, ip2));
  }

  #[test]
  fn test_ip_match_with_different_ips() {
    let ip1 = "192.168.1.1";
    let ip2 = "192.168.1.2".parse::<IpAddr>().unwrap();
    assert!(!ip_match(ip1, ip2));
  }

  #[test]
  fn test_ip_match_with_empty_string() {
    let ip1 = "";
    let ip2 = "192.168.1.1".parse::<IpAddr>().unwrap();
    assert!(!ip_match(ip1, ip2));
  }

  #[test]
  fn test_ip_match_with_localhost_and_different_ip() {
    let ip1 = "localhost";
    let ip2 = "192.168.1.1".parse::<IpAddr>().unwrap();
    assert!(!ip_match(ip1, ip2));
  }
}
