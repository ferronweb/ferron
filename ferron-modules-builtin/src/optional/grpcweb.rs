use std::collections::HashSet;
use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use async_trait::async_trait;
use bytes::Bytes;
use ferron_common::get_entries_for_validation;
use futures_util::FutureExt;
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use hyper::{Request, Response};
use tonic_web::{GrpcWebLayer, ResponseFuture};
use tower_layer::Layer;
use tower_service::Service;

use ferron_common::logging::ErrorLogger;
use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, ResponseData, SocketData};
use ferron_common::{config::ServerConfiguration, util::ModuleCache};

/// A Tower service that errors out with an original request
struct ReturnRequestService {
  tx: Option<tokio::sync::oneshot::Sender<Request<tonic::body::Body>>>,
  rx: Option<tokio::sync::oneshot::Receiver<Response<BoxBody<Bytes, std::io::Error>>>>,
}

impl Service<Request<tonic::body::Body>> for ReturnRequestService {
  type Response = Response<BoxBody<Bytes, std::io::Error>>;
  type Error = anyhow::Error;
  type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + Sync>>;

  fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), Self::Error>> {
    std::task::Poll::Ready(Ok(()))
  }

  fn call(&mut self, request: Request<tonic::body::Body>) -> Self::Future {
    if let Some(tx) = self.tx.take() {
      let _ = tx.send(request);
    }
    let rx = self.rx.take();
    Box::pin(async move {
      if let Some(rx) = rx {
        rx.await.map_err(|_| anyhow::anyhow!("Response body missing"))
      } else {
        Err(anyhow::anyhow!("Response body missing"))
      }
    })
  }
}

/// Unsafely sync Tonic body
struct SyncTonicBody {
  body: Pin<Box<tonic::body::Body>>,
}

impl SyncTonicBody {
  /// Create a new `SyncTonicBody` from a `tonic::body::Body`.
  ///
  /// ## Safety
  /// This function is unsafe because it does not check if the inner body is `Sync`.
  unsafe fn new(body: tonic::body::Body) -> Self {
    Self { body: Box::pin(body) }
  }
}

impl hyper::body::Body for SyncTonicBody {
  type Data = <tonic::body::Body as hyper::body::Body>::Data;
  type Error = <tonic::body::Body as hyper::body::Body>::Error;

  fn poll_frame(
    mut self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
    Pin::new(&mut self.body).poll_frame(cx)
  }

  fn is_end_stream(&self) -> bool {
    self.body.is_end_stream()
  }

  fn size_hint(&self) -> hyper::body::SizeHint {
    // Use default size hint, to avoid conflicts with gRPC
    hyper::body::SizeHint::default()
  }
}

// Safety: There's a safety warning in the `SyncTonicBody` struct's constructor.
unsafe impl Sync for SyncTonicBody {}

/// A gRPC-Web module loader
pub struct GrpcWebModuleLoader {
  cache: ModuleCache<GrpcWebModule>,
}

impl Default for GrpcWebModuleLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl GrpcWebModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec![]),
    }
  }
}

impl ModuleLoader for GrpcWebModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    _global_config: Option<&ServerConfiguration>,
    _secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, move |_| {
          Ok(Arc::new(GrpcWebModule {
            layer: GrpcWebLayer::new(),
          }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["grpcweb"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("grpcweb", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          return Err(anyhow::anyhow!("The `grpcweb` configuration property must have exactly one value").into());
        } else if !entry.values[0].is_bool() {
          return Err(anyhow::anyhow!("Invalid gRPC-Web translation enabling option").into());
        }
      }
    }

    Ok(())
  }
}

/// A gRPC-Web module
struct GrpcWebModule {
  layer: GrpcWebLayer,
}

impl Module for GrpcWebModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(GrpcWebModuleHandlers {
      layer: self.layer.clone(),
      service: None,
    })
  }
}

/// Handlers for the gRPC-Web module
#[allow(clippy::type_complexity)]
struct GrpcWebModuleHandlers {
  layer: GrpcWebLayer,
  service: Option<(
    tokio::sync::oneshot::Sender<Response<BoxBody<Bytes, std::io::Error>>>,
    ResponseFuture<
      Pin<Box<dyn Future<Output = Result<Response<BoxBody<Bytes, std::io::Error>>, anyhow::Error>> + Send + Sync>>,
    >,
  )>,
}

#[async_trait(?Send)]
impl ModuleHandlers for GrpcWebModuleHandlers {
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    _config: &ServerConfiguration,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let (request_tx, request_rx) = tokio::sync::oneshot::channel();
    let mut service = self.layer.layer(ReturnRequestService {
      tx: Some(request_tx),
      rx: Some(rx),
    });
    futures_util::future::poll_fn(|cx| {
      tower_service::Service::<Request<BoxBody<Bytes, std::io::Error>>>::poll_ready(&mut service, cx)
    })
    .await?;
    let mut call_future = service.call(request);
    match call_future.poll_unpin(&mut Context::from_waker(Waker::noop())) {
      Poll::Ready(response) => {
        let (response_parts, _) = response?.into_parts();
        Ok(ResponseData {
          request: None,
          response: None,
          response_status: Some(response_parts.status),
          response_headers: Some(response_parts.headers),
          new_remote_address: None,
        })
      }
      Poll::Pending => {
        let request = request_rx.await?;
        let mut request = request.map(|b| {
          // Safety: the tonic::body::Body (which SyncTonicBody wraps) is wrapped around a BoxedBody,
          // which is Send + Sync.
          let wrapped_body = unsafe { SyncTonicBody::new(b) };
          wrapped_body
            .map_err(|e| std::io::Error::other(format!("gRPC error: {e}")))
            .boxed()
        });
        self.service = Some((tx, call_future));

        // Remove the Content-Length header to avoid conflicts with gRPC
        while request.headers_mut().remove(hyper::header::CONTENT_LENGTH).is_some() {}

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

  async fn response_modifying_handler(
    &mut self,
    response: Response<BoxBody<Bytes, std::io::Error>>,
  ) -> Result<Response<BoxBody<Bytes, std::io::Error>>, Box<dyn Error>> {
    if response
      .headers()
      .get(hyper::header::CONTENT_TYPE)
      .and_then(|value| value.to_str().ok())
      .is_none_or(|value| !(value == "application/grpc" || value.starts_with("application/grpc+")))
    {
      // If the response is not gRPC, don't turn it into gRPC-Web one
      return Ok(response);
    }

    if let Some((tx, call_future)) = self.service.take() {
      tx.send(response).unwrap_or_default();
      let response = call_future.await?;
      Ok(response.map(|b| {
        // Safety: the tonic::body::Body (which SyncTonicBody wraps) is wrapped around a BoxedBody,
        // which is Send + Sync.
        let wrapped_body = unsafe { SyncTonicBody::new(b) };
        wrapped_body
          .map_err(|e| std::io::Error::other(format!("gRPC error: {e}")))
          .boxed()
      }))
    } else {
      Ok(response)
    }
  }
}
