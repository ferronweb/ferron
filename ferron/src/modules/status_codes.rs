use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::{engine::general_purpose, Engine};
use bytes::Bytes;
use fancy_regex::{Regex, RegexBuilder};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full};
use hyper::header::HeaderValue;
use hyper::{header, HeaderMap, Request, Response, StatusCode};
use password_auth::verify_password;
use tokio::sync::RwLock;

use crate::logging::ErrorLogger;
use crate::util::{get_entries, get_entries_for_validation, IpBlockList, TtlCache};
use crate::{config::ServerConfiguration, util::ModuleCache};

use super::{Module, ModuleHandlers, ModuleLoader, RequestData, ResponseData, SocketData};

/// A non-standard status code configuration
struct NonStandardCode {
  status_code: u16,
  url: Option<String>,
  regex: Option<Regex>,
  location: Option<String>,
  realm: Option<String>,
  disable_brute_force_protection: bool,
  user_list: Option<Vec<String>>,
  users: Option<IpBlockList>,
  body: Option<String>,
  not_allowed: Option<IpBlockList>,
}

/// A status codes module loader
pub struct StatusCodesModuleLoader {
  cache: ModuleCache<StatusCodesModule>,
  brute_force_db: Arc<RwLock<TtlCache<String, u8>>>,
}

impl StatusCodesModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec!["status"]),
      brute_force_db: Arc::new(RwLock::new(TtlCache::new(Duration::new(300, 0)))),
    }
  }
}

impl ModuleLoader for StatusCodesModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    _global_config: Option<&ServerConfiguration>,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, |_| {
          let mut non_standard_codes_list = Vec::new();
          if let Some(non_standard_code_config_entries) = get_entries!("status", config) {
            for non_standard_code_config_entry in &non_standard_code_config_entries.inner {
              let status_code: u16 = match non_standard_code_config_entry
                .values
                .first()
                .and_then(|v| v.as_i128())
              {
                Some(scode) => scode.try_into()?,
                None => Err(anyhow::anyhow!(
                  "Non-standard codes must include a status code"
                ))?,
              };
              let regex = match non_standard_code_config_entry
                .props
                .get("regex")
                .and_then(|v| v.as_str())
              {
                Some(regex_str) => match RegexBuilder::new(regex_str)
                  .case_insensitive(cfg!(windows))
                  .build()
                {
                  Ok(regex) => Some(regex),
                  Err(err) => Err(anyhow::anyhow!(
                    "Invalid non-standard code regular expression: {}",
                    err.to_string()
                  ))?,
                },
                None => None,
              };
              let url = non_standard_code_config_entry
                .props
                .get("url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
              if regex.is_none() && url.is_none() {
                Err(anyhow::anyhow!(
                  "Non-standard codes must either include URL or a matching regular expression"
                ))?
              }
              let location = non_standard_code_config_entry
                .props
                .get("location")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
              let realm = non_standard_code_config_entry
                .props
                .get("realm")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
              let disable_brute_force_protection = !non_standard_code_config_entry
                .props
                .get("brute_protection")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
              let user_list = non_standard_code_config_entry
                .props
                .get("users")
                .and_then(|v| v.as_str())
                .map(|userlist| {
                  userlist
                    .split(",")
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                });
              let users = match non_standard_code_config_entry
                .props
                .get("allowed")
                .and_then(|v| v.as_str())
              {
                Some(userlist) => {
                  let users_str_vec = userlist.split(",").collect::<Vec<_>>();

                  let mut users_init = IpBlockList::new();
                  users_init.load_from_vec(users_str_vec);
                  Some(users_init)
                }
                None => None,
              };
              let not_allowed = match non_standard_code_config_entry
                .props
                .get("not_allowed")
                .and_then(|v| v.as_str())
              {
                Some(userlist) => {
                  let users_str_vec = userlist.split(",").collect::<Vec<_>>();

                  let mut users_init = IpBlockList::new();
                  users_init.load_from_vec(users_str_vec);
                  Some(users_init)
                }
                None => None,
              };
              let body = non_standard_code_config_entry
                .props
                .get("body")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
              non_standard_codes_list.push(NonStandardCode {
                status_code,
                url,
                regex,
                location,
                realm,
                disable_brute_force_protection,
                user_list,
                users,
                body,
                not_allowed,
              });
            }
          }
          Ok(Arc::new(StatusCodesModule {
            non_standard_codes_list: Arc::new(non_standard_codes_list),
            brute_force_db: self.brute_force_db.clone(),
          }))
        })?,
    )
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("status", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `status` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_integer() {
          Err(anyhow::anyhow!("The custom status code must be a string"))?
        } else if !entry.props.contains_key("url") && !entry.props.contains_key("regex") {
          Err(anyhow::anyhow!(
            "Non-standard codes must either include URL or a matching regular expression"
          ))?
        } else if !entry.props.get("url").is_none_or(|v| v.is_string()) {
          Err(anyhow::anyhow!(
            "The custom status code URL must be a string"
          ))?
        } else if !entry.props.get("regex").is_none_or(|v| v.is_string()) {
          Err(anyhow::anyhow!(
            "The custom status code regular expression must be a string"
          ))?
        } else if !entry.props.get("location").is_none_or(|v| v.is_string()) {
          Err(anyhow::anyhow!(
            "The custom status code redirect destination must be a string"
          ))?
        } else if !entry.props.get("realm").is_none_or(|v| v.is_string()) {
          Err(anyhow::anyhow!(
            "The custom status code HTTP authentication realm must be a string"
          ))?
        } else if !entry
          .props
          .get("brute_protection")
          .is_none_or(|v| v.is_bool())
        {
          Err(anyhow::anyhow!(
                        "The custom status code HTTP authentication brute-force protection realm must be boolean"
                    ))?
        } else if !entry.props.get("users").is_none_or(|v| v.is_string()) {
          Err(anyhow::anyhow!(
            "The custom status code HTTP authentication allowed users list must be a string"
          ))?
        } else if !entry
          .props
          .get("brute_protection")
          .is_none_or(|v| v.is_string())
        {
          Err(anyhow::anyhow!(
            "The custom status code allowed clients list must be a string"
          ))?
        } else if !entry.props.get("body").is_none_or(|v| v.is_string()) {
          Err(anyhow::anyhow!(
            "The custom status code response body must be a string"
          ))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("user", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 2 {
          Err(anyhow::anyhow!(
            "The `user` configuration property must have exactly two values"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("Invalid username"))?
        } else if !entry.values[1].is_string() {
          Err(anyhow::anyhow!("Invalid password hash"))?
        }
      }
    }

    Ok(())
  }
}

