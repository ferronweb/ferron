use axum::{
  http::StatusCode,
  response::{IntoResponse, Response},
  routing::post,
  Json, Router,
};
use hello_world::{greeter_client::GreeterClient, HelloRequest};
use hyper_util::rt::TokioExecutor;
use tonic_web::GrpcWebClientLayer;

mod hello_world {
  tonic::include_proto!("helloworld");
}

#[derive(serde::Serialize)]
struct ErrorJson {
  message: String,
}

struct AppError(anyhow::Error);

impl IntoResponse for AppError {
  fn into_response(self) -> Response {
    (
      StatusCode::INTERNAL_SERVER_ERROR,
      Json(ErrorJson {
        message: self.0.to_string(),
      }),
    )
      .into_response()
  }
}

#[derive(serde::Deserialize)]
struct HelloRequestJson {
  name: String,
}

#[derive(serde::Serialize)]
struct HelloReplyJson {
  message: String,
}

async fn grpc_client(Json(request): Json<HelloRequestJson>) -> Result<Json<HelloReplyJson>, AppError> {
  let client = hyper_util::client::legacy::Client::builder(TokioExecutor::new()).build_http();

  let svc = tower::ServiceBuilder::new()
    .layer(GrpcWebClientLayer::new())
    .service(client);

  // The client isn't kept alive, since this is just a test proxy, and it's not as performance-critical...
  // This proxy is not intended to be used in production anyway...
  let mut client = GreeterClient::with_origin(
    svc,
    std::env::var("GRPCWEB_TEST_PROXY_BACKEND_URL")
      .unwrap_or("http://127.0.0.1:3000".to_string())
      .try_into()
      .map_err(|e| AppError(anyhow::anyhow!("gRPC-Web error: {e}")))?,
  );

  let request = tonic::Request::new(HelloRequest { name: request.name });

  let response = client
    .say_hello(request)
    .await
    .map_err(|e| AppError(anyhow::anyhow!("gRPC-Web error: {e}")))?;

  let message = response.into_inner().message;

  Ok(Json(HelloReplyJson { message }))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let app = Router::new().route("/", post(grpc_client));

  let listener = tokio::net::TcpListener::bind(
    std::env::var("GRPCWEB_TEST_PROXY_LISTEN_ADDR").unwrap_or("0.0.0.0:8080".to_string()),
  )
  .await?;
  axum::serve(listener, app).await?;

  Ok(())
}
