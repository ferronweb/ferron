use std::error::Error;
use std::str::FromStr;

use hyper::header::{self, HeaderName};
use hyper::{HeaderMap, Uri, Version};

use crate::config::ServerConfiguration;
use crate::get_value;
use crate::modules::SocketData;
use crate::util::replace_header_placeholders;

/// Constructs a proxy request based on the original request.
#[inline]
pub(super) fn construct_proxy_request_parts(
  mut request_parts: hyper::http::request::Parts,
  config: &ServerConfiguration,
  socket_data: &SocketData,
  proxy_request_url: &Uri,
  headers_to_add: &[(HeaderName, String)],
  headers_to_replace: &[(HeaderName, String)],
  headers_to_remove: &[HeaderName],
) -> Result<hyper::http::request::Parts, Box<dyn Error + Send + Sync>> {
  let headers_to_add = HeaderMap::from_iter(headers_to_add.iter().cloned().filter_map(|(name, value)| {
    replace_header_placeholders(&value, &request_parts, Some(socket_data))
      .parse()
      .ok()
      .map(|v| (name, v))
  }));
  let headers_to_replace = HeaderMap::from_iter(headers_to_replace.iter().cloned().filter_map(|(name, value)| {
    replace_header_placeholders(&value, &request_parts, Some(socket_data))
      .parse()
      .ok()
      .map(|v| (name, v))
  }));
  let headers_to_remove = headers_to_remove.to_vec();

  let authority = proxy_request_url.authority().cloned();

  let request_path = request_parts.uri.path();

  let path = match request_path.as_bytes().first() {
    Some(b'/') => {
      let mut proxy_request_path = proxy_request_url.path();
      while proxy_request_path.as_bytes().last().copied() == Some(b'/') {
        proxy_request_path = &proxy_request_path[..(proxy_request_path.len() - 1)];
      }
      format!("{proxy_request_path}{request_path}")
    }
    _ => request_path.to_string(),
  };

  request_parts.uri = Uri::from_str(&format!(
    "{}{}",
    path,
    match request_parts.uri.query() {
      Some(query) => format!("?{query}"),
      None => "".to_string(),
    }
  ))?;

  let original_host = request_parts.headers.get(header::HOST).cloned();

  match authority {
    Some(authority) => {
      request_parts
        .headers
        .insert(header::HOST, authority.to_string().parse()?);
    }
    None => {
      request_parts.headers.remove(header::HOST);
    }
  }

  if let Some(connection_header) = request_parts.headers.get(&header::CONNECTION) {
    let connection_str = String::from_utf8_lossy(connection_header.as_bytes());
    if connection_str
      .to_lowercase()
      .split(",")
      .all(|c| c != "keep-alive" && c != "upgrade" && c != "close")
    {
      request_parts
        .headers
        .insert(header::CONNECTION, format!("keep-alive, {connection_str}").parse()?);
    }
  } else {
    request_parts.headers.insert(header::CONNECTION, "keep-alive".parse()?);
  }

  let trust_x_forwarded_for = get_value!("trust_x_forwarded_for", config)
    .and_then(|v| v.as_bool())
    .unwrap_or(false);

  let remote_addr_str = socket_data.remote_addr.ip().to_canonical().to_string();
  request_parts.headers.insert(
    HeaderName::from_static("x-forwarded-for"),
    (if let Some(ref forwarded_for) = request_parts
      .headers
      .get(HeaderName::from_static("x-forwarded-for"))
      .and_then(|h| h.to_str().ok())
    {
      if trust_x_forwarded_for {
        format!("{forwarded_for}, {remote_addr_str}")
      } else {
        remote_addr_str
      }
    } else {
      remote_addr_str
    })
    .parse()?,
  );

  if !trust_x_forwarded_for
    || !request_parts
      .headers
      .contains_key(HeaderName::from_static("x-forwarded-proto"))
  {
    if socket_data.encrypted {
      request_parts
        .headers
        .insert(HeaderName::from_static("x-forwarded-proto"), "https".parse()?);
    } else {
      request_parts
        .headers
        .insert(HeaderName::from_static("x-forwarded-proto"), "http".parse()?);
    }
  }

  if !trust_x_forwarded_for
    || !request_parts
      .headers
      .contains_key(HeaderName::from_static("x-forwarded-host"))
  {
    if let Some(original_host) = original_host {
      request_parts
        .headers
        .insert(HeaderName::from_static("x-forwarded-host"), original_host);
    }
  }

  let mut forwarded_header_value = None;
  if let Some(forwarded_header_value_obtained) = request_parts
    .headers
    .get(HeaderName::from_static("x-forwarded-for"))
    .and_then(|h| h.to_str().ok())
  {
    let mut forwarded_header_value_new = Vec::new();
    let mut is_first = true;

    for ip in forwarded_header_value_obtained
      .split(',')
      .map(|s| s.trim())
      .filter(|s| !s.is_empty())
    {
      let escape_determinants: &'static [char] = &[
        '(', ')', ',', '/', ':', ';', '<', '=', '>', '?', '@', '[', '\\', ']', '{', '}', '"', '\'', '\r', '\n', '\t',
      ];

      let forwarded_for = if ip.parse::<std::net::Ipv4Addr>().is_ok() {
        ip.to_string()
      } else if ip.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("\"[{ip}]\"")
      } else if ip.contains(escape_determinants) {
        format!("\"{}\"", ip.escape_default())
      } else {
        ip.to_string()
      };

      let (forwarded_host, forwarded_proto) = if is_first {
        (
          request_parts
            .headers
            .get(HeaderName::from_static("x-forwarded-host"))
            .and_then(|h| h.to_str().ok()),
          request_parts
            .headers
            .get(HeaderName::from_static("x-forwarded-proto"))
            .and_then(|h| h.to_str().ok()),
        )
      } else {
        (None, None)
      };

      let mut forwarded_entry = Vec::new();
      forwarded_entry.push(format!("for={}", forwarded_for));
      if let Some(forwarded_proto) = forwarded_proto {
        forwarded_entry.push(format!(
          "proto={}",
          if forwarded_proto.contains(escape_determinants) {
            format!("\"{}\"", forwarded_proto.escape_default())
          } else {
            forwarded_proto.to_string()
          }
        ));
      }
      if let Some(forwarded_host) = forwarded_host {
        forwarded_entry.push(format!(
          "host={}",
          if forwarded_host.contains(escape_determinants) {
            format!("\"{}\"", forwarded_host.escape_default())
          } else {
            forwarded_host.to_string()
          }
        ));
      }
      forwarded_header_value_new.push(forwarded_entry.join(";"));

      is_first = false;
    }

    forwarded_header_value = Some(forwarded_header_value_new.join(", "));
  }
  if let Some(forwarded_header_value) = forwarded_header_value {
    request_parts
      .headers
      .insert(header::FORWARDED, forwarded_header_value.parse()?);
  } else {
    request_parts.headers.remove(header::FORWARDED);
  }

  for (header_name_option, header_value) in headers_to_add {
    if let Some(header_name) = header_name_option {
      if !request_parts.headers.contains_key(&header_name) {
        request_parts.headers.insert(header_name, header_value);
      }
    }
  }

  for (header_name_option, header_value) in headers_to_replace {
    if let Some(header_name) = header_name_option {
      request_parts.headers.insert(header_name, header_value);
    }
  }

  for header_to_remove in headers_to_remove.into_iter().rev() {
    if request_parts.headers.contains_key(&header_to_remove) {
      while request_parts.headers.remove(&header_to_remove).is_some() {}
    }
  }

  request_parts.version = Version::default();

  Ok(request_parts)
}
