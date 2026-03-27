use std::collections::HashMap;

use ferron_common::config::ServerConfigurationValue;
use ferron_common::modules::SocketData;
use serde_json::{Map, Number, Value};

const DEFAULT_ACCESS_LOG_FORMAT: &str =
  "{client_ip} - {auth_user} [{timestamp}] \"{method} {path_and_query} {version}\" \
   {status_code} {content_length} \"{header:Referer}\" \"{header:User-Agent}\"";

fn http_version_to_str(version: hyper::Version) -> &'static str {
  match version {
    hyper::Version::HTTP_09 => "HTTP/0.9",
    hyper::Version::HTTP_10 => "HTTP/1.0",
    hyper::Version::HTTP_11 => "HTTP/1.1",
    hyper::Version::HTTP_2 => "HTTP/2.0",
    hyper::Version::HTTP_3 => "HTTP/3.0",
    _ => "HTTP/Unknown",
  }
}

fn resolve_log_placeholder(
  placeholder: &str,
  request_parts: &hyper::http::request::Parts,
  socket_data: &SocketData,
  auth_user: Option<&str>,
  timestamp_str: &str,
  status_code: u16,
  content_length: Option<u64>,
) -> Option<String> {
  Some(match placeholder {
    "path" => request_parts.uri.path().to_string(),
    "path_and_query" => request_parts
      .uri
      .path_and_query()
      .map_or_else(|| request_parts.uri.path().to_string(), |p| p.as_str().to_string()),
    "method" => request_parts.method.as_str().to_string(),
    "version" => http_version_to_str(request_parts.version).to_string(),
    "scheme" => {
      if socket_data.encrypted {
        "https".to_string()
      } else {
        "http".to_string()
      }
    }
    "client_ip" => socket_data.remote_addr.ip().to_string(),
    "client_port" => socket_data.remote_addr.port().to_string(),
    "client_ip_canonical" => socket_data.remote_addr.ip().to_canonical().to_string(),
    "server_ip" => socket_data.local_addr.ip().to_string(),
    "server_port" => socket_data.local_addr.port().to_string(),
    "server_ip_canonical" => socket_data.local_addr.ip().to_canonical().to_string(),
    "auth_user" => auth_user.unwrap_or("-").to_string(),
    "timestamp" => timestamp_str.to_string(),
    "status_code" => status_code.to_string(),
    "content_length" => content_length.map_or_else(|| "-".to_string(), |len| len.to_string()),
    _ => {
      if let Some(header_name) = placeholder.strip_prefix("header:") {
        if let Some(header_value) = request_parts.headers.get(header_name) {
          header_value.to_str().unwrap_or("").to_string()
        } else {
          "-".to_string()
        }
      } else {
        return None;
      }
    }
  })
}

pub fn replace_log_placeholders(
  input: &str,
  request_parts: &hyper::http::request::Parts,
  socket_data: &SocketData,
  auth_user: Option<&str>,
  timestamp_str: &str,
  status_code: u16,
  content_length: Option<u64>,
) -> String {
  let mut output = String::new();
  let mut index_rb_saved = 0;
  loop {
    let index_lb = input[index_rb_saved..].find("{");
    if let Some(index_lb) = index_lb {
      let index_rb_afterlb = input[index_rb_saved + index_lb + 1..].find("}");
      if let Some(index_rb_afterlb) = index_rb_afterlb {
        let index_rb = index_rb_afterlb + index_lb + 1;
        let placeholder_value = &input[index_rb_saved + index_lb + 1..index_rb_saved + index_rb];
        output.push_str(&input[index_rb_saved..index_rb_saved + index_lb]);
        if let Some(value) = resolve_log_placeholder(
          placeholder_value,
          request_parts,
          socket_data,
          auth_user,
          timestamp_str,
          status_code,
          content_length,
        ) {
          output.push_str(&value);
        } else {
          // Unknown placeholder, leave it as is
          output.push('{');
          output.push_str(placeholder_value);
          output.push('}');
        }
        if index_rb < input.len() - 1 {
          index_rb_saved += index_rb + 1;
        } else {
          break;
        }
      } else {
        output.push_str(&input[index_rb_saved..]);
      }
    } else {
      output.push_str(&input[index_rb_saved..]);
      break;
    }
  }
  output
}

