use hashlink::LinkedHashMap;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct ServerToProcessPoolMessage {
  pub application_id: Option<usize>,
  pub environment_variables: Option<LinkedHashMap<String, String>>,
  #[serde(with = "serde_bytes")]
  pub body_chunk: Option<Vec<u8>>,
  pub body_error_message: Option<String>,
  pub requests_body_chunk: bool,
}

#[derive(Serialize, Deserialize)]
pub struct ProcessPoolToServerMessage {
  pub application_id: Option<usize>,
  pub status_code: Option<u16>,
  pub headers: Option<LinkedHashMap<String, Vec<String>>>,
  #[serde(with = "serde_bytes")]
  pub body_chunk: Option<Vec<u8>>,
  pub error_log_line: Option<String>,
  pub error_message: Option<String>,
  pub requests_body_chunk: bool,
}
