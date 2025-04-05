use fancy_regex::Regex;

pub struct UrlRewriteMapEntry {
  pub regex: Regex,
  pub replacement: String,
  pub is_not_directory: bool,
  pub is_not_file: bool,
  pub last: bool,
  pub allow_double_slashes: bool,
}

impl UrlRewriteMapEntry {
  pub fn new(
    regex: Regex,
    replacement: String,
    is_not_directory: bool,
    is_not_file: bool,
    last: bool,
    allow_double_slashes: bool,
  ) -> Self {
    Self {
      regex,
      replacement,
      is_not_directory,
      is_not_file,
      last,
      allow_double_slashes,
    }
  }
}

pub struct UrlRewriteMapWrap {
  pub domain: Option<String>,
  pub ip: Option<String>,
  pub rewrite_map: Vec<UrlRewriteMapEntry>,
  pub locations: Vec<UrlRewriteMapLocationWrap>,
}

impl UrlRewriteMapWrap {
  pub fn new(
    domain: Option<String>,
    ip: Option<String>,
    rewrite_map: Vec<UrlRewriteMapEntry>,
    locations: Vec<UrlRewriteMapLocationWrap>,
  ) -> Self {
    Self {
      domain,
      ip,
      rewrite_map,
      locations,
    }
  }
}

pub struct UrlRewriteMapLocationWrap {
  pub path: String,
  pub rewrite_map: Vec<UrlRewriteMapEntry>,
}

impl UrlRewriteMapLocationWrap {
  pub fn new(path: String, rewrite_map: Vec<UrlRewriteMapEntry>) -> Self {
    Self { path, rewrite_map }
  }
}
