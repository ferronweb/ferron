use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{collections::HashSet, error::Error};

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty};
use hyper::{header, Request, Response, StatusCode, Uri};

use ferron_common::config::ServerConfiguration;
use ferron_common::logging::ErrorLogger;
use ferron_common::util::{is_localhost, ModuleCache};
use ferron_common::{get_entries_for_validation, get_entry, get_value, get_values_for_validation};

use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, RequestData, ResponseData, SocketData};

/// A core module loader
pub struct CoreModuleLoader {
  cache: ModuleCache<CoreModule>,
  has_https: Arc<AtomicBool>,
}

impl Default for CoreModuleLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl CoreModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec![]),
      has_https: Arc::new(AtomicBool::new(false)),
    }
  }
}

impl ModuleLoader for CoreModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    global_config: Option<&ServerConfiguration>,
    _secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    if !config.filters.is_global_non_host()
      && (get_value!("auto_tls", config).and_then(|v| v.as_bool()).unwrap_or(
        (config.filters.hostname.is_some() || config.filters.ip.is_some())
          && config.filters.port.is_none()
          && !is_localhost(config.filters.ip.as_ref(), config.filters.hostname.as_deref()),
      ) || config.entries.contains_key("tls"))
    {
      self.has_https.store(true, Ordering::Relaxed);
    }
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, |_| {
          Ok(Arc::new(CoreModule {
            default_http_port: global_config
              .and_then(|c| get_entry!("default_http_port", c))
              .and_then(|e| e.values.first())
              .map_or(Some(80), |v| {
                if v.is_null() {
                  None
                } else {
                  Some(v.as_i128().unwrap_or(80) as u16)
                }
              }),
            default_https_port: global_config
              .and_then(|c| get_entry!("default_https_port", c))
              .and_then(|e| e.values.first())
              .map_or(Some(443), |v| {
                if v.is_null() {
                  None
                } else {
                  Some(v.as_i128().unwrap_or(443) as u16)
                }
              }),
            has_https: self.has_https.clone(),
          }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec![]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(tls_entries) = get_entries_for_validation!("tls", config, used_properties) {
      for tls_entry in &tls_entries.inner {
        if tls_entry.values.len() != 2 {
          Err(anyhow::anyhow!(
            "The `tls` configuration property must have exactly two values"
          ))?
        } else if !tls_entry.values[0].is_string() {
          Err(anyhow::anyhow!("The path to the TLS certificate must be a string"))?
        } else if !tls_entry.values[1].is_string() {
          Err(anyhow::anyhow!("The path to the TLS private key must be a string"))?
        }
      }
    };

    if let Some(error_log_entries) = get_entries_for_validation!("error_log", config, used_properties) {
      for error_log_entry in &error_log_entries.inner {
        if error_log_entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `error_log` configuration property must have exactly one value"
          ))?
        } else if !error_log_entry.values[0].is_string() {
          Err(anyhow::anyhow!("The path to the error log must be a string"))?
        }
      }
    };

    if let Some(log_entries) = get_entries_for_validation!("log", config, used_properties) {
      for log_entry in &log_entries.inner {
        if log_entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `log` configuration property must have exactly one value"
          ))?
        } else if !log_entry.values[0].is_string() {
          Err(anyhow::anyhow!("The path to the access log must be a string"))?
        }
      }
    };

    for tls_cipher in get_values_for_validation!("tls_cipher_suite", config, used_properties) {
      if !tls_cipher.is_string() {
        Err(anyhow::anyhow!("Invalid TLS cipher suite"))?
      }
    }

    for ecdh_curve in get_values_for_validation!("tls_ecdh_curve", config, used_properties) {
      if !ecdh_curve.is_string() {
        Err(anyhow::anyhow!("Invalid ECDH curve"))?
      }
    }

    if let Some(entries) = get_entries_for_validation!("tls_min_version", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `tls_min_version` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("The minimum TLS version must be a string"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("tls_max_version", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `tls_max_version` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("The maximum TLS version must be a string"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("auto_tls", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `auto_tls` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid automatic TLS enabling option"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("default_http_port", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `default_http_port` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_integer() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid default HTTP port"))?
        } else if let Some(value) = entry.values[0].as_i128() {
          if !(0..=65535).contains(&value) {
            Err(anyhow::anyhow!("Invalid default HTTP port"))?
          }
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("default_https_port", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `default_https_port` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_integer() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid default HTTPS port"))?
        } else if let Some(value) = entry.values[0].as_i128() {
          if !(0..65536).contains(&value) {
            Err(anyhow::anyhow!("Invalid default HTTPS port"))?
          }
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("h2_initial_window_size", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `h2_initial_window_size` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_integer() {
          Err(anyhow::anyhow!("Invalid HTTP/2 initial window size"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("h2_max_frame_size", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `h2_max_frame_size` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_integer() {
          Err(anyhow::anyhow!("Invalid HTTP/2 maximum frame size"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("h2_max_concurrent_streams", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `h2_max_concurrent_streams` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_integer() {
          Err(anyhow::anyhow!("Invalid HTTP/2 maximum concurrent streams amount"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("h2_max_header_list_size", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `h2_max_header_list_size` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_integer() {
          Err(anyhow::anyhow!("Invalid HTTP/2 maximum header list size"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("h2_enable_connect_protocol", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `h2_enable_connect_protocol` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid HTTP/2 CONNECT protocol enabling option"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("protocols", config, used_properties) {
      for entry in &entries.inner {
        for value in &entry.values {
          if !value.is_string() {
            Err(anyhow::anyhow!("Invalid protocol specification"))?
          }
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("header", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 2 {
          Err(anyhow::anyhow!(
            "The `header` configuration property must have exactly two values"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("The header name must be a string"))?
        } else if !entry.values[1].is_string() {
          Err(anyhow::anyhow!("The header value must be a string"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("timeout", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `timeout` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_integer() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid HTTP server processing timeout"))?
        } else if let Some(value) = entry.values[0].as_i128() {
          if value < 0 {
            Err(anyhow::anyhow!("Invalid HTTP server processing timeout"))?
          }
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("allow_double_slashes", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `allow_double_slashes` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid double slashes allowing option"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("server_administrator_email", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `server_administrator_email` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("Invalid server administrator's email address"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("error_page", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 2 {
          Err(anyhow::anyhow!(
            "The `error_page` configuration property must have exactly two values"
          ))?
        } else if !entry.values[0].is_integer() {
          Err(anyhow::anyhow!("Invalid status code for an error page"))?
        } else if !entry.values[1].is_string() {
          Err(anyhow::anyhow!("Invalid path for an error page"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("ocsp_stapling", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `ocsp_stapling` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid OCSP stapling enabling option"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("trust_x_forwarded_for", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `trust_x_forwarded_for` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!(
            "Invalid X-Forwarded-For header handling enabling option"
          ))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("no_redirect_to_https", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `no_redirect_to_https` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid redirect to HTTPS disabling option"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("wwwredirect", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `wwwredirect` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid redirect to \"www.\" URL enabling option"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("listen_ip", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `listen_ip` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("Invalid IP address to listen to"))?
        } else if let Some(value) = entry.values[0].as_str() {
          if value.parse::<IpAddr>().is_err() {
            Err(anyhow::anyhow!("Invalid IP address to listen to"))?
          }
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("io_uring", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `io_uring` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid io_uring enabling option"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("auto_tls_contact", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `auto_tls_contact` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid ACME contact email address"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("auto_tls_cache", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `auto_tls_cache` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid ACME cache path"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("auto_tls_letsencrypt_production", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `auto_tls_letsencrypt_production` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!(
            "Invalid automatic TLS Let's Encrypt production directory enabling option"
          ))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("auto_tls_challenge", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `auto_tls_challenge` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("Invalid ACME challenge type"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("root", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `root` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid webroot"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("tcp_send_buffer", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `tcp_send_buffer` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_integer() || entry.values[0].as_i128().is_some_and(|v| v < 0) {
          Err(anyhow::anyhow!("Invalid TCP listener send buffer size"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("tcp_recv_buffer", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `tcp_recv_buffer` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_integer() || entry.values[0].as_i128().is_some_and(|v| v < 0) {
          Err(anyhow::anyhow!("Invalid TCP listener receive buffer size"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("header_remove", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `header_remove` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("The header name must be a string"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("auto_tls_directory", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `auto_tls_directory` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("Invalid ACME directory URL"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("auto_tls_no_verification", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `auto_tls_no_verification` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!(
            "Invalid ACME server TLS certificate verification disabling option"
          ))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("auto_tls_profile", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `auto_tls_profile` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid ACME profile name"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("header_replace", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 2 {
          Err(anyhow::anyhow!(
            "The `header_replace` configuration property must have exactly two values"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("The header name must be a string"))?
        } else if !entry.values[1].is_string() {
          Err(anyhow::anyhow!("The header value must be a string"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("protocol_proxy", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `protocol_proxy` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid PROXY protocol enabling option"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("auto_tls_on_demand", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `auto_tls_on_demand` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid on-demand automatic TLS enabling option"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("auto_tls_on_demand_ask", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `auto_tls_on_demand_ask` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid automatic TLS on demand ask endpoint URL"))?
        }
      }
    }

    if let Some(entries) =
      get_entries_for_validation!("auto_tls_on_demand_ask_no_verification", config, used_properties)
    {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `auto_tls_on_demand_ask_no_verification` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!(
            "Invalid automatic TLS on demand ask endpoint certificate verification disabling option"
          ))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("auto_tls_eab", config, used_properties) {
      for entry in &entries.inner {
        if (1..=2).contains(&entry.values.len()) {
          Err(anyhow::anyhow!(
            "The `auto_tls_eab` configuration property must have one (if disabled) or two values"
          ))?
        } else if !entry.values[0].is_null() && entry.values.len() != 2 {
          Err(anyhow::anyhow!(
            "The `auto_tls_eab` configuration property must have exactly two values if enabled"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("Invalid ACME EAB key ID"))?
        } else if !entry.values[1].is_string() {
          Err(anyhow::anyhow!("Invalid ACME EAB key"))?
        }
      }
    }

    Ok(())
  }
}

/// A core module
struct CoreModule {
  default_http_port: Option<u16>,
  default_https_port: Option<u16>,
  has_https: Arc<AtomicBool>,
}

impl Module for CoreModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(CoreModuleHandlers {
      default_http_port: self.default_http_port,
      default_https_port: self.default_https_port,
      has_https: self.has_https.load(Ordering::Relaxed),
    })
  }
}

/// Handlers for the core module
struct CoreModuleHandlers {
  default_http_port: Option<u16>,
  default_https_port: Option<u16>,
  has_https: bool,
}

#[async_trait(?Send)]
impl ModuleHandlers for CoreModuleHandlers {
  async fn request_handler(
    &mut self,
    mut request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    // Determine if the request is a forward proxy request
    let is_proxy_request = match request.version() {
      hyper::Version::HTTP_2 | hyper::Version::HTTP_3 => {
        request.method() == hyper::Method::CONNECT && request.uri().host().is_some()
      }
      _ => request.uri().host().is_some(),
    };

    if !is_proxy_request {
      // Remove the location prefix using an undocumented configuration property
      if let Some(path) = get_value!("UNDOCUMENTED_REMOVE_PATH_PREFIX", config).and_then(|v| v.as_str()) {
        let mut path_without_trailing_slashes = path;
        while path_without_trailing_slashes.ends_with("/") {
          path_without_trailing_slashes = &path_without_trailing_slashes[..(path_without_trailing_slashes.len() - 1)];
        }

        let mut path_prepared = path_without_trailing_slashes.to_owned();
        while path_prepared.contains("//") {
          path_prepared = path_prepared.replace("//", "/");
        }

        if cfg!(windows) {
          path_prepared = path_prepared.to_lowercase();
        }

        let original_uri = request.uri().clone();
        let request_path = original_uri.path().to_string();
        let (mut request_parts, request_body) = request.into_parts();
        let mut uri_parts = request_parts.uri.into_parts();
        let new_path = if request_path == path_prepared {
          Some("/")
        } else if request_path.starts_with(&format!("{path_prepared}/")) {
          request_path.strip_prefix(&path_prepared)
        } else {
          None
        };
        if let Some(new_path) = new_path {
          if let Some(path_and_query) = uri_parts.path_and_query {
            uri_parts.path_and_query = Some(
              format!(
                "{}{}",
                new_path,
                path_and_query.query().map_or("".to_string(), |q| format!("?{q}"))
              )
              .parse()?,
            )
          } else {
            uri_parts.path_and_query = Some(new_path.parse()?)
          }
        }
        request_parts.uri = Uri::from_parts(uri_parts)?;
        request = Request::from_parts(request_parts, request_body);
        let request_data = request.extensions_mut().get_mut::<RequestData>();
        if let Some(request_data) = request_data {
          if request_data.original_url.is_none() {
            request_data.original_url = Some(original_uri);
          }
        }
      }

      // Save the new socket address from X-Forwarded-For header
      let mut new_remote_address = None;
      if get_value!("trust_x_forwarded_for", config)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
      {
        if let Some(x_forwarded_for_value) = request.headers().get("x-forwarded-for") {
          let x_forwarded_for = x_forwarded_for_value.to_str()?;

          let prepared_remote_ip_str = match x_forwarded_for.split(",").nth(0) {
            Some(ip_address_str) => ip_address_str.replace(" ", ""),
            None => {
              return Ok(ResponseData {
                request: Some(request),
                response: None,
                response_status: Some(StatusCode::BAD_REQUEST),
                response_headers: None,
                new_remote_address: None,
              });
            }
          };

          let prepared_remote_ip: IpAddr = match prepared_remote_ip_str.parse() {
            Ok(ip_address) => ip_address,
            Err(_) => {
              return Ok(ResponseData {
                request: Some(request),
                response: None,
                response_status: Some(StatusCode::BAD_REQUEST),
                response_headers: None,
                new_remote_address: None,
              });
            }
          };

          new_remote_address = Some(SocketAddr::new(prepared_remote_ip, socket_data.remote_addr.port()));
        }
      }

      // Redirect from HTTP to HTTPS when there are configurations with HTTPS, port is set implicitly, and TLS is enabled
      if !get_value!("no_redirect_to_https", config)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        && self.has_https
        && config.filters.port.is_none()
        && (get_value!("auto_tls", config).and_then(|v| v.as_bool()).unwrap_or(
          (config.filters.hostname.is_some()
            || config.filters.ip.is_some()
            || get_value!("auto_tls_on_demand", config)
              .and_then(|v| v.as_bool())
              .unwrap_or(false))
            && !is_localhost(config.filters.ip.as_ref(), config.filters.hostname.as_deref()),
        ) || config.entries.contains_key("tls"))
      {
        if let Some(default_http_port) = self.default_http_port {
          if let Some(default_https_port) = self.default_https_port {
            if !socket_data.encrypted && socket_data.local_addr.port() == default_http_port {
              let host_header_option = request.headers().get(header::HOST);
              let host_header = match host_header_option {
                Some(header_data) => header_data.to_str()?,
                None => {
                  return Ok(ResponseData {
                    request: Some(request),
                    response: None,
                    response_status: Some(StatusCode::BAD_REQUEST),
                    response_headers: None,
                    new_remote_address: None,
                  });
                }
              };

              let path_and_query_option = request.uri().path_and_query();
              let path_and_query = match path_and_query_option {
                Some(path_and_query) => path_and_query.to_string(),
                None => {
                  return Ok(ResponseData {
                    request: Some(request),
                    response: None,
                    response_status: Some(StatusCode::BAD_REQUEST),
                    response_headers: None,
                    new_remote_address: None,
                  });
                }
              };

              let mut parts: Vec<&str> = host_header.split(':').collect();

              if parts.len() > 1
                && !(parts[0].starts_with('[') && parts.last().map(|part| part.ends_with(']')).unwrap_or(false))
              {
                parts.pop();
              }

              let host_name = parts.join(":");

              let new_uri = Uri::builder()
                .scheme("https")
                .authority(match default_https_port {
                  443 => host_name,
                  port => format!("{host_name}:{port}"),
                })
                .path_and_query(path_and_query)
                .build()?;

              return Ok(ResponseData {
                request: Some(request),
                response: Some(
                  Response::builder()
                    .status(StatusCode::MOVED_PERMANENTLY)
                    .header(header::LOCATION, new_uri.to_string())
                    .body(Empty::new().map_err(|e| match e {}).boxed())?,
                ),
                response_status: None,
                response_headers: None,
                new_remote_address,
              });
            }
          }
        }
      }

      // Redirect from URL without "www." to URL with "www."
      if let Some(domain) = config.filters.hostname.as_deref() {
        if get_value!("wwwredirect", config)
          .and_then(|v| v.as_bool())
          .unwrap_or(false)
        {
          // Even more code rewritten from SVR.JS (and Ferron 1.x, of course)...
          if let Some(host_header_value) = request.headers().get(header::HOST) {
            let host_header = host_header_value.to_str()?;

            let path_and_query_option = request.uri().path_and_query();
            let path_and_query = match path_and_query_option {
              Some(path_and_query) => path_and_query.to_string(),
              None => {
                return Ok(ResponseData {
                  request: Some(request),
                  response: None,
                  response_status: Some(StatusCode::BAD_REQUEST),
                  response_headers: None,
                  new_remote_address: None,
                });
              }
            };

            let mut parts: Vec<&str> = host_header.split(':').collect();
            let mut host_port: Option<&str> = None;

            if parts.len() > 1
              && !(parts[0].starts_with('[') && parts.last().map(|part| part.ends_with(']')).unwrap_or(false))
            {
              host_port = parts.pop();
            }

            let host_name = parts.join(":");

            if host_name == domain
              && (host_port.is_none()
                || Some(
                  host_port
                    .unwrap_or(if socket_data.encrypted { "443" } else { "80" })
                    .parse::<u16>(),
                ) == config
                  .filters
                  .port
                  .or(if socket_data.encrypted {
                    self.default_https_port
                  } else {
                    self.default_http_port
                  })
                  .map(Ok))
              && !host_name.starts_with("www.")
            {
              let new_uri = Uri::builder()
                .scheme(match socket_data.encrypted {
                  true => "https",
                  false => "http",
                })
                .authority(match host_port {
                  Some(port) => format!("www.{host_name}:{port}"),
                  None => host_name,
                })
                .path_and_query(path_and_query)
                .build()?;

              return Ok(ResponseData {
                request: Some(request),
                response: Some(
                  Response::builder()
                    .status(StatusCode::MOVED_PERMANENTLY)
                    .header(header::LOCATION, new_uri.to_string())
                    .body(Empty::new().map_err(|e| match e {}).boxed())?,
                ),
                response_status: None,
                response_headers: None,
                new_remote_address,
              });
            }
          }
        }
      }

      Ok(ResponseData {
        request: Some(request),
        response: None,
        response_status: None,
        response_headers: None,
        new_remote_address,
      })
    } else {
      Ok(ResponseData {
        request: Some(request),
        response: None,
        response_status: None,
        response_headers: None,
        new_remote_address: None,
      })
    }
  }
}
