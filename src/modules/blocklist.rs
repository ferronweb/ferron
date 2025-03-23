use std::error::Error;
use std::sync::Arc;

use crate::ferron_common::{
  ErrorLogger, HyperResponse, RequestData, ResponseData, ServerConfig, ServerModule,
  ServerModuleHandlers, SocketData,
};
use crate::ferron_common::{HyperUpgraded, WithRuntime};
use async_trait::async_trait;
use hyper::StatusCode;
use hyper_tungstenite::HyperWebsocket;
use tokio::runtime::Handle;

use crate::ferron_util::ip_blocklist::IpBlockList;

struct BlockListModule {
  blocklist: Arc<IpBlockList>,
}

pub fn server_module_init(
  config: &ServerConfig,
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  let blocklist_vec = match config["global"]["blocklist"].as_vec() {
    Some(blocklist_vec) => blocklist_vec,
    None => &Vec::new(),
  };

  let mut blocklist_str_vec = Vec::new();
  for blocked_yaml in blocklist_vec.iter() {
    if let Some(blocked) = blocked_yaml.as_str() {
      blocklist_str_vec.push(blocked);
    }
  }

  let mut blocklist = IpBlockList::new();
  blocklist.load_from_vec(blocklist_str_vec);

  Ok(Box::new(BlockListModule::new(Arc::new(blocklist))))
}

impl BlockListModule {
  fn new(blocklist: Arc<IpBlockList>) -> Self {
    BlockListModule { blocklist }
  }
}

impl ServerModule for BlockListModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(BlockListModuleHandlers {
      blocklist: self.blocklist.clone(),
      handle,
    })
  }
}
struct BlockListModuleHandlers {
  blocklist: Arc<IpBlockList>,
  handle: Handle,
}

#[async_trait]
impl ServerModuleHandlers for BlockListModuleHandlers {
  async fn request_handler(
    &mut self,
    request: RequestData,
    _config: &ServerConfig,
    socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      if self.blocklist.is_blocked(socket_data.remote_addr.ip()) {
        return Ok(
          ResponseData::builder(request)
            .status(StatusCode::FORBIDDEN)
            .build(),
        );
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