/// A status codes module
struct StatusCodesModule {
  non_standard_codes_list: Arc<Vec<NonStandardCode>>,
  brute_force_db: Arc<RwLock<TtlCache<String, u8>>>,
}

impl Module for StatusCodesModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(StatusCodesModuleHandlers {
      non_standard_codes_list: self.non_standard_codes_list.clone(),
      brute_force_db: self.brute_force_db.clone(),
    })
  }
}

// Parses the HTTP "WWW-Authenticate" header for HTTP Basic authentication
fn parse_basic_auth(auth_str: &str) -> Option<(String, String)> {
  if let Some(base64_credentials) = auth_str.strip_prefix("Basic ") {
    if let Ok(decoded) = general_purpose::STANDARD.decode(base64_credentials) {
      if let Ok(decoded_str) = std::str::from_utf8(&decoded) {
        let parts: Vec<&str> = decoded_str.splitn(2, ':').collect();
        if parts.len() == 2 {
          return Some((parts[0].to_string(), parts[1].to_string()));
        }
      }
    }
  }
  None
}

/// Handlers for the status codes module
struct StatusCodesModuleHandlers {
  non_standard_codes_list: Arc<Vec<NonStandardCode>>,
  brute_force_db: Arc<RwLock<TtlCache<String, u8>>>,
}

