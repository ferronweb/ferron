use std::error::Error;

use crate::ferron_common::{
  ErrorLogger, HyperResponse, RequestData, ResponseData, ServerConfig, ServerModule,
  ServerModuleHandlers, SocketData,
};
use crate::ferron_common::{HyperUpgraded, WithRuntime};
use async_trait::async_trait;
use http_body_util::{BodyExt, Empty};
use hyper::{header, Response, StatusCode, Uri};
use hyper_tungstenite::HyperWebsocket;
use tokio::runtime::Handle;

struct RedirectsModule;

pub fn server_module_init(
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  Ok(Box::new(RedirectsModule::new()))
}

impl RedirectsModule {
  fn new() -> Self {
    Self
  }
}

impl ServerModule for RedirectsModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(RedirectsModuleHandlers { handle })
  }
}
struct RedirectsModuleHandlers {
  handle: Handle,
}

#[async_trait]
impl ServerModuleHandlers for RedirectsModuleHandlers {
  async fn request_handler(
    &mut self,
    request: RequestData,
    config: &ServerConfig,
    socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      let hyper_request = request.get_hyper_request();

      if config["secure"].as_bool() == Some(true)
        && !socket_data.encrypted
        && config["disableNonEncryptedServer"].as_bool() != Some(true)
        && config["disableToHTTPSRedirect"].as_bool() != Some(true)
      {
        let host_header_option = hyper_request.headers().get(header::HOST);
        let host_header = match host_header_option {
          Some(header_data) => header_data.to_str()?,
          None => {
            return Ok(
              ResponseData::builder(request)
                .status(StatusCode::BAD_REQUEST)
                .build(),
            )
          }
        };

        let path_and_query_option = hyper_request.uri().path_and_query();
        let path_and_query = match path_and_query_option {
          Some(path_and_query) => path_and_query.to_string(),
          None => {
            return Ok(
              ResponseData::builder(request)
                .status(StatusCode::BAD_REQUEST)
                .build(),
            )
          }
        };

        let mut parts: Vec<&str> = host_header.split(':').collect();

        if parts.len() > 1
          && !(parts[0].starts_with('[')
            && parts
              .last()
              .map(|part| part.ends_with(']'))
              .unwrap_or(false))
        {
          parts.pop();
        }

        let host_name = parts.join(":");

        let new_uri = Uri::builder()
          .scheme("https")
          .authority(match config["sport"].as_i64() {
            None | Some(443) => host_name,
            Some(port) => format!("{}:{}", host_name, port),
          })
          .path_and_query(path_and_query)
          .build()?;

        return Ok(
          ResponseData::builder(request)
            .response(
              Response::builder()
                .status(StatusCode::MOVED_PERMANENTLY)
                .header(header::LOCATION, new_uri.to_string())
                .body(Empty::new().map_err(|e| match e {}).boxed())?,
            )
            .build(),
        );
      }

      let domain_yaml = &config["domain"];
      let domain = domain_yaml.as_str();

      if let Some(domain) = domain {
        if config["wwwredirect"].as_bool() == Some(true) {
          // Even more code rewritten from SVR.JS...
          if let Some(host_header_value) = hyper_request.headers().get(header::HOST) {
            let host_header = host_header_value.to_str()?;

            let path_and_query_option = hyper_request.uri().path_and_query();
            let path_and_query = match path_and_query_option {
              Some(path_and_query) => path_and_query.to_string(),
              None => {
                return Ok(
                  ResponseData::builder(request)
                    .status(StatusCode::BAD_REQUEST)
                    .build(),
                )
              }
            };

            let mut parts: Vec<&str> = host_header.split(':').collect();
            let mut host_port: Option<&str> = None;

            if parts.len() > 1
              && !(parts[0].starts_with('[')
                && parts
                  .last()
                  .map(|part| part.ends_with(']'))
                  .unwrap_or(false))
            {
              host_port = parts.pop();
            }

            let host_name = parts.join(":");

            if host_name == domain && !host_name.starts_with("www.") {
              let new_uri = Uri::builder()
                .scheme(match socket_data.encrypted {
                  true => "https",
                  false => "http",
                })
                .authority(match host_port {
                  Some(port) => format!("www.{}:{}", host_name, port),
                  None => host_name,
                })
                .path_and_query(path_and_query)
                .build()?;

              return Ok(
                ResponseData::builder(request)
                  .response(
                    Response::builder()
                      .status(StatusCode::MOVED_PERMANENTLY)
                      .header(header::LOCATION, new_uri.to_string())
                      .body(Empty::new().map_err(|e| match e {}).boxed())?,
                  )
                  .build(),
              );
            }
          }
        }
      }

      Ok(ResponseData::builder(request).build())
    })
    .await
  }

  async fn proxy_request_handler(
    &mut self,
    request: RequestData,
    config: &ServerConfig,
    socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    if config["secure"].as_bool() == Some(true)
      && !socket_data.encrypted
      && config["disableNonEncryptedServer"].as_bool() != Some(true)
      && config["disableToHTTPSRedirect"].as_bool() != Some(true)
    {
      return Ok(
        ResponseData::builder(request)
          .status(StatusCode::NOT_IMPLEMENTED)
          .build(),
      );
    }
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
