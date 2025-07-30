use std::net::IpAddr;

/// Determines if the server configuration filter implies `localhost`.
pub fn is_localhost(ip: Option<&IpAddr>, hostname: Option<&str>) -> bool {
  if let Some(ip) = ip {
    if ip.to_canonical().is_loopback() {
      return true;
    }
  }
  if let Some(hostname) = hostname {
    let normalized_hostname = hostname.to_lowercase();
    let normalized_hostname = normalized_hostname.trim_end_matches('.');
    if normalized_hostname == "localhost" || normalized_hostname.ends_with(".localhost") {
      return true;
    }
  }
  false
}
