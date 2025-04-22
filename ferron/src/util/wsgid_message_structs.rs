use hashlink::LinkedHashMap;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct ServerToProcessPoolMessage {
  pub environment_variables: Option<LinkedHashMap<String, String>>,
  #[serde(with = "serde_bytes")]
  pub body_chunk: Option<Vec<u8>>,
  pub requests_body_chunk: bool,
}

#[derive(Serialize, Deserialize)]
pub struct ProcessPoolToServerMessage {
  pub status_code: Option<u16>,
  pub headers: Option<LinkedHashMap<String, Vec<String>>>,
  #[serde(with = "serde_bytes")]
  pub body_chunk: Option<Vec<u8>>,
  pub error_log_line: Option<String>,
  pub error_message: Option<String>,
  pub requests_body_chunk: bool,
}
