use ferron_common::modules::SocketData;

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
        match placeholder_value {
          "path" => output.push_str(request_parts.uri.path()),
          "path_and_query" => output.push_str(
            request_parts
              .uri
              .path_and_query()
              .map_or(request_parts.uri.path(), |p| p.as_str()),
          ),
          "method" => output.push_str(request_parts.method.as_str()),
          "version" => output.push_str(match request_parts.version {
            hyper::Version::HTTP_09 => "HTTP/0.9",
            hyper::Version::HTTP_10 => "HTTP/1.0",
            hyper::Version::HTTP_11 => "HTTP/1.1",
            hyper::Version::HTTP_2 => "HTTP/2.0",
            hyper::Version::HTTP_3 => "HTTP/3.0",
            _ => "HTTP/Unknown",
          }),
          "scheme" => output.push_str(if socket_data.encrypted { "https" } else { "http" }),
          "client_ip" => output.push_str(&socket_data.remote_addr.ip().to_string()),
          "client_port" => output.push_str(&socket_data.remote_addr.port().to_string()),
          "server_ip" => output.push_str(&socket_data.local_addr.ip().to_string()),
          "server_port" => output.push_str(&socket_data.local_addr.port().to_string()),
          "auth_user" => output.push_str(auth_user.unwrap_or("-")),
          "timestamp" => output.push_str(timestamp_str),
          "status_code" => output.push_str(&status_code.to_string()),
          "content_length" => output.push_str(&content_length.map_or("-".to_string(), |len| len.to_string())),
          _ => {
            if let Some(header_name) = placeholder_value.strip_prefix("header:") {
              if let Some(header_value) = request_parts.headers.get(header_name) {
                output.push_str(header_value.to_str().unwrap_or(""));
              } else {
                // Header not found, leave "-"
                output.push('-');
              }
            } else {
              // Unknown placeholder, leave it as is
              output.push('{');
              output.push_str(placeholder_value);
              output.push('}');
            }
          }
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

#[cfg(test)]
mod tests {
  use super::*;
  use hyper::header::HeaderName;
  use hyper::http::{request::Parts, Method, Version};
  use hyper::Request;

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
}
