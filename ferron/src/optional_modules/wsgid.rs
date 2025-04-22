#[cfg(not(unix))]
compile_error!("This module is supported only on Unix and Unix-like systems.");

use std::error::Error;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::thread;

use crate::ferron_common::{
  ErrorLogger, HyperUpgraded, RequestData, ResponseData, ServerConfig, ServerModule,
  ServerModuleHandlers, SocketData,
};
use crate::ferron_common::{HyperResponse, WithRuntime};
use crate::ferron_util::preforked_process_pool::{
  read_ipc_message, read_ipc_message_async, write_ipc_message, write_ipc_message_async,
  PreforkedProcessPool,
};
use crate::ferron_util::wsgi_load_application::load_wsgi_application;
use crate::ferron_util::wsgid_structs::{WsgidApplicationLocationWrap, WsgidApplicationWrap};
use async_trait::async_trait;
use http_body_util::{BodyExt, Full};
use hyper::Response;
use hyper_tungstenite::HyperWebsocket;
use interprocess::unnamed_pipe::{Recver, Sender};
use tokio::runtime::Handle;

fn wsgi_pool_fn(mut tx: Sender, mut rx: Recver, wsgi_script_path: PathBuf) {
  let wsgi_application_result = load_wsgi_application(wsgi_script_path.as_path(), false);

  println!(
    "This WSGI pool is just a stub; WSGI application loading result: {:?}",
    wsgi_application_result
  );

  // TODO: this is just a placeholder implementation to test the pre-fork process pool
  loop {
    let _ = match read_ipc_message(&mut rx) {
      Ok(message) => message,
      Err(_) => break,
    };

    if write_ipc_message(
      &mut tx,
      format!("Hello, pre-fork! My PID is {}", std::process::id()).as_bytes(),
    )
    .is_err()
    {
      break;
    }
  }
}

fn init_wsgi_process_pool(
  wsgi_script_path: PathBuf,
) -> Result<PreforkedProcessPool, Box<dyn Error + Send + Sync>> {
  let available_parallelism = thread::available_parallelism()?.get();
  // Safety: The function depends on `nix::unistd::fork`, which is executed before any threads are spawned.
  // The forking function is safe to call for single-threaded applications.
  unsafe {
    PreforkedProcessPool::new(available_parallelism, move |tx, rx| {
      let wsgi_script_path_clone = wsgi_script_path.clone();
      wsgi_pool_fn(tx, rx, wsgi_script_path_clone)
    })
  }
}

pub fn server_module_init(
  config: &ServerConfig,
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  let mut global_wsgi_process_pool = None;
  let mut host_wsgi_process_pools = Vec::new();
  if let Some(wsgi_process_pool_path) = config["global"]["wsgidApplicationPath"].as_str() {
    global_wsgi_process_pool = Some(Arc::new(init_wsgi_process_pool(PathBuf::from_str(
      wsgi_process_pool_path,
    )?)?));
  }
  let global_wsgi_path = config["global"]["wsgidPath"]
    .as_str()
    .map(|s| s.to_string());

  if let Some(hosts) = config["hosts"].as_vec() {
    for host_yaml in hosts.iter() {
      let domain = host_yaml["domain"].as_str().map(String::from);
      let ip = host_yaml["ip"].as_str().map(String::from);
      let mut locations = Vec::new();
      if let Some(locations_yaml) = host_yaml["locations"].as_vec() {
        for location_yaml in locations_yaml.iter() {
          if let Some(path_str) = location_yaml["path"].as_str() {
            let path = String::from(path_str);
            if let Some(wsgi_process_pool_path) = location_yaml["wsgidApplicationPath"].as_str() {
              locations.push(WsgidApplicationLocationWrap::new(
                path,
                Arc::new(init_wsgi_process_pool(PathBuf::from_str(
                  wsgi_process_pool_path,
                )?)?),
                location_yaml["wsgidPath"].as_str().map(|s| s.to_string()),
              ));
            }
          }
        }
      }
      if let Some(wsgi_process_pool_path) = host_yaml["wsgidApplicationPath"].as_str() {
        host_wsgi_process_pools.push(WsgidApplicationWrap::new(
          domain,
          ip,
          Some(Arc::new(init_wsgi_process_pool(PathBuf::from_str(
            wsgi_process_pool_path,
          )?)?)),
          host_yaml["wsgiPath"].as_str().map(|s| s.to_string()),
          locations,
        ));
      } else if !locations.is_empty() {
        host_wsgi_process_pools.push(WsgidApplicationWrap::new(
          domain,
          ip,
          None,
          host_yaml["wsgiPath"].as_str().map(|s| s.to_string()),
          locations,
        ));
      }
    }
  }

  Ok(Box::new(WsgidModule::new(
    global_wsgi_process_pool,
    global_wsgi_path,
    Arc::new(host_wsgi_process_pools),
  )))
}

struct WsgidModule {
  global_wsgi_process_pool: Option<Arc<PreforkedProcessPool>>,
  global_wsgi_path: Option<String>,
  host_wsgi_process_pools: Arc<Vec<WsgidApplicationWrap>>,
}

impl WsgidModule {
  fn new(
    global_wsgi_process_pool: Option<Arc<PreforkedProcessPool>>,
    global_wsgi_path: Option<String>,
    host_wsgi_process_pools: Arc<Vec<WsgidApplicationWrap>>,
  ) -> Self {
    Self {
      global_wsgi_process_pool,
      global_wsgi_path,
      host_wsgi_process_pools,
    }
  }
}

impl ServerModule for WsgidModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(WsgidModuleHandlers {
      handle,
      global_wsgi_process_pool: self.global_wsgi_process_pool.clone(),
      global_wsgi_path: self.global_wsgi_path.clone(),
      host_wsgi_process_pools: self.host_wsgi_process_pools.clone(),
    })
  }
}

struct WsgidModuleHandlers {
  handle: Handle,
  global_wsgi_process_pool: Option<Arc<PreforkedProcessPool>>,
  global_wsgi_path: Option<String>,
  host_wsgi_process_pools: Arc<Vec<WsgidApplicationWrap>>,
}

#[async_trait]
impl ServerModuleHandlers for WsgidModuleHandlers {
  async fn request_handler(
    &mut self,
    request: RequestData,
    _config: &ServerConfig,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      // TODO: this is just a placeholder implementation to test the pre-fork process pool
      // WARNING: this might contain .unwrap() calls, which might cause Rust panics
      if request.get_hyper_request().uri().path() == "/hello" {
        if let Some(global_wsgi_process_pool) = &self.global_wsgi_process_pool {
          let ipc_mutex = global_wsgi_process_pool
            .obtain_process_with_init_async_ipc()
            .await
            .unwrap();
          let (tx, rx) = &mut *ipc_mutex.lock().await;
          write_ipc_message_async(tx, b"").await?;
          let received_message = read_ipc_message_async(rx).await?;
          drop((tx, rx));
          Ok(
            ResponseData::builder(request)
              .response(
                Response::builder().body(
                  Full::new(received_message.into())
                    .map_err(|e| match e {})
                    .boxed(),
                )?,
              )
              .build(),
          )
        } else {
          Ok(ResponseData::builder(request).build())
        }
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
