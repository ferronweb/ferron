use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cachability {
  NoCache,
  Private,
  Public,
}

#[derive(Debug, Clone, Default)]
pub struct CacheControl {
  pub no_store: bool,
  pub no_cache: bool,
  pub cachability: Option<Cachability>,
  pub max_age: Option<Duration>,
  pub s_max_age: Option<Duration>,
}

impl CacheControl {
  pub fn from_value(value: &str) -> Option<Self> {
    let mut cc = CacheControl::default();
    let parts = value.split(',');

    for part in parts {
      let part = part.trim();
      if part.is_empty() {
        continue;
      }

      // Split by '=' to handle key=value
      let mut kv = part.splitn(2, '=');
      let key = kv.next()?.trim();
      let value = kv.next().map(|v| v.trim());

      if key.eq_ignore_ascii_case("no-store") {
        cc.no_store = true;
      } else if key.eq_ignore_ascii_case("no-cache") {
        cc.no_cache = true;
        cc.cachability = Some(Cachability::NoCache);
      } else if key.eq_ignore_ascii_case("private") {
        cc.cachability = Some(Cachability::Private);
      } else if key.eq_ignore_ascii_case("public") {
        cc.cachability = Some(Cachability::Public);
      } else if key.eq_ignore_ascii_case("max-age") {
        if let Some(secs) = value
          .map(|v| v.trim_matches('"'))
          .and_then(|v| v.parse::<u64>().ok())
        {
          cc.max_age = Some(Duration::from_secs(secs));
        }
      } else if key.eq_ignore_ascii_case("s-maxage") {
        if let Some(secs) = value
          .map(|v| v.trim_matches('"'))
          .and_then(|v| v.parse::<u64>().ok())
        {
          cc.s_max_age = Some(Duration::from_secs(secs));
        }
      }
    }

    Some(cc)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_parse_no_store() {
    let cc = CacheControl::from_value("no-store").unwrap();
    assert!(cc.no_store);
  }

  #[test]
  fn test_parse_no_cache() {
    let cc = CacheControl::from_value("no-cache").unwrap();
    assert!(cc.no_cache);
    assert_eq!(cc.cachability, Some(Cachability::NoCache));
  }

  #[test]
  fn test_parse_public_max_age() {
    let cc = CacheControl::from_value("public, max-age=3600").unwrap();
    assert_eq!(cc.cachability, Some(Cachability::Public));
    assert_eq!(cc.max_age, Some(Duration::from_secs(3600)));
  }

  #[test]
  fn test_parse_s_maxage() {
    let cc = CacheControl::from_value("s-maxage=600").unwrap();
    assert_eq!(cc.s_max_age, Some(Duration::from_secs(600)));
  }

  #[test]
  fn test_parse_complex() {
    let cc = CacheControl::from_value("private, no-store, max-age=0").unwrap();
    assert_eq!(cc.cachability, Some(Cachability::Private));
    assert!(cc.no_store);
    assert_eq!(cc.max_age, Some(Duration::from_secs(0)));
  }

  #[test]
  fn test_parse_case_insensitive() {
    let cc = CacheControl::from_value("PuBlIc, Max-Age=100").unwrap();
    assert_eq!(cc.cachability, Some(Cachability::Public));
    assert_eq!(cc.max_age, Some(Duration::from_secs(100)));
  }

  #[test]
  fn test_parse_extra_whitespace() {
    let cc = CacheControl::from_value(" public , max-age = 60 ").unwrap();
    assert_eq!(cc.cachability, Some(Cachability::Public));
    assert_eq!(cc.max_age, Some(Duration::from_secs(60)));
  }
}
