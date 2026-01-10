use base64::Engine;

/// Parses the HTTP "WWW-Authenticate" header for HTTP Basic authentication
pub fn parse_basic_auth(auth_str: &str) -> Option<(String, String)> {
  if let Some(base64_credentials) = auth_str.strip_prefix("Basic ") {
    if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(base64_credentials) {
      if let Ok(decoded_str) = std::str::from_utf8(&decoded) {
        let parts: Vec<&str> = decoded_str.splitn(2, ':').collect();
        if parts.len() == 2 {
          return Some((parts[0].to_string(), parts[1].to_string()));
        }
      }
    }
  }
  None
}
