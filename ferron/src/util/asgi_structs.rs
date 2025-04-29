pub struct AsgiApplicationWrap {
  pub domain: Option<String>,
  pub ip: Option<String>,
  pub asgi_application_id: Option<usize>,
  pub asgi_application_path: Option<String>,
  pub asgi_path: Option<String>,
  pub locations: Vec<AsgiApplicationLocationWrap>,
}

impl AsgiApplicationWrap {
  pub fn new(
    domain: Option<String>,
    ip: Option<String>,
    asgi_application_id: Option<usize>,
    asgi_application_path: Option<String>,
    asgi_path: Option<String>,
    locations: Vec<AsgiApplicationLocationWrap>,
  ) -> Self {
    Self {
      domain,
      ip,
      asgi_application_id,
      asgi_application_path,
      asgi_path,
      locations,
    }
  }
}

pub struct AsgiApplicationLocationWrap {
  pub path: String,
  pub asgi_application_id: usize,
  #[allow(dead_code)]
  pub asgi_application_path: String,
  pub asgi_path: Option<String>,
}

impl AsgiApplicationLocationWrap {
  pub fn new(
    path: String,
    asgi_application_id: usize,
    asgi_application_path: String,
    asgi_path: Option<String>,
  ) -> Self {
    Self {
      path,
      asgi_application_id,
      asgi_application_path,
      asgi_path,
    }
  }
}
