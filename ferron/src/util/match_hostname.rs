// Hostname matching function from SVR.JS rewritten from JavaScript to Rust
pub fn match_hostname(hostname: Option<&str>, req_hostname: Option<&str>) -> bool {
  if hostname.is_none() || hostname == Some("*") {
    return true;
  }

  if let (Some(hostname), Some(req_hostname)) = (hostname, req_hostname) {
    if hostname.starts_with("*.") && hostname != "*." {
      let hostnames_root = &hostname[2..];
      if req_hostname == hostnames_root
        || (req_hostname.len() > hostnames_root.len()
          && req_hostname.ends_with(&format!(".{}", hostnames_root)[..]))
      {
        return true;
      }
    } else if req_hostname == hostname {
      return true;
    }
  }

  false
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn should_return_true_if_hostname_is_undefined() {
    assert!(match_hostname(None, Some("example.com")));
  }

  #[test]
  fn should_return_true_if_hostname_is_star() {
    assert!(match_hostname(Some("*"), Some("example.com")));
  }

  #[test]
  fn should_return_true_if_req_hostname_matches_hostname_exactly() {
    assert!(match_hostname(Some("example.com"), Some("example.com")));
  }

  #[test]
  fn should_return_false_if_req_hostname_does_not_match_hostname_exactly() {
    assert!(!match_hostname(Some("example.com"), Some("example.org")));
  }

  #[test]
  fn should_return_true_if_hostname_starts_with_star_dot_and_req_hostname_matches_the_root() {
    assert!(match_hostname(
      Some("*.example.com"),
      Some("sub.example.com")
    ));
  }

  #[test]
  fn should_return_false_if_hostname_starts_with_star_dot_and_req_hostname_does_not_match_the_root()
  {
    assert!(!match_hostname(Some("*.example.com"), Some("example.org")));
  }

  #[test]
  fn should_return_true_if_hostname_starts_with_star_dot_and_req_hostname_is_the_root() {
    assert!(match_hostname(Some("*.example.com"), Some("example.com")));
  }

  #[test]
  fn should_return_false_if_hostname_is_star_dot() {
    assert!(!match_hostname(Some("*."), Some("example.com")));
  }

  #[test]
  fn should_return_false_if_req_hostname_is_undefined() {
    assert!(!match_hostname(Some("example.com"), None));
  }

  #[test]
  fn should_return_false_if_hostname_does_not_start_with_star_dot_and_req_hostname_does_not_match()
  {
    assert!(!match_hostname(
      Some("sub.example.com"),
      Some("example.com")
    ));
  }

  #[test]
  fn should_return_true_if_hostname_starts_with_star_dot_and_req_hostname_matches_the_root_with_additional_subdomains(
  ) {
    assert!(match_hostname(
      Some("*.example.com"),
      Some("sub.sub.example.com")
    ));
  }

  #[test]
  fn should_return_false_if_hostname_starts_with_star_dot_and_req_hostname_does_not_match_the_root_with_additional_subdomains(
  ) {
    assert!(!match_hostname(
      Some("*.example.com"),
      Some("sub.sub.example.org")
    ));
  }
}
