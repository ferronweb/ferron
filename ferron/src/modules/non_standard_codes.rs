use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use crate::ferron_util::ip_blocklist::IpBlockList;
use crate::ferron_util::obtain_config_struct_vec::ObtainConfigStructVec;
use crate::ferron_util::ttl_cache::TtlCache;

use crate::ferron_common::{
  ErrorLogger, HyperResponse, RequestData, ResponseData, ServerConfig, ServerModule,
  ServerModuleHandlers, SocketData,
};
use crate::ferron_common::{HyperUpgraded, WithRuntime};
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine};
use fancy_regex::{Regex, RegexBuilder};
use http_body_util::{BodyExt, Empty};
use hyper::header::HeaderValue;
use hyper::{header, HeaderMap, Response, StatusCode};
use hyper_tungstenite::HyperWebsocket;
use password_auth::verify_password;
use tokio::runtime::Handle;
use tokio::sync::RwLock;
use yaml_rust2::Yaml;

#[allow(dead_code)]
struct NonStandardCode {
  status_code: u16,
  url: Option<String>,
  regex: Option<Regex>,
  location: Option<String>,
  realm: Option<String>,
  disable_brute_force_protection: bool,
  user_list: Option<Vec<String>>,
  users: Option<IpBlockList>,
}

impl NonStandardCode {
  #[allow(clippy::too_many_arguments)]
  fn new(
    status_code: u16,
    url: Option<String>,
    regex: Option<Regex>,
    location: Option<String>,
    realm: Option<String>,
    disable_brute_force_protection: bool,
    user_list: Option<Vec<String>>,
    users: Option<IpBlockList>,
  ) -> Self {
    Self {
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

fn non_standard_codes_config_init(
  non_standard_codes_list: &[Yaml],
) -> Result<Vec<NonStandardCode>, anyhow::Error> {
  let non_standard_codes_list_iter = non_standard_codes_list.iter();
  let mut non_standard_codes_list_vec = Vec::new();
  for non_standard_codes_list_entry in non_standard_codes_list_iter {
    let status_code: u16 = match non_standard_codes_list_entry["scode"].as_i64() {
      Some(scode) => scode.try_into()?,
      None => {
        return Err(anyhow::anyhow!(
          "Non-standard codes must include a status code"
        ));
      }
    };
    let regex = match non_standard_codes_list_entry["regex"].as_str() {
      Some(regex_str) => match RegexBuilder::new(regex_str)
        .case_insensitive(cfg!(windows))
        .build()
      {
        Ok(regex) => Some(regex),
        Err(err) => {
          return Err(anyhow::anyhow!(
            "Invalid non-standard code regular expression: {}",
            err.to_string()
          ));
        }
      },
      None => None,
    };
    let url = non_standard_codes_list_entry["url"]
      .as_str()
      .map(|s| s.to_string());

    if regex.is_none() && url.is_none() {
      return Err(anyhow::anyhow!(
        "Non-standard codes must either include URL or a matching regular expression"
      ));
    }

    let location = non_standard_codes_list_entry["location"]
      .as_str()
      .map(|s| s.to_string());
    let realm = non_standard_codes_list_entry["realm"]
      .as_str()
      .map(|s| s.to_string());
    let disable_brute_force_protection = non_standard_codes_list_entry["disableBruteProtection"]
      .as_bool()
      .unwrap_or(false);
    let user_list = match non_standard_codes_list_entry["userList"].as_vec() {
      Some(userlist) => {
        let mut new_userlist = Vec::new();
        for user_yaml in userlist.iter() {
          if let Some(user) = user_yaml.as_str() {
            new_userlist.push(user.to_string());
          }
        }
        Some(new_userlist)
      }
      None => None,
    };
    let users = match non_standard_codes_list_entry["users"].as_vec() {
      Some(users_vec) => {
        let mut users_str_vec = Vec::new();
        for user_yaml in users_vec.iter() {
          if let Some(user) = user_yaml.as_str() {
            users_str_vec.push(user);
          }
        }

        let mut users_init = IpBlockList::new();
        users_init.load_from_vec(users_str_vec);
        Some(users_init)
      }
      None => None,
    };
    non_standard_codes_list_vec.push(NonStandardCode::new(
      status_code,
      url,
      regex,
      location,
      realm,
      disable_brute_force_protection,
      user_list,
      users,
    ));
  }

  Ok(non_standard_codes_list_vec)
}

pub fn server_module_init(
  config: &ServerConfig,
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  Ok(Box::new(NonStandardCodesModule::new(
    ObtainConfigStructVec::new(config, |config| {
      if let Some(non_standard_codes_yaml) = config["nonStandardCodes"].as_vec() {
        Ok(non_standard_codes_config_init(non_standard_codes_yaml)?)
      } else {
        Ok(vec![])
      }
    })?,
    Arc::new(RwLock::new(TtlCache::new(Duration::new(300, 0)))),
  )))
}

struct NonStandardCodesModule {
  non_standard_codes_list: ObtainConfigStructVec<NonStandardCode>,
  brute_force_db: Arc<RwLock<TtlCache<String, u8>>>,
}

impl NonStandardCodesModule {
  fn new(
    non_standard_codes_list: ObtainConfigStructVec<NonStandardCode>,
    brute_force_db: Arc<RwLock<TtlCache<String, u8>>>,
  ) -> Self {
    Self {
      non_standard_codes_list,
      brute_force_db,
    }
  }
}

impl ServerModule for NonStandardCodesModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(NonStandardCodesModuleHandlers {
      non_standard_codes_list: self.non_standard_codes_list.clone(),
      brute_force_db: self.brute_force_db.clone(),
      handle,
    })
  }
}

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

struct NonStandardCodesModuleHandlers {
  non_standard_codes_list: ObtainConfigStructVec<NonStandardCode>,
  brute_force_db: Arc<RwLock<TtlCache<String, u8>>>,
  handle: Handle,
}

#[async_trait]
impl ServerModuleHandlers for NonStandardCodesModuleHandlers {
  async fn request_handler(
    &mut self,
    request: RequestData,
    config: &ServerConfig,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      let hyper_request = request.get_hyper_request();
      let combined_non_standard_codes_list = self.non_standard_codes_list.obtain(
        match hyper_request.headers().get(header::HOST) {
          Some(value) => value.to_str().ok(),
          None => None,
        },
        socket_data.remote_addr.ip(),
        request
          .get_original_url()
          .unwrap_or(request.get_hyper_request().uri())
          .path(),
        request.get_error_status_code().map(|x| x.as_u16()),
      );

      let request_url = format!(
        "{}{}",
        hyper_request.uri().path(),
        match hyper_request.uri().query() {
          Some(query) => format!("?{query}"),
          None => String::from(""),
        }
      );

      let mut auth_user = None;

      for non_standard_code in combined_non_standard_codes_list {
        let mut redirect_url = None;
        let mut url_matched = false;

        if let Some(users) = &non_standard_code.users {
          if !users.is_blocked(socket_data.remote_addr.ip()) {
            // Don't process this non-standard code
            continue;
          }
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
            if url == hyper_request.uri().path() {
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
                    match hyper_request.uri().query() {
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
              return Ok(
                ResponseData::builder(request)
                  .response(
                    Response::builder()
                      .status(StatusCode::from_u16(non_standard_code.status_code)?)
                      .header(header::LOCATION, redirect_url.unwrap_or(request_url))
                      .body(Empty::new().map_err(|e| match e {}).boxed())?,
                  )
                  .build(),
              );
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

                  return Ok(
                    ResponseData::builder(request)
                      .status(StatusCode::TOO_MANY_REQUESTS)
                      .build(),
                  );
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

              if let Some(authorization_header_value) =
                hyper_request.headers().get(header::AUTHORIZATION)
              {
                let authorization_str = match authorization_header_value.to_str() {
                  Ok(str) => str,
                  Err(_) => {
                    return Ok(
                      ResponseData::builder(request)
                        .status(StatusCode::BAD_REQUEST)
                        .build(),
                    );
                  }
                };

                if let Some((username, password)) = parse_basic_auth(authorization_str) {
                  if let Some(users_vec_yaml) = config["users"].as_vec() {
                    let mut authorized_user = None;
                    for user_yaml in users_vec_yaml {
                      if let Some(username_db) = user_yaml["name"].as_str() {
                        if username_db != username {
                          continue;
                        }
                        if let Some(user_list) = &non_standard_code.user_list {
                          if !user_list.contains(&username) {
                            continue;
                          }
                        }
                        if let Some(password_hash_db) = user_yaml["pass"].as_str() {
                          let password_cloned = password.clone();
                          let password_hash_db_cloned = password_hash_db.to_string();
                          // Offload verifying the hash into a separate blocking thread.
                          let password_valid = tokio::task::spawn_blocking(move || {
                            verify_password(password_cloned, &password_hash_db_cloned).is_ok()
                          })
                          .await?;
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

              return Ok(
                ResponseData::builder(request)
                  .status(StatusCode::UNAUTHORIZED)
                  .headers(header_map)
                  .build(),
              );
            }
            _ => {
              return Ok(
                ResponseData::builder(request)
                  .status(StatusCode::from_u16(non_standard_code.status_code)?)
                  .build(),
              )
            }
          }
        }
      }

      if auth_user.is_some() {
        let (hyper_request, _, original_url, error_status_code) = request.into_parts();
        Ok(
          ResponseData::builder(RequestData::new(
            hyper_request,
            auth_user,
            original_url,
            error_status_code,
          ))
          .build(),
        )
      } else {
        Ok(ResponseData::builder(request).build())
      }
    })
    .await
  }

  async fn proxy_request_handler(
    &mut self,
    request: RequestData,
    _config: &ServerConfig,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    Ok(ResponseData::builder(request).build())
  }

  async fn response_modifying_handler(
    &mut self,
    response: HyperResponse,
  ) -> Result<HyperResponse, Box<dyn Error + Send + Sync>> {
    Ok(response)
  }

  async fn proxy_response_modifying_handler(
    &mut self,
    response: HyperResponse,
  ) -> Result<HyperResponse, Box<dyn Error + Send + Sync>> {
    Ok(response)
  }

  async fn connect_proxy_request_handler(
    &mut self,
    _upgraded_request: HyperUpgraded,
    _connect_address: &str,
    _config: &ServerConfig,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    Ok(())
  }

  fn does_connect_proxy_requests(&mut self) -> bool {
    false
  }

  async fn websocket_request_handler(
    &mut self,
    _websocket: HyperWebsocket,
    _uri: &hyper::Uri,
    _headers: &hyper::HeaderMap,
    _config: &ServerConfig,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    Ok(())
  }

  fn does_websocket_requests(&mut self, _config: &ServerConfig, _socket_data: &SocketData) -> bool {
    false
  }
}
