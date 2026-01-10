use std::collections::HashSet;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hyper::header::HeaderValue;
use hyper::{header, HeaderMap, Request, StatusCode};
use password_auth::verify_password;
use tokio::sync::RwLock;

use ferron_common::logging::ErrorLogger;
use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, RequestData, ResponseData, SocketData};
use ferron_common::util::TtlCache;
use ferron_common::{config::ServerConfiguration, util::ModuleCache};
use ferron_common::{get_entries, get_entries_for_validation, get_entry};

use crate::util::parse_basic_auth;

/// A forward proxy authentication module loader
pub struct ForwardProxyAuthenticationModuleLoader {
  cache: ModuleCache<ForwardProxyAuthenticationModule>,
  brute_force_db: Arc<RwLock<TtlCache<String, u8>>>,
}

impl Default for ForwardProxyAuthenticationModuleLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl ForwardProxyAuthenticationModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec!["forward_proxy_auth"]),
      brute_force_db: Arc::new(RwLock::new(TtlCache::new(Duration::new(300, 0)))),
    }
  }
}

impl ModuleLoader for ForwardProxyAuthenticationModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    _global_config: Option<&ServerConfiguration>,
    _secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, |config| {
          let forward_proxy_auth = get_entry!("forward_proxy_auth", config);
          Ok(Arc::new(ForwardProxyAuthenticationModule {
            allowed_users: forward_proxy_auth
              .and_then(|e| e.props.get("users"))
              .and_then(|v| v.as_str())
              .map(|v| Arc::new(v.split(',').map(ToString::to_string).collect::<Vec<_>>())),
            brute_force_protection: forward_proxy_auth
              .and_then(|e| e.props.get("brute_protection"))
              .and_then(|v| v.as_bool())
              .unwrap_or(true),
            realm: forward_proxy_auth
              .and_then(|e| e.props.get("realm"))
              .and_then(|v| v.as_str())
              .map(|v| Arc::new(v.to_string())),
            brute_force_db: self.brute_force_db.clone(),
          }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["forward_proxy_auth"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("forward_proxy_auth", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          return Err(
            anyhow::anyhow!("The `forward_proxy_auth` configuration property must have exactly one value").into(),
          );
        } else if !entry.values[0].is_bool() {
          return Err(anyhow::anyhow!("Invalid forward proxy HTTP authentication enabling option").into());
        } else if !entry.props.get("users").is_none_or(|v| v.is_string()) {
          Err(anyhow::anyhow!(
            "The forward proxy HTTP authentication allowed users list must be a string"
          ))?
        } else if !entry.props.get("brute_protection").is_none_or(|v| v.is_bool()) {
          Err(anyhow::anyhow!(
            "The forward proxy HTTP authentication brute-force protection realm must be boolean"
          ))?
        }
      }
    }

    Ok(())
  }
}

/// A forward proxy authentication module
struct ForwardProxyAuthenticationModule {
  allowed_users: Option<Arc<Vec<String>>>,
  realm: Option<Arc<String>>,
  brute_force_protection: bool,
  brute_force_db: Arc<RwLock<TtlCache<String, u8>>>,
}

impl Module for ForwardProxyAuthenticationModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(ForwardProxyAuthenticationModuleHandlers {
      allowed_users: self.allowed_users.clone(),
      realm: self.realm.clone(),
      brute_force_protection: self.brute_force_protection,
      brute_force_db: self.brute_force_db.clone(),
    })
  }
}

/// Handlers for the forward proxy authentication module
struct ForwardProxyAuthenticationModuleHandlers {
  allowed_users: Option<Arc<Vec<String>>>,
  realm: Option<Arc<String>>,
  brute_force_protection: bool,
  brute_force_db: Arc<RwLock<TtlCache<String, u8>>>,
}