#[allow(clippy::too_many_arguments)]
pub fn generate_access_log_message(
  request_parts: &hyper::http::request::Parts,
  socket_data: &SocketData,
  auth_user: Option<&str>,
  timestamp_str: &str,
  status_code: u16,
  content_length: Option<u64>,
  log_format: Option<&str>,
  log_json_props: Option<&HashMap<String, ServerConfigurationValue>>,
) -> String {
  if let Some(log_json_props) = log_json_props {
    let mut log_entry = Map::new();
    log_entry.insert(
      "auth_user".to_string(),
      auth_user.map_or(Value::Null, |user| Value::String(user.to_string())),
    );
    log_entry.insert(
      "client_ip".to_string(),
      Value::String(socket_data.remote_addr.ip().to_string()),
    );
    log_entry.insert(
      "content_length".to_string(),
      content_length.map_or(Value::Null, |len| Value::Number(Number::from(len))),
    );
    log_entry.insert(
      "method".to_string(),
      Value::String(request_parts.method.as_str().to_string()),
    );
    log_entry.insert(
      "path_and_query".to_string(),
      Value::String(
        request_parts
          .uri
          .path_and_query()
          .map_or_else(|| request_parts.uri.path().to_string(), |p| p.as_str().to_string()),
      ),
    );
    log_entry.insert("status_code".to_string(), Value::Number(Number::from(status_code)));
    log_entry.insert("timestamp".to_string(), Value::String(timestamp_str.to_string()));
    log_entry.insert(
      "version".to_string(),
      Value::String(http_version_to_str(request_parts.version).to_string()),
    );
    log_entry.insert(
      "referer".to_string(),
      request_parts
        .headers
        .get("Referer")
        .and_then(|value| value.to_str().ok())
        .map_or(Value::Null, |value| Value::String(value.to_string())),
    );
    log_entry.insert(
      "user_agent".to_string(),
      request_parts
        .headers
        .get("User-Agent")
        .and_then(|value| value.to_str().ok())
        .map_or(Value::Null, |value| Value::String(value.to_string())),
    );

    for (property_name, property_value) in log_json_props {
      if let Some(property_template) = property_value.as_str() {
        log_entry.insert(
          property_name.clone(),
          Value::String(replace_log_placeholders(
            property_template,
            request_parts,
            socket_data,
            auth_user,
            timestamp_str,
            status_code,
            content_length,
          )),
        );
      }
    }

    Value::Object(log_entry).to_string()
  } else {
    replace_log_placeholders(
      log_format.unwrap_or(DEFAULT_ACCESS_LOG_FORMAT),
      request_parts,
      socket_data,
      auth_user,
      timestamp_str,
      status_code,
      content_length,
    )
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use ferron_common::config::ServerConfigurationValue;
  use hyper::header::HeaderName;
  use hyper::http::{request::Parts, Method, Version};
  use hyper::Request;
  use serde_json::json;

  fn make_parts(uri_str: &str, method: Method, version: Version, headers: Option<Vec<(&str, &str)>>) -> Parts {
    let mut parts = Request::builder()
      .uri(uri_str)
      .method(method)
      .version(version)
      .body(())
      .unwrap()
      .into_parts()
      .0;

    if let Some(hdrs) = headers {
      for (k, v) in hdrs {
        parts
          .headers
          .insert(k.parse::<HeaderName>().unwrap(), v.parse().unwrap());
      }
    }
    parts
  }

  #[test]
  fn test_basic_placeholders() {
    let parts = make_parts("/some/path", Method::GET, Version::HTTP_11, None);
    let input = "Path: {path}, Method: {method}, Version: {version}";
    let expected = "Path: /some/path, Method: GET, Version: HTTP/1.1";
    let output = replace_log_placeholders(
      input,
      &parts,
      &SocketData {
        remote_addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 40000)),
        local_addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 80)),
        encrypted: false,
      },
      None,
      "06/Oct/2025:15:12:51 +0200",
      200,
      None,
    );
    assert_eq!(output, expected);
  }

  #[test]
  fn test_header_placeholder() {
    let parts = make_parts(
      "/test",
      Method::POST,
      Version::HTTP_2,
      Some(vec![("User-Agent", "MyApp/1.0")]),
    );
    let input = "Header: {header:User-Agent}";
    let expected = "Header: MyApp/1.0";
    let output = replace_log_placeholders(
      input,
      &parts,
      &SocketData {
        remote_addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 40000)),
        local_addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 80)),
        encrypted: false,
      },
      None,
      "06/Oct/2025:15:12:51 +0200",
      200,
      None,
    );
    assert_eq!(output, expected);
  }

  #[test]
  fn test_missing_header() {
    let parts = make_parts("/", Method::GET, Version::HTTP_11, None);
    let input = "Header: {header:Missing}";
    let expected = "Header: -";
    let output = replace_log_placeholders(
      input,
      &parts,
      &SocketData {
        remote_addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 40000)),
        local_addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 80)),
        encrypted: false,
      },
      None,
      "06/Oct/2025:15:12:51 +0200",
      200,
      None,
    );
    assert_eq!(output, expected);
  }

  #[test]
  fn test_unknown_placeholder() {
    let parts = make_parts("/", Method::GET, Version::HTTP_11, None);
    let input = "Unknown: {foo}";
    let expected = "Unknown: {foo}";
    let output = replace_log_placeholders(
      input,
      &parts,
      &SocketData {
        remote_addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 40000)),
        local_addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 80)),
        encrypted: false,
      },
      None,
      "06/Oct/2025:15:12:51 +0200",
      200,
      None,
    );
    assert_eq!(output, expected);
  }

  #[test]
  fn test_no_placeholders() {
    let parts = make_parts("/", Method::GET, Version::HTTP_11, None);
    let input = "Static string with no placeholders.";
    let output = replace_log_placeholders(
      input,
      &parts,
      &SocketData {
        remote_addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 40000)),
        local_addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 80)),
        encrypted: false,
      },
      None,
      "06/Oct/2025:15:12:51 +0200",
      200,
      None,
    );
    assert_eq!(output, input);
  }

  #[test]
  fn test_multiple_placeholders() {
    let parts = make_parts(
      "/data",
      Method::PUT,
      Version::HTTP_2,
      Some(vec![("Content-Type", "application/json"), ("Host", "api.example.com")]),
    );
    let input = "{method} {path} {version} Host: {header:Host} Content-Type: {header:Content-Type}";
    let expected = "PUT /data HTTP/2.0 Host: api.example.com Content-Type: application/json";
    let output = replace_log_placeholders(
      input,
      &parts,
      &SocketData {
        remote_addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 40000)),
        local_addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 80)),
        encrypted: false,
      },
      None,
      "06/Oct/2025:15:12:51 +0200",
      200,
      None,
    );
    assert_eq!(output, expected);
  }

  #[test]
  fn test_log_placeholders() {
    let parts = make_parts(
      "/data",
      Method::PUT,
      Version::HTTP_2,
      Some(vec![("Content-Type", "application/json"), ("Host", "api.example.com")]),
    );
    let input = "[{timestamp}] {auth_user} {status_code} {content_length}";
    let expected = "[06/Oct/2025:15:12:51 +0200] - 200 -";
    let output = replace_log_placeholders(
      input,
      &parts,
      &SocketData {
        remote_addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 40000)),
        local_addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 80)),
        encrypted: false,
      },
      None,
      "06/Oct/2025:15:12:51 +0200",
      200,
      None,
    );
    assert_eq!(output, expected);
  }

  #[test]
  fn test_generate_access_log_message_plain_text() {
    let parts = make_parts("/test?hello=world", Method::GET, Version::HTTP_11, None);
    let output = generate_access_log_message(
      &parts,
      &SocketData {
        remote_addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 40000)),
        local_addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 80)),
        encrypted: false,
      },
      None,
      "06/Oct/2025:15:12:51 +0200",
      200,
      Some(123),
      None,
      None,
    );
    assert_eq!(
      output,
      "127.0.0.1 - - [06/Oct/2025:15:12:51 +0200] \"GET /test?hello=world HTTP/1.1\" 200 123 \"-\" \"-\""
    );
  }

  #[test]
  fn test_generate_access_log_message_json_with_extra_props() {
    let parts = make_parts(
      "/api/items?id=1",
      Method::POST,
      Version::HTTP_2,
      Some(vec![
        ("Referer", "https://example.com/app"),
        ("User-Agent", "FerronTest/1.0"),
        ("X-Request-Id", "req-123"),
      ]),
    );
    let mut extra_props = HashMap::new();
    extra_props.insert(
      "request_id".to_string(),
      ServerConfigurationValue::String("{header:X-Request-Id}".to_string()),
    );
    extra_props.insert(
      "request_target".to_string(),
      ServerConfigurationValue::String("{method} {path_and_query}".to_string()),
    );

    let output = generate_access_log_message(
      &parts,
      &SocketData {
        remote_addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 40000)),
        local_addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 443)),
        encrypted: true,
      },
      Some("alice"),
      "06/Oct/2025:15:12:51 +0200",
      201,
      Some(456),
      Some("{method} {path_and_query}"),
      Some(&extra_props),
    );

    let output: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(
      output,
      json!({
        "auth_user": "alice",
        "client_ip": "127.0.0.1",
        "content_length": 456,
        "method": "POST",
        "path_and_query": "/api/items?id=1",
        "referer": "https://example.com/app",
        "request_id": "req-123",
        "request_target": "POST /api/items?id=1",
        "status_code": 201,
        "timestamp": "06/Oct/2025:15:12:51 +0200",
        "user_agent": "FerronTest/1.0",
        "version": "HTTP/2.0"
      })
    );
  }

  #[test]
  fn test_generate_access_log_message_json_can_override_default_fields() {
    let parts = make_parts("/health", Method::GET, Version::HTTP_11, None);
    let mut extra_props = HashMap::new();
    extra_props.insert(
      "status_code".to_string(),
      ServerConfigurationValue::String("ok".to_string()),
    );

    let output = generate_access_log_message(
      &parts,
      &SocketData {
        remote_addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 40000)),
        local_addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 80)),
        encrypted: false,
      },
      None,
      "06/Oct/2025:15:12:51 +0200",
      200,
      None,
      None,
      Some(&extra_props),
    );

    let output: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(output["status_code"], "ok");
    assert_eq!(output["content_length"], Value::Null);
    assert_eq!(output["auth_user"], Value::Null);
  }
}
