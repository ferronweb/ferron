pub struct AsgiApplicationWrap {
  pub domain: Option<String>,
  pub ip: Option<String>,
  pub asgi_application_id: Option<usize>,
  pub asgi_path: Option<String>,
  pub locations: Vec<AsgiApplicationLocationWrap>,
}

impl AsgiApplicationWrap {
  pub fn new(
    domain: Option<String>,
    ip: Option<String>,
    asgi_application_id: Option<usize>,
    asgi_path: Option<String>,
    locations: Vec<AsgiApplicationLocationWrap>,
  ) -> Self {
    Self {
      domain,
      ip,
      asgi_application_id,
      asgi_path,
      locations,
    }
  }
}

pub struct AsgiApplicationLocationWrap {
  pub path: String,
  pub asgi_application_id: usize,
  pub asgi_path: Option<String>,
}

impl AsgiApplicationLocationWrap {
  pub fn new(path: String, asgi_application_id: usize, asgi_path: Option<String>) -> Self {
    Self {
      path,
      asgi_application_id,
      asgi_path,
    }
  }
}
