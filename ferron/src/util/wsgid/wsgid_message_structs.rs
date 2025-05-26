use hashlink::LinkedHashMap;
use serde::{Deserialize, Serialize};

/// A message sent from the web server to the WSGI process pool
#[derive(Serialize, Deserialize)]
pub struct ServerToProcessPoolMessage {
  /// The WSGI application ID
  pub application_id: Option<usize>,

  /// The WSGI environment variables
  pub environment_variables: Option<LinkedHashMap<String, String>>,

  /// The request body chunk
  #[serde(with = "serde_bytes")]
  pub body_chunk: Option<Vec<u8>>,

  /// The request body error message
  pub body_error_message: Option<String>,

  /// A flag determining if the message has the request body chunk
  pub requests_body_chunk: bool,
}

/// A message sent from the WSGI process pool to the web server
#[derive(Serialize, Deserialize)]
pub struct ProcessPoolToServerMessage {
  /// The WSGI application ID
  pub application_id: Option<usize>,

  /// The response status code
  pub status_code: Option<u16>,

  /// The response headers
  pub headers: Option<LinkedHashMap<String, Vec<String>>>,

  /// The response body chunk
  #[serde(with = "serde_bytes")]
  pub body_chunk: Option<Vec<u8>>,

  /// The error log line
  pub error_log_line: Option<String>,

  /// The error message
  pub error_message: Option<String>,

  /// A flag determining if the message has the response body chunk
  pub requests_body_chunk: bool,
}
