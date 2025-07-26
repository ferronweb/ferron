use crate::config::ServerConfigurationFilters;

/// Determines if the server configuration filter implies `localhost`.
pub fn is_localhost(config_filters: &ServerConfigurationFilters) -> bool {
  if let Some(ip) = &config_filters.ip {
    if ip.to_canonical().is_loopback() {
      return true;
    }
  }
  if let Some(hostname) = &config_filters.hostname {
    let normalized_hostname = hostname.to_lowercase();
    let normalized_hostname = normalized_hostname.trim_end_matches('.');
    if normalized_hostname == "localhost" || normalized_hostname.ends_with(".localhost") {
      return true;
    }
  }
  false
}
