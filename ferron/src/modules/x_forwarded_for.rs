use std::error::Error;
use std::net::{IpAddr, SocketAddr};

use crate::ferron_common::{
  ErrorLogger, HyperResponse, RequestData, ResponseData, ServerConfig, ServerModule,
  ServerModuleHandlers, SocketData,
};
use crate::ferron_common::{HyperUpgraded, WithRuntime};
use async_trait::async_trait;
use hyper::header::HeaderName;
use hyper::StatusCode;
use hyper_tungstenite::HyperWebsocket;
use tokio::runtime::Handle;

struct XForwardedForModule;

pub fn server_module_init(
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  Ok(Box::new(XForwardedForModule::new()))
}

impl XForwardedForModule {
  fn new() -> Self {
    Self
  }
}

impl ServerModule for XForwardedForModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(XForwardedForModuleHandlers { handle })
  }
}
struct XForwardedForModuleHandlers {
  handle: Handle,
}

#[async_trait]
impl ServerModuleHandlers for XForwardedForModuleHandlers {
  async fn request_handler(
    &mut self,
    request: RequestData,
    config: &ServerConfig,
    socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      if config["enableIPSpoofing"].as_bool() == Some(true) {
        let hyper_request = request.get_hyper_request();

        if let Some(x_forwarded_for_value) = hyper_request
          .headers()
          .get(HeaderName::from_static("x-forwarded-for"))
        {
          let x_forwarded_for = x_forwarded_for_value.to_str()?;

          let prepared_remote_ip_str = match x_forwarded_for.split(",").nth(0) {
            Some(ip_address_str) => ip_address_str.replace(" ", ""),
            None => {
              return Ok(
                ResponseData::builder(request)
                  .status(StatusCode::BAD_REQUEST)
                  .build(),
              );
            }
          };

          let prepared_remote_ip: IpAddr = match prepared_remote_ip_str.parse() {
            Ok(ip_address) => ip_address,
            Err(_) => {
              return Ok(
                ResponseData::builder(request)
                  .status(StatusCode::BAD_REQUEST)
                  .build(),
              );
            }
          };

          let new_socket_addr = SocketAddr::new(prepared_remote_ip, socket_data.remote_addr.port());

          return Ok(
            ResponseData::builder(request)
              .new_remote_address(new_socket_addr)
              .build(),
          );
        }

        return Ok(ResponseData::builder(request).build());
      }

      Ok(ResponseData::builder(request).build())
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
