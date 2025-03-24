use crate::ferron_util::ip_blocklist::IpBlockList;
use fancy_regex::Regex;

#[allow(dead_code)]
pub struct NonStandardCode {
  pub status_code: u16,
  pub url: Option<String>,
  pub regex: Option<Regex>,
  pub location: Option<String>,
  pub realm: Option<String>,
  pub disable_brute_force_protection: bool,
  pub user_list: Option<Vec<String>>,
  pub users: Option<IpBlockList>,
}

impl NonStandardCode {
  #[allow(clippy::too_many_arguments)]
  pub fn new(
    status_code: u16,
    url: Option<String>,
    regex: Option<Regex>,
    location: Option<String>,
    realm: Option<String>,
    disable_brute_force_protection: bool,
    user_list: Option<Vec<String>>,
    users: Option<IpBlockList>,
  ) -> Self {
    NonStandardCode {
      status_code,
      url,
      regex,
      location,
      realm,
      disable_brute_force_protection,
      user_list,
      users,
    }
  }
}

pub struct NonStandardCodesWrap {
  pub domain: Option<String>,
  pub ip: Option<String>,
  pub non_standard_codes: Vec<NonStandardCode>,
  pub locations: Vec<NonStandardCodesLocationWrap>,
}

impl NonStandardCodesWrap {
  pub fn new(
    domain: Option<String>,
    ip: Option<String>,
    non_standard_codes: Vec<NonStandardCode>,
    locations: Vec<NonStandardCodesLocationWrap>,
  ) -> Self {
    NonStandardCodesWrap {
      domain,
      ip,
      non_standard_codes,
      locations,
    }
  }
}

pub struct NonStandardCodesLocationWrap {
  pub path: String,
  pub non_standard_codes: Vec<NonStandardCode>,
}

impl NonStandardCodesLocationWrap {
  pub fn new(path: String, non_standard_codes: Vec<NonStandardCode>) -> Self {
    NonStandardCodesLocationWrap {
      path,
      non_standard_codes,
    }
  }
}
