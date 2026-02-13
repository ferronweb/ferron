use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use ferron_common::logging::ErrorLogger;
use http_body_util::combinators::BoxBody;
use hyper::header::HeaderName;
use hyper::Request;

use ferron_common::http_proxy::{Connections, LoadBalancerAlgorithm, ProxyHeader, ReverseProxy, ReverseProxyHandler};
use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, ResponseData, SocketData};
use ferron_common::observability::MetricsMultiSender;
use ferron_common::{config::ServerConfiguration, util::ModuleCache};
use ferron_common::{get_entries, get_entries_for_validation, get_value};

const DEFAULT_CONCURRENT_CONNECTIONS: usize = 16384;
const DEFAULT_KEEPALIVE_IDLE_TIMEOUT: u64 = 60000;

/// A reverse proxy module loader
pub struct ReverseProxyModuleLoader {
  cache: ModuleCache<ReverseProxyModule>,
  connections: Option<Connections>,
}

impl Default for ReverseProxyModuleLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl ReverseProxyModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec![
        "lb_algorithm",
        "lb_health_check",
        "lb_health_check_max_fails",
        "lb_health_check_window",
        "lb_retry_connection",
        "proxy",
        "proxy_concurrent_conns",
        "proxy_http2",
        "proxy_http2_only",
        "proxy_intercept_errors",
        "proxy_keepalive",
        "proxy_no_verification",
        "proxy_proxy_header",
        "proxy_request_header",
        "proxy_request_header_remove",
        "proxy_request_header_replace",
        "proxy_srv",
      ]),
      connections: None,
    }
  }
}

