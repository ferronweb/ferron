use std::sync::Arc;

use super::preforked_process_pool::PreforkedProcessPool;

pub struct WsgidApplicationWrap {
  pub domain: Option<String>,
  pub ip: Option<String>,
  pub wsgi_process_pool: Option<Arc<PreforkedProcessPool>>,
  pub wsgi_path: Option<String>,
  pub locations: Vec<WsgidApplicationLocationWrap>,
}

impl WsgidApplicationWrap {
  pub fn new(
    domain: Option<String>,
    ip: Option<String>,
    wsgi_process_pool: Option<Arc<PreforkedProcessPool>>,
    wsgi_path: Option<String>,
    locations: Vec<WsgidApplicationLocationWrap>,
  ) -> Self {
    Self {
      domain,
      ip,
      wsgi_process_pool,
      wsgi_path,
      locations,
    }
  }
}

pub struct WsgidApplicationLocationWrap {
  pub path: String,
  pub wsgi_process_pool: Arc<PreforkedProcessPool>,
  pub wsgi_path: Option<String>,
}

impl WsgidApplicationLocationWrap {
  pub fn new(
    path: String,
    wsgi_process_pool: Arc<PreforkedProcessPool>,
    wsgi_path: Option<String>,
  ) -> Self {
    Self {
      path,
      wsgi_process_pool,
      wsgi_path,
    }
  }
}
