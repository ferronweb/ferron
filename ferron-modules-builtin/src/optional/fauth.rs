use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use ferron_common::logging::ErrorLogger;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty};
use hyper::{header, Request};

use ferron_common::http_proxy::{Connections, ReverseProxy, ReverseProxyHandler};
use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, ResponseData, SocketData};
use ferron_common::{config::ServerConfiguration, util::ModuleCache};
use ferron_common::{get_entries_for_validation, get_entry, get_value, get_values};
use hyper::header::HeaderName;

const DEFAULT_CONCURRENT_CONNECTIONS: usize = 16384;
const DEFAULT_KEEPALIVE_IDLE_TIMEOUT: u64 = 60000;

/// A forwarded authentication module loader
pub struct ForwardedAuthenticationModuleLoader {
  cache: ModuleCache<ForwardedAuthenticationModule>,
  connections: Option<Connections>,
}

impl Default for ForwardedAuthenticationModuleLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl ForwardedAuthenticationModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec!["auth_to", "auth_to_concurrent_conns", "auth_to_no_verification"]),
      connections: None,
    }
  }
}

impl ModuleLoader for ForwardedAuthenticationModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    global_config: Option<&ServerConfiguration>,
    _secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    let concurrency_limit = global_config
      .and_then(|c| get_value!("auth_to_concurrent_conns", c))
      .map_or(Some(DEFAULT_CONCURRENT_CONNECTIONS), |v| {
        if v.is_null() {
          None
        } else {
          Some(
            v.as_i128()
              .map(|v| v as usize)
              .unwrap_or(DEFAULT_CONCURRENT_CONNECTIONS),
          )
        }
      });
    let connections = self
      .connections
      .get_or_insert(if let Some(limit) = concurrency_limit {
        Connections::with_global_limit(limit)
      } else {
        Connections::new()
      })
      .clone();
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, |config| {
          let proxy_to_raw = get_entry!("auth_to", config).and_then(|e| {
            e.values
              .first()
              .and_then(|v| v.as_str().map(|s| s.to_owned()))
              .map(|v| {
                (
                  v,
                  e.props.get("unix").and_then(|v| v.as_str()).map(|s| s.to_owned()),
                  e.props.get("limit").and_then(|v| v.as_i128()).map(|v| v as usize),
                  e.props
                    .get("idle_timeout")
                    .map_or(Some(DEFAULT_KEEPALIVE_IDLE_TIMEOUT), |v| {
                      if v.is_null() {
                        None
                      } else {
                        Some(v.as_i128().map(|v| v as u64).unwrap_or(DEFAULT_KEEPALIVE_IDLE_TIMEOUT))
                      }
                    })
                    .map(Duration::from_millis),
                )
              })
          });
          let mut proxy_builder = connections.get_builder();
          if let Some((proxy_to, proxy_unix, keepalive_limit, keepalive_idle_timeout)) = proxy_to_raw {
            proxy_builder = proxy_builder.upstream(proxy_to, proxy_unix, keepalive_limit, keepalive_idle_timeout);
          }
          let proxy = proxy_builder.build();

          Ok(Arc::new(ForwardedAuthenticationModule { proxy }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["auth_to"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("auth_to", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `auth_to` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid forwarded authentication backend server"))?
        }
        if let Some(prop) = entry.props.get("limit") {
          if !prop.is_null() && prop.as_i128().unwrap_or(0) < 1 {
            Err(anyhow::anyhow!(
              "Invalid forwarded authentication connection limit for a backend server"
            ))?
          }
        }
        if let Some(prop) = entry.props.get("idle_timeout") {
          if !prop.is_null() && prop.as_i128().unwrap_or(0) < 1 {
            Err(anyhow::anyhow!(
              "Invalid forwarded authentication idle keep-alive connection timeout for a backend server"
            ))?
          }
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("auth_to_no_verification", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `auth_to_no_verification` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!(
            "Invalid authentication backend server certificate verification option"
          ))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("auth_to_copy", config, used_properties) {
      for entry in &entries.inner {
        for value in &entry.values {
          if !value.is_string() {
            Err(anyhow::anyhow!(
              "Invalid request headers to copy to the authentication server request configuration"
            ))?
          }
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("auth_to_concurrent_conns", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          return Err(
            anyhow::anyhow!("The `auth_to_concurrent_conns` configuration property must have exactly one value").into(),
          );
        } else if (!entry.values[0].is_integer() && !entry.values[0].is_null())
          || entry.values[0].as_i128().is_some_and(|v| v < 0)
        {
          return Err(
            anyhow::anyhow!("Invalid global maximum concurrent connections for forwarded authentication configuration")
              .into(),
          );
        }
      }
    }

    Ok(())
  }
}

/// A forwarded authentication module
struct ForwardedAuthenticationModule {
  proxy: ReverseProxy,
}

impl Module for ForwardedAuthenticationModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(ForwardedAuthenticationModuleHandlers {
      inner: self.proxy.get_handler(),
    })
  }
}

/// Forwarded authentication module handlers
struct ForwardedAuthenticationModuleHandlers {
  inner: ReverseProxyHandler,
}

#[async_trait(?Send)]
impl ModuleHandlers for ForwardedAuthenticationModuleHandlers {
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    let forwarded_auth_copy_headers = get_values!("auth_to_copy", config)
      .into_iter()
      .filter_map(|v| v.as_str().map(|v| v.to_string()))
      .collect::<Vec<_>>();

    let (request_parts, request_body) = request.into_parts();

    let request_path = request_parts.uri.path();
    let path_and_query = format!(
      "{}{}",
      request_path,
      match request_parts.uri.query() {
        Some(query) => format!("?{query}"),
        None => "".to_string(),
      }
    );

    let mut auth_request_parts = request_parts.clone();

    // No HTTP upgrades at all...
    auth_request_parts
      .headers
      .insert(header::CONNECTION, "keep-alive".parse()?);
    while auth_request_parts.headers.remove(header::UPGRADE).is_some() {}

    auth_request_parts
      .headers
      .insert(HeaderName::from_static("x-forwarded-uri"), path_and_query.parse()?);
    auth_request_parts.headers.insert(
      HeaderName::from_static("x-forwarded-method"),
      request_parts.method.as_str().parse()?,
    );

    let auth_request = Request::from_parts(auth_request_parts, Empty::new().map_err(|e| match e {}).boxed());
    let mut original_request = Request::from_parts(request_parts, request_body);

    let auth_response = self
      .inner
      .request_handler(auth_request, config, socket_data, error_logger)
      .await?;

    if let Some(proxy_response) = auth_response.response {
      let response = if proxy_response.status().is_success() {
        if !forwarded_auth_copy_headers.is_empty() {
          let response_headers = proxy_response.headers();
          let request_headers = original_request.headers_mut();
          for forwarded_auth_copy_header_string in forwarded_auth_copy_headers.iter() {
            let forwarded_auth_copy_header = HeaderName::from_str(forwarded_auth_copy_header_string)?;
            if response_headers.contains_key(&forwarded_auth_copy_header) {
              while request_headers.remove(&forwarded_auth_copy_header).is_some() {}
              for header_value in response_headers.get_all(&forwarded_auth_copy_header).iter() {
                request_headers.append(&forwarded_auth_copy_header, header_value.clone());
              }
            }
          }
        }
        ResponseData {
          request: Some(original_request),
          response: None,
          response_status: None,
          response_headers: None,
          new_remote_address: None,
        }
      } else {
        ResponseData {
          request: None,
          response: Some(proxy_response),
          response_status: None,
          response_headers: None,
          new_remote_address: None,
        }
      };
      Ok(response)
    } else {
      Ok(auth_response)
    }
  }
}