#[async_trait(?Send)]
impl ModuleHandlers for StatusCodesModuleHandlers {
  async fn request_handler(
    &mut self,
    mut request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    let request_url = format!(
      "{}{}",
      request.uri().path(),
      match request.uri().query() {
        Some(query) => format!("?{query}"),
        None => String::from(""),
      }
    );

    let mut auth_user = None;

    for non_standard_code in self.non_standard_codes_list.iter() {
      let mut redirect_url = None;
      let mut url_matched = false;

      let not_applicable = non_standard_code
        .users
        .as_ref()
        .is_some_and(|allowed| allowed.is_blocked(socket_data.remote_addr.ip()))
        || !non_standard_code
          .not_allowed
          .as_ref()
          .is_none_or(|not_allowed| not_allowed.is_blocked(socket_data.remote_addr.ip()));
      if not_applicable {
        // Don't process this non-standard code if not applicable
        continue;
      }

      if let Some(regex) = &non_standard_code.regex {
        let regex_match_option = regex.find(&request_url)?;
        if let Some(regex_match) = regex_match_option {
          url_matched = true;
          if non_standard_code.status_code == 301
            || non_standard_code.status_code == 302
            || non_standard_code.status_code == 307
            || non_standard_code.status_code == 308
          {
            let matched_text = regex_match.as_str();
            if let Some(location) = &non_standard_code.location {
              redirect_url = Some(regex.replace(matched_text, location).to_string());
            }
          }
        }
      }

      if !url_matched {
        if let Some(url) = &non_standard_code.url {
          if url == request.uri().path() {
            url_matched = true;
            if non_standard_code.status_code == 301
              || non_standard_code.status_code == 302
              || non_standard_code.status_code == 307
              || non_standard_code.status_code == 308
            {
              if let Some(location) = &non_standard_code.location {
                redirect_url = Some(format!(
                  "{}{}",
                  location,
                  match request.uri().query() {
                    Some(query) => format!("?{query}"),
                    None => String::from(""),
                  }
                ));
              }
            }
          }
        }
      }

      if url_matched {
        match non_standard_code.status_code {
          301 | 302 | 307 | 308 => {
            return Ok(ResponseData {
              request: Some(request),
              response: Some(
                Response::builder()
                  .status(StatusCode::from_u16(non_standard_code.status_code)?)
                  .header(header::LOCATION, redirect_url.unwrap_or(request_url))
                  .body(if let Some(body) = &non_standard_code.body {
                    Full::new(Bytes::from(body.clone()))
                      .map_err(|e| match e {})
                      .boxed()
                  } else {
                    Empty::new().map_err(|e| match e {}).boxed()
                  })?,
              ),
              response_status: None,
              response_headers: None,
              new_remote_address: None,
            });
          }
          401 => {
            let brute_force_db_key = socket_data.remote_addr.ip().to_string();
            if !non_standard_code.disable_brute_force_protection {
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
              header::WWW_AUTHENTICATE,
              HeaderValue::from_str(&format!(
                "Basic realm=\"{}\", charset=\"UTF-8\"",
                non_standard_code
                  .realm
                  .clone()
                  .unwrap_or("Ferron HTTP Basic Authorization".to_string())
                  .replace("\\", "\\\\")
                  .replace("\"", "\\\"")
              ))?,
            );

            if let Some(authorization_header_value) = request.headers().get(header::AUTHORIZATION) {
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
                      if let Some(user_list) = &non_standard_code.user_list {
                        if !user_list.contains(&username) {
                          continue;
                        }
                      }
                      if let Some(password_hash_db) =
                        user_config.values.get(1).and_then(|v| v.as_str())
                      {
                        let password_cloned = password.clone();
                        let password_hash_db_cloned = password_hash_db.to_string();
                        // Offload verifying the hash into a separate blocking thread.
                        let password_valid = crate::runtime::spawn_blocking(move || {
                          verify_password(password_cloned, &password_hash_db_cloned).is_ok()
                        })
                        .await
                        .map_err(|_| {
                          anyhow::anyhow!("Can't spawn a blocking task to verify the password")
                        })?;
                        if password_valid {
                          authorized_user = Some(&username);
                          break;
                        }
                      }
                    }
                  }
                  if let Some(authorized_user) = authorized_user {
                    auth_user = Some(authorized_user.to_owned());
                    continue;
                  }
                }

                if !non_standard_code.disable_brute_force_protection {
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

            if let Some(body) = &non_standard_code.body {
              let mut response_builder = Response::builder().status(StatusCode::UNAUTHORIZED);
              if let Some(headers) = response_builder.headers_mut() {
                *headers = header_map;
              }
              let response = response_builder.body(
                Full::new(Bytes::from(body.clone()))
                  .map_err(|e| match e {})
                  .boxed(),
              )?;
              return Ok(ResponseData {
                request: Some(request),
                response: Some(response),
                response_status: None,
                response_headers: None,
                new_remote_address: None,
              });
            } else {
              return Ok(ResponseData {
                request: Some(request),
                response: None,
                response_status: Some(StatusCode::UNAUTHORIZED),
                response_headers: Some(header_map),
                new_remote_address: None,
              });
            }
          }
          _ => {
            let status_code = StatusCode::from_u16(non_standard_code.status_code)?;
            if let Some(body) = &non_standard_code.body {
              let response = Response::builder().status(status_code).body(
                Full::new(Bytes::from(body.clone()))
                  .map_err(|e| match e {})
                  .boxed(),
              )?;
              return Ok(ResponseData {
                request: Some(request),
                response: Some(response),
                response_status: None,
                response_headers: None,
                new_remote_address: None,
              });
            } else {
              return Ok(ResponseData {
                request: Some(request),
                response: None,
                response_status: Some(status_code),
                response_headers: None,
                new_remote_address: None,
              });
            }
          }
        }
      }
    }

    if auth_user.is_some() {
      let request_data = request.extensions_mut().get_mut::<RequestData>();
      if let Some(request_data) = request_data {
        request_data.auth_user = auth_user;
      }
    }

    Ok(ResponseData {
      request: Some(request),
      response: None,
      response_status: None,
      response_headers: None,
      new_remote_address: None,
    })
  }
}