#[async_trait(?Send)]
impl ModuleHandlers for ForwardProxyAuthenticationModuleHandlers {
  async fn request_handler(
    &mut self,
    mut request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    // Determine if the request is a forward proxy request
    let is_proxy_request = match request.version() {
      hyper::Version::HTTP_2 | hyper::Version::HTTP_3 => {
        request.method() == hyper::Method::CONNECT && request.uri().host().is_some()
      }
      _ => request.uri().host().is_some(),
    };

    if !is_proxy_request {
      // Don't handle HTTP proxy authentication on non-forward proxy requests
      return Ok(ResponseData {
        request: Some(request),
        response: None,
        response_status: None,
        response_headers: None,
        new_remote_address: None,
      });
    }

    let brute_force_db_key = socket_data.remote_addr.ip().to_string();
    if self.brute_force_protection {
      let rwlock_read = self.brute_force_db.read().await;
      let current_attempts = rwlock_read.get(&brute_force_db_key).unwrap_or(0);
      if current_attempts >= 10 {
        error_logger
          .log(&format!(
            "Too many failed authorization attempts for client \"{}\"",
            socket_data.remote_addr.ip()
          ))
          .await;

        return Ok(ResponseData {
          request: Some(request),
          response: None,
          response_status: Some(StatusCode::TOO_MANY_REQUESTS),
          response_headers: None,
          new_remote_address: None,
        });
      }
    }
    let mut header_map = HeaderMap::new();
    header_map.insert(
      header::PROXY_AUTHENTICATE,
      HeaderValue::from_str(&format!(
        "Basic realm=\"{}\", charset=\"UTF-8\"",
        self
          .realm
          .as_deref()
          .cloned()
          .unwrap_or("Ferron HTTP Basic Authorization".to_string())
          .replace("\\", "\\\\")
          .replace("\"", "\\\"")
      ))?,
    );

    if let Some(authorization_header_value) = request.headers().get(header::PROXY_AUTHORIZATION) {
      let authorization_str = match authorization_header_value.to_str() {
        Ok(str) => str,
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

      if let Some((username, password)) = parse_basic_auth(authorization_str) {
        if let Some(users_vec_config) = get_entries!("user", config) {
          let mut authorized_user = None;
          for user_config in &users_vec_config.inner {
            if let Some(username_db) = user_config.values.first().and_then(|v| v.as_str()) {
              if username_db != username {
                continue;
              }
              if let Some(user_list) = &self.allowed_users {
                if !user_list.contains(&username) {
                  continue;
                }
              }
              if let Some(password_hash_db) = user_config.values.get(1).and_then(|v| v.as_str()) {
                let password_cloned = password.clone();
                let password_hash_db_cloned = password_hash_db.to_string();
                // Offload verifying the hash into a separate blocking thread.
                let password_valid = ferron_common::runtime::spawn_blocking(move || {
                  verify_password(password_cloned, &password_hash_db_cloned).is_ok()
                })
                .await
                .map_err(|_| anyhow::anyhow!("Can't spawn a blocking task to verify the password"))?;
                if password_valid {
                  authorized_user = Some(&username);
                  break;
                }
              }
            }
          }
          if let Some(authorized_user) = authorized_user {
            let auth_user = authorized_user.to_owned();

            let request_data = request.extensions_mut().get_mut::<RequestData>();
            if let Some(request_data) = request_data {
              request_data.auth_user = Some(auth_user);
            }

            return Ok(ResponseData {
              request: Some(request),
              response: None,
              response_status: None,
              response_headers: None,
              new_remote_address: None,
            });
          }
        }

        if self.brute_force_protection {
          let mut rwlock_write = self.brute_force_db.write().await;
          rwlock_write.cleanup();
          let current_attempts = rwlock_write.get(&brute_force_db_key).unwrap_or(0);
          rwlock_write.insert(brute_force_db_key, current_attempts + 1);
        }

        error_logger
          .log(&format!(
            "Authorization failed for user \"{}\" and client \"{}\"",
            username,
            socket_data.remote_addr.ip()
          ))
          .await;
      }
    }

    Ok(ResponseData {
      request: Some(request),
      response: None,
      response_status: Some(StatusCode::PROXY_AUTHENTICATION_REQUIRED),
      response_headers: Some(header_map),
      new_remote_address: None,
    })
  }
}
