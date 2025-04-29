use pyo3::types::PyAny;
use pyo3::Py;
use std::sync::Arc;

pub struct WsgiApplicationWrap {
  pub domain: Option<String>,
  pub ip: Option<String>,
  pub wsgi_application: Option<Arc<Py<PyAny>>>,
  pub wsgi_path: Option<String>,
  pub locations: Vec<WsgiApplicationLocationWrap>,
}

impl WsgiApplicationWrap {
  pub fn new(
    domain: Option<String>,
    ip: Option<String>,
    wsgi_application: Option<Arc<Py<PyAny>>>,
    wsgi_path: Option<String>,
    locations: Vec<WsgiApplicationLocationWrap>,
  ) -> Self {
    Self {
      domain,
      ip,
      wsgi_application,
      wsgi_path,
      locations,
    }
  }
}

pub struct WsgiApplicationLocationWrap {
  pub path: String,
  pub wsgi_application: Arc<Py<PyAny>>,
  pub wsgi_path: Option<String>,
}

impl WsgiApplicationLocationWrap {
  pub fn new(path: String, wsgi_application: Arc<Py<PyAny>>, wsgi_path: Option<String>) -> Self {
    Self {
      path,
      wsgi_application,
      wsgi_path,
    }
  }
}