impl ModuleLoader for ReverseProxyModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    global_config: Option<&ServerConfiguration>,
    secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    let concurrency_limit = global_config
      .and_then(|c| get_value!("proxy_concurrent_conns", c))
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
    let connections = self.connections.get_or_insert(if let Some(limit) = concurrency_limit {
      Connections::with_global_limit(limit)
    } else {
      Connections::new()
    });
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, |config| {
          let proxy_to_raw = get_entries!("proxy", config).map_or(vec![], |e| {
            e.inner
              .iter()
              .filter_map(|e| {
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
              })
              .collect()
          });
          let proxy_to_srv_raw = get_entries!("proxy_srv", config).map_or(vec![], |e| {
            e.inner
              .iter()
              .filter_map(|e| {
                e.values
                  .first()
                  .and_then(|v| v.as_str().map(|s| s.to_owned()))
                  .map(|v| {
                    (
                      v,
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
                      e.props
                        .get("dns_servers")
                        .and_then(|v| v.as_str())
                        .map_or(vec![], |s| s.split(",").collect())
                        .into_iter()
                        .filter_map(|s| s.trim().parse::<std::net::IpAddr>().ok())
                        .collect::<Vec<_>>(),
                    )
                  })
              })
              .collect()
          });
          let mut proxy_builder = connections.get_builder();
          for (proxy_to, proxy_unix, keepalive_limit, keepalive_idle_timeout) in proxy_to_raw {
            proxy_builder = proxy_builder.upstream(proxy_to, proxy_unix, keepalive_limit, keepalive_idle_timeout);
          }
          for (to, keepalive_limit, keepalive_idle_timeout, dns_servers) in proxy_to_srv_raw {
            proxy_builder = proxy_builder.upstream_srv(
              to,
              keepalive_limit,
              keepalive_idle_timeout,
              secondary_runtime.handle().to_owned(),
              dns_servers,
            );
          }
          if let Some(custom_headers) = get_entries!("proxy_request_header", config) {
            for custom_header in custom_headers.inner.iter().rev() {
              if let Some(header_name) = custom_header.values.first().and_then(|v| v.as_str()) {
                if let Some(header_value) = custom_header.values.get(1).and_then(|v| v.as_str()) {
                  if let Ok(header_name) = HeaderName::from_str(header_name) {
                    proxy_builder = proxy_builder.proxy_request_header(header_name, header_value.to_string());
                  }
                }
              }
            }
          }
          if let Some(custom_headers) = get_entries!("proxy_request_header_replace", config) {
            for custom_header in custom_headers.inner.iter().rev() {
              if let Some(header_name) = custom_header.values.first().and_then(|v| v.as_str()) {
                if let Some(header_value) = custom_header.values.get(1).and_then(|v| v.as_str()) {
                  if let Ok(header_name) = HeaderName::from_str(header_name) {
                    proxy_builder = proxy_builder.proxy_request_header_replace(header_name, header_value.to_string());
                  }
                }
              }
            }
          }
          if let Some(custom_headers_to_remove) = get_entries!("proxy_request_header_remove", config) {
            for custom_header in custom_headers_to_remove.inner.iter().rev() {
              if let Some(header_name) = custom_header.values.first().and_then(|v| v.as_str()) {
                if let Ok(header_name) = HeaderName::from_str(header_name) {
                  proxy_builder = proxy_builder.proxy_request_header_remove(header_name);
                }
              }
            }
          }
          let proxy = proxy_builder
            .lb_algorithm({
              let algorithm_name = get_value!("lb_algorithm", config)
                .and_then(|v| v.as_str())
                .unwrap_or("two_random");
              match algorithm_name {
                "two_random" => LoadBalancerAlgorithm::TwoRandomChoices,
                "least_conn" => LoadBalancerAlgorithm::LeastConnections,
                "round_robin" => LoadBalancerAlgorithm::RoundRobin,
                "random" => LoadBalancerAlgorithm::Random,
                _ => Err(anyhow::anyhow!(
                  "Unsupported load balancing algorithm: {algorithm_name}"
                ))?,
              }
            })
            .lb_health_check(
              get_value!("lb_health_check", config)
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            )
            .lb_health_check_window(Duration::from_millis(
              get_value!("lb_health_check_window", config)
                .and_then(|v| v.as_i128())
                .unwrap_or(5000) as u64,
            ))
            .lb_health_check_max_fails(
              get_value!("lb_health_check_max_fails", config)
                .and_then(|v| v.as_i128())
                .unwrap_or(3) as u64,
            )
            .lb_retry_connection(
              get_value!("lb_retry_connection", config)
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            )
            .proxy_http2(
              get_value!("proxy_http2", config)
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            )
            .proxy_http2_only(
              get_value!("proxy_http2_only", config)
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            )
            .proxy_intercept_errors(
              get_value!("proxy_intercept_errors", config)
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            )
            .proxy_keepalive(
              get_value!("proxy_keepalive", config)
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            )
            .proxy_no_verification(
              get_value!("proxy_no_verification", config)
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            )
            .proxy_proxy_header(
              get_value!("proxy_proxy_header", config)
                .and_then(|v| v.as_str())
                .and_then(|v| match v {
                  "v1" => Some(ProxyHeader::V1),
                  "v2" => Some(ProxyHeader::V2),
                  _ => None,
                }),
            )
            .build();

          Ok(Arc::new(ReverseProxyModule { proxy }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["proxy", "proxy_srv"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("lb_health_check", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `lb_health_check` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid load balancer health check enabling option"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("lb_health_check_max_fails", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `lb_health_check_max_fails` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_integer() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid load balancer health check maximum failures"))?
        } else if let Some(value) = entry.values[0].as_i128() {
          if value < 0 {
            Err(anyhow::anyhow!("Invalid load balancer health check maximum failures"))?
          }
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("lb_health_check_window", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `lb_health_check_window` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_integer() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid load balancer health check window"))?
        } else if let Some(value) = entry.values[0].as_i128() {
          if value < 0 {
            Err(anyhow::anyhow!("Invalid load balancer health check window"))?
          }
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("proxy", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `proxy` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid proxy backend server"))?
        } else if !entry.props.get("unix").is_none_or(|v| v.is_string()) {
          Err(anyhow::anyhow!("Invalid proxy Unix socket path"))?
        }
        if let Some(prop) = entry.props.get("limit") {
          if !prop.is_null() && prop.as_i128().unwrap_or(0) < 1 {
            Err(anyhow::anyhow!("Invalid proxy connection limit for a backend server"))?
          }
        }
        if let Some(prop) = entry.props.get("idle_timeout") {
          if !prop.is_null() && prop.as_i128().unwrap_or(0) < 1 {
            Err(anyhow::anyhow!(
              "Invalid proxy idle keep-alive connection timeout for a backend server"
            ))?
          }
        }

        #[cfg(not(unix))]
        if entry.props.get("unix").is_some() {
          Err(anyhow::anyhow!("Unix sockets are not supported on this platform"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("proxy_srv", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `proxy_srv` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid proxy dynamic SRV backend server"))?
        }
        if let Some(prop) = entry.props.get("limit") {
          if !prop.is_null() && prop.as_i128().unwrap_or(0) < 1 {
            Err(anyhow::anyhow!("Invalid proxy connection limit for a backend server"))?
          }
        }
        if let Some(prop) = entry.props.get("idle_timeout") {
          if !prop.is_null() && prop.as_i128().unwrap_or(0) < 1 {
            Err(anyhow::anyhow!(
              "Invalid proxy idle keep-alive connection timeout for a backend server"
            ))?
          }
        }
        if let Some(prop) = entry.props.get("dns_servers") {
          if !prop.is_null()
            && (prop
              .as_str()
              .is_none_or(|p| p.split(",").any(|s| s.trim().parse::<std::net::IpAddr>().is_err())))
          {
            Err(anyhow::anyhow!("Invalid proxy dynamic SRV backend server DNS servers"))?
          }
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("proxy_intercept_errors", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `proxy_intercept_errors` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid proxy error interception enabling option"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("proxy_no_verification", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `proxy_no_verification` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!(
            "Invalid proxy backend server certificate verification option"
          ))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("proxy_request_header", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 2 {
          Err(anyhow::anyhow!(
            "The `proxy_request_header` configuration property must have exactly two values"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("The header name must be a string"))?
        } else if !entry.values[1].is_string() {
          Err(anyhow::anyhow!("The header value must be a string"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("proxy_request_header_remove", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `proxy_request_header_remove` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("The header name must be a string"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("proxy_keepalive", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `proxy_keepalive` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid reverse proxy HTTP keep-alive enabling option"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("proxy_request_header_replace", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 2 {
          Err(anyhow::anyhow!(
            "The `proxy_request_header_replace` configuration property must have exactly two values"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("The header name must be a string"))?
        } else if !entry.values[1].is_string() {
          Err(anyhow::anyhow!("The header value must be a string"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("proxy_http2", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `proxy_http2` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid reverse proxy HTTP/2 enabling option"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("lb_retry_connection", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `lb_retry_connection` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!(
            "Invalid load balancer retry connection enabling option"
          ))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("lb_algorithm", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `lb_algorithm` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid load balancer algorithm"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("proxy_http2_only", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `proxy_http2_only` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid reverse proxy HTTP/2 only enabling option"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("proxy_proxy_header", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `proxy_proxy_header` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid PROXY header version"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("proxy_concurrent_conns", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          return Err(
            anyhow::anyhow!("The `proxy_concurrent_conns` configuration property must have exactly one value").into(),
          );
        } else if (!entry.values[0].is_integer() && !entry.values[0].is_null())
          || entry.values[0].as_i128().is_some_and(|v| v < 0)
        {
          return Err(
            anyhow::anyhow!("Invalid global maximum concurrent connections for reverse proxy configuration").into(),
          );
        }
      }
    }

    Ok(())
  }
}

/// A reverse proxy module
struct ReverseProxyModule {
  proxy: ReverseProxy,
}

impl Module for ReverseProxyModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(ReverseProxyModuleHandlers {
      inner: self.proxy.get_handler(),
    })
  }
}

/// Reverse proxy module handlers
struct ReverseProxyModuleHandlers {
  inner: ReverseProxyHandler,
}

#[async_trait(?Send)]
impl ModuleHandlers for ReverseProxyModuleHandlers {
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    self
      .inner
      .request_handler(request, config, socket_data, error_logger)
      .await
  }

  async fn metric_data_before_handler(
    &mut self,
    request: &Request<BoxBody<Bytes, std::io::Error>>,
    socket_data: &SocketData,
    metrics_sender: &MetricsMultiSender,
  ) {
    self
      .inner
      .metric_data_before_handler(request, socket_data, metrics_sender)
      .await
  }

  async fn metric_data_after_handler(&mut self, metrics_sender: &MetricsMultiSender) {
    self.inner.metric_data_after_handler(metrics_sender).await
  }
}
