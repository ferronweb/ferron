use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use async_channel::Sender;
use chrono::{DateTime, Local};
use futures_util::stream::TryStreamExt;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full, StreamBody};
use hyper::body::{Body, Bytes, Frame};
use hyper::header::{HeaderName, HeaderValue};
use hyper::{header, HeaderMap, Method, Request, Response, StatusCode};
#[cfg(feature = "runtime-tokio")]
use tokio::io::BufReader;
#[cfg(feature = "runtime-tokio")]
use tokio_util::io::ReaderStream;

use crate::config::{ServerConfiguration, ServerConfigurations};
use crate::get_value;
use crate::logging::{ErrorLogger, LogMessage, Loggers};
use crate::modules::{ModuleHandlers, RequestData, SocketData};
use crate::runtime::timeout;
#[cfg(feature = "runtime-monoio")]
use crate::util::MonoioFileStream;
use crate::util::{
  generate_default_error_page, get_entries, get_entry, replace_header_placeholders, sanitize_url,
  SERVER_SOFTWARE,
};

/// Generates an error response
async fn generate_error_response(
  status_code: StatusCode,
  config: &ServerConfiguration,
  headers: &Option<HeaderMap>,
) -> Response<BoxBody<Bytes, std::io::Error>> {
  let bare_body = generate_default_error_page(
    status_code,
    get_value!("server_administrator_email", config).and_then(|v| v.as_str()),
  );
  let mut content_length: Option<u64> = bare_body.len().try_into().ok();
  let mut response_body = Full::new(Bytes::from(bare_body))
    .map_err(|e| match e {})
    .boxed();

  if let Some(error_pages) = get_entries!("error_page", config) {
    for error_page in &error_pages.inner {
      if let Some(page_status_code) = error_page.values.first().and_then(|v| v.as_i128()) {
        let page_status_code = match StatusCode::from_u16(match page_status_code.try_into() {
          Ok(status_code) => status_code,
          Err(_) => continue,
        }) {
          Ok(status_code) => status_code,
          Err(_) => continue,
        };
        if status_code != page_status_code {
          continue;
        }
        if let Some(page_path) = error_page.values.get(1).and_then(|v| v.as_str()) {
          #[cfg(feature = "runtime-monoio")]
          let file = monoio::fs::File::open(page_path).await;
          #[cfg(feature = "runtime-tokio")]
          let file = tokio::fs::File::open(page_path).await;

          let file = match file {
            Ok(file) => file,
            Err(_) => continue,
          };

          // Monoio's `File` doesn't expose `metadata()` on Windows, so we have to spawn a blocking task to obtain the metadata on this platform
          #[cfg(any(feature = "runtime-tokio", all(feature = "runtime-monoio", unix)))]
          let metadata = file.metadata().await;
          #[cfg(all(feature = "runtime-monoio", windows))]
          let metadata = {
            let page_path = page_path.to_owned();
            monoio::spawn_blocking(move || std::fs::metadata(page_path))
              .await
              .unwrap_or(Err(std::io::Error::other(
                "Can't spawn a blocking task to obtain the file metadata",
              )))
          };

          content_length = match metadata {
            Ok(metadata) => Some(metadata.len()),
            Err(_) => None,
          };

          #[cfg(feature = "runtime-monoio")]
          let file_stream = MonoioFileStream::new(file, None, None);
          #[cfg(feature = "runtime-tokio")]
          let file_stream = ReaderStream::new(BufReader::with_capacity(12800, file));

          let stream_body = StreamBody::new(file_stream.map_ok(Frame::data));
          let boxed_body = stream_body.boxed();

          response_body = boxed_body;

          break;
        }
      }
    }
  }

  let mut response_builder = Response::builder().status(status_code);

  if let Some(headers) = headers {
    let headers_iter = headers.iter();
    for (name, value) in headers_iter {
      if name != header::CONTENT_TYPE && name != header::CONTENT_LENGTH {
        response_builder = response_builder.header(name, value);
      }
    }
  }

  if let Some(content_length) = content_length {
    response_builder = response_builder.header(header::CONTENT_LENGTH, content_length);
  }
  response_builder = response_builder.header(header::CONTENT_TYPE, "text/html");

  response_builder.body(response_body).unwrap_or_default()
}

/// Sends a log message formatted according to the Combined Log Format
#[allow(clippy::too_many_arguments)]
async fn log_combined(
  logger: &Sender<LogMessage>,
  client_ip: IpAddr,
  auth_user: Option<String>,
  method: String,
  request_path: String,
  protocol: String,
  status_code: u16,
  content_length: Option<u64>,
  referrer: Option<String>,
  user_agent: Option<String>,
) {
  let now: DateTime<Local> = Local::now();
  let formatted_time = now.format("%d/%b/%Y:%H:%M:%S %z").to_string();
  logger
    .send(LogMessage::new(
      format!(
        "{} - {} [{}] \"{} {} {}\" {} {} {} {}",
        client_ip,
        match auth_user {
          Some(auth_user) => auth_user,
          None => String::from("-"),
        },
        formatted_time,
        method,
        request_path,
        protocol,
        status_code,
        match content_length {
          Some(content_length) => format!("{content_length}"),
          None => String::from("-"),
        },
        match referrer {
          Some(referrer) => format!(
            "\"{}\"",
            referrer.replace("\\", "\\\\").replace("\"", "\\\"")
          ),
          None => String::from("-"),
        },
        match user_agent {
          Some(user_agent) => format!(
            "\"{}\"",
            user_agent.replace("\\", "\\\\").replace("\"", "\\\"")
          ),
          None => String::from("-"),
        },
      ),
      false,
    ))
    .await
    .unwrap_or_default();
}

/// Helper function to add custom headers to response
fn add_custom_headers(
  response_parts: &mut hyper::http::response::Parts,
  headers_to_add: &HeaderMap,
  headers_to_replace: &HeaderMap,
  headers_to_remove: &[HeaderName],
) {
  for (header_name, header_value) in headers_to_add {
    if !response_parts.headers.contains_key(header_name) {
      response_parts
        .headers
        .insert(header_name, header_value.to_owned());
    }
  }

  for (header_name, header_value) in headers_to_replace {
    response_parts
      .headers
      .insert(header_name, header_value.to_owned());
  }

  for header_to_remove in headers_to_remove.iter().rev() {
    if response_parts.headers.contains_key(header_to_remove) {
      while response_parts.headers.remove(header_to_remove).is_some() {}
    }
  }
}

/// Helper function to add HTTP/3 Alt-Svc header
fn add_http3_alt_svc_header(
  response_parts: &mut hyper::http::response::Parts,
  http3_alt_port: Option<u16>,
) {
  if let Some(http3_alt_port) = http3_alt_port {
    if let Ok(header_value) = match response_parts.headers.get(header::ALT_SVC) {
      Some(value) => {
        let header_value_old = String::from_utf8_lossy(value.as_bytes());
        let header_value_new = format!("h3=\":{http3_alt_port}\", h3-29=\":{http3_alt_port}\"");

        if header_value_old != header_value_new {
          HeaderValue::from_bytes(format!("{header_value_old}, {header_value_new}").as_bytes())
        } else {
          HeaderValue::from_bytes(header_value_old.as_bytes())
        }
      }
      None => HeaderValue::from_bytes(
        format!("h3=\":{http3_alt_port}\", h3-29=\":{http3_alt_port}\"").as_bytes(),
      ),
    } {
      response_parts.headers.insert(header::ALT_SVC, header_value);
    }
  }
}

/// Helper function to add server header
fn add_server_header(response_parts: &mut hyper::http::response::Parts) {
  response_parts
    .headers
    .insert(header::SERVER, HeaderValue::from_static(SERVER_SOFTWARE));
}

/// Helper function to extract content length for logging
fn extract_content_length(response: &Response<BoxBody<Bytes, std::io::Error>>) -> Option<u64> {
  match response.headers().get(header::CONTENT_LENGTH) {
    Some(header_value) => match header_value.to_str() {
      Ok(header_value) => match header_value.parse::<u64>() {
        Ok(content_length) => Some(content_length),
        Err(_) => response.body().size_hint().exact(),
      },
      Err(_) => response.body().size_hint().exact(),
    },
    None => response.body().size_hint().exact(),
  }
}

/// Helper function to apply all response headers and log if needed
#[allow(clippy::too_many_arguments)]
async fn finalize_response_and_log(
  response: Response<BoxBody<Bytes, std::io::Error>>,
  http3_alt_port: Option<u16>,
  headers_to_add: HeaderMap,
  headers_to_replace: HeaderMap,
  headers_to_remove: Vec<HeaderName>,
  logger: &Option<Sender<LogMessage>>,
  log_enabled: bool,
  socket_data: &SocketData,
  latest_auth_data: Option<String>,
  log_method: String,
  log_request_path: String,
  log_protocol: String,
  log_referrer: Option<String>,
  log_user_agent: Option<String>,
) -> Response<BoxBody<Bytes, std::io::Error>> {
  let (mut response_parts, response_body) = response.into_parts();

  add_custom_headers(
    &mut response_parts,
    &headers_to_add,
    &headers_to_replace,
    &headers_to_remove,
  );
  add_http3_alt_svc_header(&mut response_parts, http3_alt_port);
  add_server_header(&mut response_parts);

  let response = Response::from_parts(response_parts, response_body);

  if log_enabled {
    if let Some(logger) = &logger {
      log_combined(
        logger,
        socket_data.remote_addr.ip(),
        latest_auth_data,
        log_method,
        log_request_path,
        log_protocol,
        response.status().as_u16(),
        extract_content_length(&response),
        log_referrer,
        log_user_agent,
      )
      .await;
    }
  }

  response
}

/// Helper function to execute response modifying handlers
#[allow(clippy::too_many_arguments)]
async fn execute_response_modifying_handlers(
  mut response: Response<BoxBody<Bytes, std::io::Error>>,
  mut executed_handlers: Vec<Box<dyn ModuleHandlers>>,
  configuration: &ServerConfiguration,
  http3_alt_port: Option<u16>,
  headers_to_add: HeaderMap,
  headers_to_replace: HeaderMap,
  headers_to_remove: Vec<HeaderName>,
  logger: &Option<Sender<LogMessage>>,
  error_log_enabled: bool,
  log_enabled: bool,
  socket_data: &SocketData,
  latest_auth_data: Option<String>,
  log_method: String,
  log_request_path: String,
  log_protocol: String,
  log_referrer: Option<String>,
  log_user_agent: Option<String>,
) -> Result<Response<BoxBody<Bytes, std::io::Error>>, Response<BoxBody<Bytes, std::io::Error>>> {
  while let Some(mut executed_handler) = executed_handlers.pop() {
    let response_status = executed_handler.response_modifying_handler(response).await;
    response = match response_status {
      Ok(response) => response,
      Err(err) => {
        if error_log_enabled {
          if let Some(logger) = &logger {
            logger
              .send(LogMessage::new(
                format!("Unexpected error while serving a request: {err}"),
                true,
              ))
              .await
              .unwrap_or_default();
          }
        }

        let error_response =
          generate_error_response(StatusCode::INTERNAL_SERVER_ERROR, configuration, &None).await;

        let final_response = finalize_response_and_log(
          error_response,
          http3_alt_port,
          headers_to_add,
          headers_to_replace,
          headers_to_remove,
          logger,
          log_enabled,
          socket_data,
          latest_auth_data,
          log_method,
          log_request_path,
          log_protocol,
          log_referrer,
          log_user_agent,
        )
        .await;

        return Err(final_response);
      }
    };
  }
  Ok(response)
}

/// The HTTP request handler
#[allow(clippy::too_many_arguments)]
async fn request_handler_wrapped(
  mut request: Request<BoxBody<Bytes, std::io::Error>>,
  client_address: SocketAddr,
  server_address: SocketAddr,
  encrypted: bool,
  configurations: Arc<ServerConfigurations>,
  loggers: Loggers,
  http3_alt_port: Option<u16>,
  acme_http_01_resolvers: Arc<Vec<crate::acme::Http01DataLock>>,
  proxy_protocol_client_address: Option<SocketAddr>,
  proxy_protocol_server_address: Option<SocketAddr>,
) -> Result<Response<BoxBody<Bytes, std::io::Error>>, Infallible> {
  // Global configuration
  let global_configuration = configurations.find_global_configuration();

  // Collect request data for logging
  let is_proxy_request = match request.version() {
    hyper::Version::HTTP_2 | hyper::Version::HTTP_3 => {
      request.method() == hyper::Method::CONNECT && request.uri().host().is_some()
    }
    _ => request.uri().host().is_some(),
  };
  let log_method = String::from(request.method().as_str());
  let log_request_path = match is_proxy_request {
    true => request.uri().to_string(),
    false => format!(
      "{}{}",
      request.uri().path(),
      match request.uri().query() {
        Some(query) => format!("?{query}"),
        None => String::from(""),
      }
    ),
  };
  let log_protocol = String::from(match request.version() {
    hyper::Version::HTTP_09 => "HTTP/0.9",
    hyper::Version::HTTP_10 => "HTTP/1.0",
    hyper::Version::HTTP_11 => "HTTP/1.1",
    hyper::Version::HTTP_2 => "HTTP/2.0",
    hyper::Version::HTTP_3 => "HTTP/3.0",
    _ => "HTTP/Unknown",
  });
  let log_referrer = match request.headers().get(header::REFERER) {
    Some(header_value) => match header_value.to_str() {
      Ok(header_value) => Some(String::from(header_value)),
      Err(_) => None,
    },
    None => None,
  };
  let log_user_agent = match request.headers().get(header::USER_AGENT) {
    Some(header_value) => match header_value.to_str() {
      Ok(header_value) => Some(String::from(header_value)),
      Err(_) => None,
    },
    None => None,
  };
  let log_enabled = !global_configuration
    .as_deref()
    .and_then(|c| get_value!("log", c))
    .is_none_or(|v| v.is_null());
  let error_log_enabled = !global_configuration
    .as_deref()
    .and_then(|c| get_value!("error_log", c))
    .is_none_or(|v| v.is_null());

  // Normalize HTTP/2 and HTTP/3 request objects
  match request.version() {
    hyper::Version::HTTP_2 | hyper::Version::HTTP_3 => {
      // Set "Host" request header for HTTP/2 and HTTP/3 connections
      if let Some(authority) = request.uri().authority() {
        let authority = authority.to_owned();
        let headers = request.headers_mut();
        if !headers.contains_key(header::HOST) {
          if let Ok(authority_value) = HeaderValue::from_bytes(authority.as_str().as_bytes()) {
            headers.append(header::HOST, authority_value);
          }
        }
      }

      // Normalize the Cookie header for HTTP/2 and HTTP/3
      let mut cookie_normalized = String::new();
      let mut cookie_set = false;
      let headers = request.headers_mut();
      for cookie in headers.get_all(header::COOKIE) {
        if let Ok(cookie) = cookie.to_str() {
          if cookie_set {
            cookie_normalized.push_str("; ");
          }
          cookie_set = true;
          cookie_normalized.push_str(cookie);
        }
      }
      if cookie_set {
        if let Ok(cookie_value) = HeaderValue::from_bytes(cookie_normalized.as_bytes()) {
          headers.insert(header::COOKIE, cookie_value);
        }
      }
    }
    _ => (),
  }

  // Construct socket data
  let mut socket_data = SocketData {
    remote_addr: proxy_protocol_client_address.unwrap_or(client_address),
    local_addr: proxy_protocol_server_address.unwrap_or(server_address),
    encrypted,
  };

  // Sanitize "Host" header
  let host_header_option = request.headers().get(header::HOST);
  if let Some(header_data) = host_header_option {
    match header_data.to_str() {
      Ok(host_header) => {
        let host_header_lower_case = host_header.to_lowercase();
        if host_header_lower_case != *host_header {
          let host_header_value = match HeaderValue::from_str(&host_header_lower_case) {
            Ok(host_header_value) => host_header_value,
            Err(err) => {
              if error_log_enabled {
                if let Some(logger) = loggers.find_global_logger() {
                  logger
                    .send(LogMessage::new(
                      format!("Host header sanitation error: {err}"),
                      true,
                    ))
                    .await
                    .unwrap_or_default();
                }
              }
              let response = Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header(header::CONTENT_TYPE, "text/html")
                .body(
                  Full::new(Bytes::from(generate_default_error_page(
                    StatusCode::BAD_REQUEST,
                    None,
                  )))
                  .map_err(|e| match e {})
                  .boxed(),
                )
                .unwrap_or_default();

              if log_enabled {
                if let Some(logger) = loggers.find_global_logger() {
                  log_combined(
                    &logger,
                    socket_data.remote_addr.ip(),
                    None,
                    log_method,
                    log_request_path,
                    log_protocol,
                    response.status().as_u16(),
                    match response.headers().get(header::CONTENT_LENGTH) {
                      Some(header_value) => match header_value.to_str() {
                        Ok(header_value) => match header_value.parse::<u64>() {
                          Ok(content_length) => Some(content_length),
                          Err(_) => response.body().size_hint().exact(),
                        },
                        Err(_) => response.body().size_hint().exact(),
                      },
                      None => response.body().size_hint().exact(),
                    },
                    log_referrer,
                    log_user_agent,
                  )
                  .await;
                }
              }
              let (mut response_parts, response_body) = response.into_parts();
              if let Some(http3_alt_port) = http3_alt_port {
                if let Ok(header_value) = match response_parts.headers.get(header::ALT_SVC) {
                  Some(value) => {
                    let header_value_old = String::from_utf8_lossy(value.as_bytes());
                    let header_value_new =
                      format!("h3=\":{http3_alt_port}\", h3-29=\":{http3_alt_port}\"");

                    if header_value_old != header_value_new {
                      HeaderValue::from_bytes(
                        format!("{header_value_old}, {header_value_new}").as_bytes(),
                      )
                    } else {
                      HeaderValue::from_bytes(header_value_old.as_bytes())
                    }
                  }
                  None => HeaderValue::from_bytes(
                    format!("h3=\":{http3_alt_port}\", h3-29=\":{http3_alt_port}\"").as_bytes(),
                  ),
                } {
                  response_parts.headers.insert(header::ALT_SVC, header_value);
                }
              }
              response_parts
                .headers
                .insert(header::SERVER, HeaderValue::from_static(SERVER_SOFTWARE));

              return Ok(Response::from_parts(response_parts, response_body));
            }
          };

          request
            .headers_mut()
            .insert(header::HOST, host_header_value);
        }
      }
      Err(err) => {
        if error_log_enabled {
          if let Some(logger) = loggers.find_global_logger() {
            logger
              .send(LogMessage::new(
                format!("Host header sanitation error: {err}"),
                true,
              ))
              .await
              .unwrap_or_default();
          }
        }
        let response = Response::builder()
          .status(StatusCode::BAD_REQUEST)
          .header(header::CONTENT_TYPE, "text/html")
          .body(
            Full::new(Bytes::from(generate_default_error_page(
              StatusCode::BAD_REQUEST,
              None,
            )))
            .map_err(|e| match e {})
            .boxed(),
          )
          .unwrap_or_default();
        if log_enabled {
          if let Some(logger) = loggers.find_global_logger() {
            log_combined(
              &logger,
              socket_data.remote_addr.ip(),
              None,
              log_method,
              log_request_path,
              log_protocol,
              response.status().as_u16(),
              match response.headers().get(header::CONTENT_LENGTH) {
                Some(header_value) => match header_value.to_str() {
                  Ok(header_value) => match header_value.parse::<u64>() {
                    Ok(content_length) => Some(content_length),
                    Err(_) => response.body().size_hint().exact(),
                  },
                  Err(_) => response.body().size_hint().exact(),
                },
                None => response.body().size_hint().exact(),
              },
              log_referrer,
              log_user_agent,
            )
            .await;
          }
        }
        let (mut response_parts, response_body) = response.into_parts();
        if let Some(http3_alt_port) = http3_alt_port {
          if let Ok(header_value) = match response_parts.headers.get(header::ALT_SVC) {
            Some(value) => {
              let header_value_old = String::from_utf8_lossy(value.as_bytes());
              let header_value_new =
                format!("h3=\":{http3_alt_port}\", h3-29=\":{http3_alt_port}\"");

              if header_value_old != header_value_new {
                HeaderValue::from_bytes(
                  format!("{header_value_old}, {header_value_new}").as_bytes(),
                )
              } else {
                HeaderValue::from_bytes(header_value_old.as_bytes())
              }
            }
            None => HeaderValue::from_bytes(
              format!("h3=\":{http3_alt_port}\", h3-29=\":{http3_alt_port}\"").as_bytes(),
            ),
          } {
            response_parts.headers.insert(header::ALT_SVC, header_value);
          }
        }
        response_parts
          .headers
          .insert(header::SERVER, HeaderValue::from_static(SERVER_SOFTWARE));

        return Ok(Response::from_parts(response_parts, response_body));
      }
    }
  };

  let hostname_determinant = match request.headers().get(header::HOST) {
    Some(value) => value.to_str().ok().map(|h| {
      if let Some((left, right)) = h.rsplit_once(':') {
        if right.parse::<u16>().is_ok() {
          left
        } else {
          h
        }
      } else {
        h
      }
    }),
    None => None,
  };

  // Find the server configuration
  let mut configuration = match configurations.find_configuration(
    request.uri().path(),
    hostname_determinant,
    socket_data.local_addr.ip(),
    socket_data.local_addr.port(),
  ) {
    Some(configuration) => configuration,
    None => {
      if error_log_enabled {
        if let Some(logger) = loggers.find_global_logger() {
          logger
            .send(LogMessage::new(
              String::from("Cannot determine server configuration"),
              true,
            ))
            .await
            .unwrap_or_default()
        }
      }
      let response = Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .header(header::CONTENT_TYPE, "text/html")
        .body(
          Full::new(Bytes::from(generate_default_error_page(
            StatusCode::INTERNAL_SERVER_ERROR,
            None,
          )))
          .map_err(|e| match e {})
          .boxed(),
        )
        .unwrap_or_default();
      if log_enabled {
        if let Some(logger) = loggers.find_global_logger() {
          log_combined(
            &logger,
            socket_data.remote_addr.ip(),
            None,
            log_method,
            log_request_path,
            log_protocol,
            response.status().as_u16(),
            match response.headers().get(header::CONTENT_LENGTH) {
              Some(header_value) => match header_value.to_str() {
                Ok(header_value) => match header_value.parse::<u64>() {
                  Ok(content_length) => Some(content_length),
                  Err(_) => response.body().size_hint().exact(),
                },
                Err(_) => response.body().size_hint().exact(),
              },
              None => response.body().size_hint().exact(),
            },
            log_referrer,
            log_user_agent,
          )
          .await;
        }
      }
      let (mut response_parts, response_body) = response.into_parts();
      if let Some(http3_alt_port) = http3_alt_port {
        if let Ok(header_value) = match response_parts.headers.get(header::ALT_SVC) {
          Some(value) => {
            let header_value_old = String::from_utf8_lossy(value.as_bytes());
            let header_value_new = format!("h3=\":{http3_alt_port}\", h3-29=\":{http3_alt_port}\"");

            if header_value_old != header_value_new {
              HeaderValue::from_bytes(format!("{header_value_old}, {header_value_new}").as_bytes())
            } else {
              HeaderValue::from_bytes(header_value_old.as_bytes())
            }
          }
          None => HeaderValue::from_bytes(
            format!("h3=\":{http3_alt_port}\", h3-29=\":{http3_alt_port}\"").as_bytes(),
          ),
        } {
          response_parts.headers.insert(header::ALT_SVC, header_value);
        }
      }
      response_parts
        .headers
        .insert(header::SERVER, HeaderValue::from_static(SERVER_SOFTWARE));

      return Ok(Response::from_parts(response_parts, response_body));
    }
  };

  // Determine the logger
  let logger = loggers.find_logger(
    hostname_determinant,
    socket_data.local_addr.ip(),
    socket_data.local_addr.port(),
  );
  let log_enabled = !get_value!("log", configuration).is_none_or(|v| v.is_null());
  let error_log_enabled = !get_value!("error_log", configuration).is_none_or(|v| v.is_null());

  // Sanitize the URL
  let url_pathname = request.uri().path();
  let sanitized_url_pathname = match sanitize_url(
    url_pathname,
    get_value!("allow_double_slashes", configuration)
      .and_then(|v| v.as_bool())
      .unwrap_or(false),
  ) {
    Ok(sanitized_url) => sanitized_url,
    Err(err) => {
      if error_log_enabled {
        if let Some(logger) = &logger {
          logger
            .send(LogMessage::new(
              format!("URL sanitation error: {err}"),
              true,
            ))
            .await
            .unwrap_or_default();
        }
      }
      let response = generate_error_response(StatusCode::BAD_REQUEST, &configuration, &None).await;

      // Determine headers to add/remove/replace
      let mut headers_to_add = HeaderMap::new();
      let mut headers_to_replace = HeaderMap::new();
      let mut headers_to_remove = Vec::new();
      let (request_parts, _) = request.into_parts();
      if let Some(custom_headers) = get_entries!("header", configuration) {
        for custom_header in custom_headers.inner.iter().rev() {
          if let Some(header_name) = custom_header.values.first().and_then(|v| v.as_str()) {
            if let Some(header_value) = custom_header.values.get(1).and_then(|v| v.as_str()) {
              if !headers_to_add.contains_key(header_name) {
                if let Ok(header_name) = HeaderName::from_str(header_name) {
                  if let Ok(header_value) = HeaderValue::from_str(&replace_header_placeholders(
                    header_value,
                    &request_parts,
                    None,
                  )) {
                    headers_to_add.insert(header_name, header_value);
                  }
                }
              }
            }
          }
        }
      }
      if let Some(custom_headers) = get_entries!("header_replace", configuration) {
        for custom_header in custom_headers.inner.iter().rev() {
          if let Some(header_name) = custom_header.values.first().and_then(|v| v.as_str()) {
            if let Some(header_value) = custom_header.values.get(1).and_then(|v| v.as_str()) {
              if let Ok(header_name) = HeaderName::from_str(header_name) {
                if let Ok(header_value) = HeaderValue::from_str(&replace_header_placeholders(
                  header_value,
                  &request_parts,
                  None,
                )) {
                  headers_to_replace.insert(header_name, header_value);
                }
              }
            }
          }
        }
      }
      if let Some(custom_headers_to_remove) = get_entries!("header_remove", configuration) {
        for custom_header in custom_headers_to_remove.inner.iter().rev() {
          if let Some(header_name) = custom_header.values.first().and_then(|v| v.as_str()) {
            if let Ok(header_name) = HeaderName::from_str(header_name) {
              headers_to_remove.push(header_name);
            }
          }
        }
      }

      return Ok(
        finalize_response_and_log(
          response,
          http3_alt_port,
          headers_to_add,
          headers_to_replace,
          headers_to_remove,
          &logger,
          log_enabled,
          &socket_data,
          None,
          log_method,
          log_request_path,
          log_protocol,
          log_referrer,
          log_user_agent,
        )
        .await,
      );
    }
  };

  if sanitized_url_pathname != url_pathname {
    let (mut parts, body) = request.into_parts();
    let orig_uri = parts.uri.clone();
    let mut url_parts = parts.uri.into_parts();
    url_parts.path_and_query =
      Some(
        match format!(
          "{}{}",
          sanitized_url_pathname,
          match url_parts.path_and_query {
            Some(path_and_query) => {
              match path_and_query.query() {
                Some(query) => format!("?{query}"),
                None => String::from(""),
              }
            }
            None => String::from(""),
          }
        )
        .parse()
        {
          Ok(path_and_query) => path_and_query,
          Err(err) => {
            if error_log_enabled {
              if let Some(logger) = &logger {
                logger
                  .send(LogMessage::new(
                    format!("URL sanitation error: {err}"),
                    true,
                  ))
                  .await
                  .unwrap_or_default();
              }
            }
            let response =
              generate_error_response(StatusCode::BAD_REQUEST, &configuration, &None).await;

            parts.uri = orig_uri;

            // Determine headers to add/remove/replace
            let mut headers_to_add = HeaderMap::new();
            let mut headers_to_replace = HeaderMap::new();
            let mut headers_to_remove = Vec::new();
            if let Some(custom_headers) = get_entries!("header", configuration) {
              for custom_header in custom_headers.inner.iter().rev() {
                if let Some(header_name) = custom_header.values.first().and_then(|v| v.as_str()) {
                  if let Some(header_value) = custom_header.values.get(1).and_then(|v| v.as_str()) {
                    if !headers_to_add.contains_key(header_name) {
                      if let Ok(header_name) = HeaderName::from_str(header_name) {
                        if let Ok(header_value) = HeaderValue::from_str(
                          &replace_header_placeholders(header_value, &parts, None),
                        ) {
                          headers_to_add.insert(header_name, header_value);
                        }
                      }
                    }
                  }
                }
              }
            }
            if let Some(custom_headers) = get_entries!("header_replace", configuration) {
              for custom_header in custom_headers.inner.iter().rev() {
                if let Some(header_name) = custom_header.values.first().and_then(|v| v.as_str()) {
                  if let Some(header_value) = custom_header.values.get(1).and_then(|v| v.as_str()) {
                    if let Ok(header_name) = HeaderName::from_str(header_name) {
                      if let Ok(header_value) = HeaderValue::from_str(&replace_header_placeholders(
                        header_value,
                        &parts,
                        None,
                      )) {
                        headers_to_replace.insert(header_name, header_value);
                      }
                    }
                  }
                }
              }
            }
            if let Some(custom_headers_to_remove) = get_entries!("header_remove", configuration) {
              for custom_header in custom_headers_to_remove.inner.iter().rev() {
                if let Some(header_name) = custom_header.values.first().and_then(|v| v.as_str()) {
                  if let Ok(header_name) = HeaderName::from_str(header_name) {
                    headers_to_remove.push(header_name);
                  }
                }
              }
            }

            return Ok(
              finalize_response_and_log(
                response,
                http3_alt_port,
                headers_to_add,
                headers_to_replace,
                headers_to_remove,
                &logger,
                log_enabled,
                &socket_data,
                None,
                log_method,
                log_request_path,
                log_protocol,
                log_referrer,
                log_user_agent,
              )
              .await,
            );
          }
        },
      );
    parts.uri = match hyper::Uri::from_parts(url_parts) {
      Ok(uri) => uri,
      Err(err) => {
        if error_log_enabled {
          if let Some(logger) = &logger {
            logger
              .send(LogMessage::new(
                format!("URL sanitation error: {err}"),
                true,
              ))
              .await
              .unwrap_or_default();
          }
        }
        let response =
          generate_error_response(StatusCode::BAD_REQUEST, &configuration, &None).await;

        parts.uri = orig_uri;

        // Determine headers to add/remove/replace
        let mut headers_to_add = HeaderMap::new();
        let mut headers_to_replace = HeaderMap::new();
        let mut headers_to_remove = Vec::new();
        if let Some(custom_headers) = get_entries!("header", configuration) {
          for custom_header in custom_headers.inner.iter().rev() {
            if let Some(header_name) = custom_header.values.first().and_then(|v| v.as_str()) {
              if let Some(header_value) = custom_header.values.get(1).and_then(|v| v.as_str()) {
                if !headers_to_add.contains_key(header_name) {
                  if let Ok(header_name) = HeaderName::from_str(header_name) {
                    if let Ok(header_value) = HeaderValue::from_str(&replace_header_placeholders(
                      header_value,
                      &parts,
                      None,
                    )) {
                      headers_to_add.insert(header_name, header_value);
                    }
                  }
                }
              }
            }
          }
        }
        if let Some(custom_headers) = get_entries!("header_replace", configuration) {
          for custom_header in custom_headers.inner.iter().rev() {
            if let Some(header_name) = custom_header.values.first().and_then(|v| v.as_str()) {
              if let Some(header_value) = custom_header.values.get(1).and_then(|v| v.as_str()) {
                if let Ok(header_name) = HeaderName::from_str(header_name) {
                  if let Ok(header_value) =
                    HeaderValue::from_str(&replace_header_placeholders(header_value, &parts, None))
                  {
                    headers_to_replace.insert(header_name, header_value);
                  }
                }
              }
            }
          }
        }
        if let Some(custom_headers_to_remove) = get_entries!("header_remove", configuration) {
          for custom_header in custom_headers_to_remove.inner.iter().rev() {
            if let Some(header_name) = custom_header.values.first().and_then(|v| v.as_str()) {
              if let Ok(header_name) = HeaderName::from_str(header_name) {
                headers_to_remove.push(header_name);
              }
            }
          }
        }

        return Ok(
          finalize_response_and_log(
            response,
            http3_alt_port,
            headers_to_add,
            headers_to_replace,
            headers_to_remove,
            &logger,
            log_enabled,
            &socket_data,
            None,
            log_method,
            log_request_path,
            log_protocol,
            log_referrer,
            log_user_agent,
          )
          .await,
        );
      }
    };
    request = Request::from_parts(parts, body);
  }

  // Determine headers to add/remove/replace
  let mut headers_to_add = HeaderMap::new();
  let mut headers_to_replace = HeaderMap::new();
  let mut headers_to_remove = Vec::new();
  let (request_parts, request_body) = request.into_parts();
  if let Some(custom_headers) = get_entries!("header", configuration) {
    for custom_header in custom_headers.inner.iter().rev() {
      if let Some(header_name) = custom_header.values.first().and_then(|v| v.as_str()) {
        if let Some(header_value) = custom_header.values.get(1).and_then(|v| v.as_str()) {
          if !headers_to_add.contains_key(header_name) {
            if let Ok(header_name) = HeaderName::from_str(header_name) {
              if let Ok(header_value) = HeaderValue::from_str(&replace_header_placeholders(
                header_value,
                &request_parts,
                None,
              )) {
                headers_to_add.insert(header_name, header_value);
              }
            }
          }
        }
      }
    }
  }
  if let Some(custom_headers) = get_entries!("header_replace", configuration) {
    for custom_header in custom_headers.inner.iter().rev() {
      if let Some(header_name) = custom_header.values.first().and_then(|v| v.as_str()) {
        if let Some(header_value) = custom_header.values.get(1).and_then(|v| v.as_str()) {
          if let Ok(header_name) = HeaderName::from_str(header_name) {
            if let Ok(header_value) = HeaderValue::from_str(&replace_header_placeholders(
              header_value,
              &request_parts,
              None,
            )) {
              headers_to_replace.insert(header_name, header_value);
            }
          }
        }
      }
    }
  }
  if let Some(custom_headers_to_remove) = get_entries!("header_remove", configuration) {
    for custom_header in custom_headers_to_remove.inner.iter().rev() {
      if let Some(header_name) = custom_header.values.first().and_then(|v| v.as_str()) {
        if let Ok(header_name) = HeaderName::from_str(header_name) {
          headers_to_remove.push(header_name);
        }
      }
    }
  }
  let mut request = Request::from_parts(request_parts, request_body);

  if request.uri().path() == "*" {
    let response = match request.method() {
      &Method::OPTIONS => Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header(header::ALLOW, "GET, POST, HEAD, OPTIONS")
        .body(Empty::new().map_err(|e| match e {}).boxed())
        .unwrap_or_default(),
      _ => {
        let mut header_map = HeaderMap::new();
        if let Ok(header_value) = HeaderValue::from_str("GET, POST, HEAD, OPTIONS") {
          header_map.insert(header::ALLOW, header_value);
        };
        generate_error_response(StatusCode::BAD_REQUEST, &configuration, &Some(header_map)).await
      }
    };
    return Ok(
      finalize_response_and_log(
        response,
        http3_alt_port,
        headers_to_add,
        headers_to_replace,
        headers_to_remove,
        &logger,
        log_enabled,
        &socket_data,
        None,
        log_method,
        log_request_path,
        log_protocol,
        log_referrer,
        log_user_agent,
      )
      .await,
    );
  }

  // HTTP-01 ACME challenge for automatic TLS
  if !acme_http_01_resolvers.is_empty() {
    if let Some(challenge_token) = request
      .uri()
      .path()
      .strip_prefix("/.well-known/acme-challenge/")
    {
      for acme_http01_resolver in &*acme_http_01_resolvers {
        if let Some(http01_acme_data) = &*acme_http01_resolver.read().await {
          let acme_response = http01_acme_data.1.clone();
          if challenge_token == http01_acme_data.0 {
            let response = Response::builder()
              .status(StatusCode::OK)
              .header(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
              )
              .body(
                Full::new(Bytes::from(acme_response))
                  .map_err(|e| match e {})
                  .boxed(),
              )
              .unwrap_or_default();

            return Ok(
              finalize_response_and_log(
                response,
                http3_alt_port,
                headers_to_add,
                headers_to_replace,
                headers_to_remove,
                &logger,
                log_enabled,
                &socket_data,
                None,
                log_method,
                log_request_path,
                log_protocol,
                log_referrer,
                log_user_agent,
              )
              .await,
            );
          }
        }
      }
    }
  };

  // Create an error logger
  let cloned_logger = logger.clone();
  let error_logger = match (cloned_logger, error_log_enabled) {
    (Some(cloned_logger), true) => ErrorLogger::new(cloned_logger),
    _ => ErrorLogger::without_logger(),
  };

  // Obtain module handlers
  let mut module_handlers = Vec::new();
  for module in &configuration.modules {
    module_handlers.push(module.get_module_handlers());
  }

  // Execute modules!
  request.extensions_mut().insert(RequestData {
    auth_user: None,
    original_url: None,
    error_status_code: None,
  });
  let mut executed_handlers = Vec::new();
  let (request_parts, request_body) = request.into_parts();
  let request_parts_cloned = if configurations.inner.iter().rev().any(|c| {
    c.filters.hostname == configuration.filters.hostname
      && c.filters.ip == configuration.filters.ip
      && c.filters.port == configuration.filters.port
      && (c.filters.location_prefix.is_none()
        || c.filters.location_prefix == configuration.filters.location_prefix)
      && c.filters.error_handler_status.is_some()
  }) {
    let mut request_parts_cloned = request_parts.clone();
    request_parts_cloned
      .headers
      .insert(header::CONTENT_LENGTH, HeaderValue::from_static("0"));
    Some(request_parts_cloned)
  } else {
    // If the error configuration is not specified, don't clone the request parts to improve performance
    None
  };
  let mut request = Request::from_parts(request_parts, request_body);
  let mut latest_auth_data = None;
  let mut is_error_handler = false;
  let mut handlers_iter: Box<dyn Iterator<Item = Box<dyn ModuleHandlers>>> =
    Box::new(module_handlers.into_iter());
  while let Some(mut handlers) = handlers_iter.next() {
    let response_result = handlers
      .request_handler(request, &configuration, &socket_data, &error_logger)
      .await;

    executed_handlers.push(handlers);
    match response_result {
      Ok(response) => {
        let status = response.response_status;
        let headers = response.response_headers;
        let new_remote_address = response.new_remote_address;
        let request_option = response.request;
        let response = response.response;
        let request_extensions = request_option
          .as_ref()
          .and_then(|r| r.extensions().get::<RequestData>());
        if let Some(request_extensions) = request_extensions {
          latest_auth_data = request_extensions.auth_user.clone();
        }
        if let Some(new_remote_address) = new_remote_address {
          socket_data.remote_addr = new_remote_address;
        };

        match response {
          Some(response) => {
            let (mut response_parts, response_body) = response.into_parts();
            add_custom_headers(
              &mut response_parts,
              &headers_to_add,
              &headers_to_replace,
              &headers_to_remove,
            );
            add_http3_alt_svc_header(&mut response_parts, http3_alt_port);
            add_server_header(&mut response_parts);

            let response = Response::from_parts(response_parts, response_body);

            match execute_response_modifying_handlers(
              response,
              executed_handlers,
              &configuration,
              http3_alt_port,
              headers_to_add,
              headers_to_replace,
              headers_to_remove,
              &logger,
              error_log_enabled,
              log_enabled,
              &socket_data,
              latest_auth_data.clone(),
              log_method.clone(),
              log_request_path.clone(),
              log_protocol.clone(),
              log_referrer.clone(),
              log_user_agent.clone(),
            )
            .await
            {
              Ok(response) => {
                if log_enabled {
                  if let Some(logger) = &logger {
                    log_combined(
                      logger,
                      socket_data.remote_addr.ip(),
                      latest_auth_data,
                      log_method,
                      log_request_path,
                      log_protocol,
                      response.status().as_u16(),
                      extract_content_length(&response),
                      log_referrer,
                      log_user_agent,
                    )
                    .await;
                  }
                }
                return Ok(response);
              }
              Err(error_response) => {
                return Ok(error_response);
              }
            }
          }
          None => match status {
            Some(status) => {
              if !is_error_handler {
                if let Some(error_configuration) =
                  configurations.find_error_configuration(&configuration.filters, status.as_u16())
                {
                  let request_option = if let Some(request) = request_option {
                    Some(request)
                  } else {
                    request_parts_cloned.clone().map(|request_parts_cloned| {
                      Request::from_parts(
                        request_parts_cloned,
                        Empty::new().map_err(|e| match e {}).boxed(),
                      )
                    })
                  };
                  if let Some(request_cloned) = request_option {
                    configuration = error_configuration;
                    handlers_iter = Box::new(executed_handlers.into_iter().chain(handlers_iter));
                    executed_handlers = Vec::new();
                    request = request_cloned;
                    if let Some(request_data) = request.extensions_mut().get_mut::<RequestData>() {
                      request_data.error_status_code = Some(status);
                    }
                    is_error_handler = true;
                    continue;
                  }
                }
              }
              let response = generate_error_response(status, &configuration, &headers).await;

              let (mut response_parts, response_body) = response.into_parts();
              add_custom_headers(
                &mut response_parts,
                &headers_to_add,
                &headers_to_replace,
                &headers_to_remove,
              );
              add_http3_alt_svc_header(&mut response_parts, http3_alt_port);
              add_server_header(&mut response_parts);

              let response = Response::from_parts(response_parts, response_body);

              match execute_response_modifying_handlers(
                response,
                executed_handlers,
                &configuration,
                http3_alt_port,
                headers_to_add,
                headers_to_replace,
                headers_to_remove,
                &logger,
                error_log_enabled,
                log_enabled,
                &socket_data,
                latest_auth_data.clone(),
                log_method.clone(),
                log_request_path.clone(),
                log_protocol.clone(),
                log_referrer.clone(),
                log_user_agent.clone(),
              )
              .await
              {
                Ok(response) => {
                  if log_enabled {
                    if let Some(logger) = &logger {
                      log_combined(
                        logger,
                        socket_data.remote_addr.ip(),
                        latest_auth_data,
                        log_method,
                        log_request_path,
                        log_protocol,
                        response.status().as_u16(),
                        extract_content_length(&response),
                        log_referrer,
                        log_user_agent,
                      )
                      .await;
                    }
                  }
                  return Ok(response);
                }
                Err(error_response) => {
                  return Ok(error_response);
                }
              }
            }
            None => match request_option {
              Some(request_obtained) => {
                request = request_obtained;
                continue;
              }
              None => {
                break;
              }
            },
          },
        }
      }
      Err(err) => {
        let response =
          generate_error_response(StatusCode::INTERNAL_SERVER_ERROR, &configuration, &None).await;

        if error_log_enabled {
          if let Some(logger) = &logger {
            logger
              .send(LogMessage::new(
                format!("Unexpected error while serving a request: {err}"),
                true,
              ))
              .await
              .unwrap_or_default();
          }
        }

        let (mut response_parts, response_body) = response.into_parts();
        add_custom_headers(
          &mut response_parts,
          &headers_to_add,
          &headers_to_replace,
          &headers_to_remove,
        );
        add_http3_alt_svc_header(&mut response_parts, http3_alt_port);
        add_server_header(&mut response_parts);

        let response = Response::from_parts(response_parts, response_body);

        match execute_response_modifying_handlers(
          response,
          executed_handlers,
          &configuration,
          http3_alt_port,
          headers_to_add,
          headers_to_replace,
          headers_to_remove,
          &logger,
          error_log_enabled,
          log_enabled,
          &socket_data,
          latest_auth_data.clone(),
          log_method.clone(),
          log_request_path.clone(),
          log_protocol.clone(),
          log_referrer.clone(),
          log_user_agent.clone(),
        )
        .await
        {
          Ok(response) => {
            if log_enabled {
              if let Some(logger) = &logger {
                log_combined(
                  logger,
                  socket_data.remote_addr.ip(),
                  latest_auth_data,
                  log_method,
                  log_request_path,
                  log_protocol,
                  response.status().as_u16(),
                  extract_content_length(&response),
                  log_referrer,
                  log_user_agent,
                )
                .await;
              }
            }
            return Ok(response);
          }
          Err(error_response) => {
            return Ok(error_response);
          }
        }
      }
    }
  }

  let response = generate_error_response(StatusCode::NOT_FOUND, &configuration, &None).await;

  let (mut response_parts, response_body) = response.into_parts();
  add_custom_headers(
    &mut response_parts,
    &headers_to_add,
    &headers_to_replace,
    &headers_to_remove,
  );
  add_http3_alt_svc_header(&mut response_parts, http3_alt_port);
  add_server_header(&mut response_parts);

  let response = Response::from_parts(response_parts, response_body);

  match execute_response_modifying_handlers(
    response,
    executed_handlers,
    &configuration,
    http3_alt_port,
    headers_to_add,
    headers_to_replace,
    headers_to_remove,
    &logger,
    error_log_enabled,
    log_enabled,
    &socket_data,
    latest_auth_data.clone(),
    log_method.clone(),
    log_request_path.clone(),
    log_protocol.clone(),
    log_referrer.clone(),
    log_user_agent.clone(),
  )
  .await
  {
    Ok(response) => {
      if log_enabled {
        if let Some(logger) = &logger {
          log_combined(
            logger,
            socket_data.remote_addr.ip(),
            latest_auth_data,
            log_method,
            log_request_path,
            log_protocol,
            response.status().as_u16(),
            extract_content_length(&response),
            log_referrer,
            log_user_agent,
          )
          .await;
        }
      }
      Ok(response)
    }
    Err(error_response) => Ok(error_response),
  }
}

/// The HTTP request handler, with timeout
#[allow(clippy::too_many_arguments)]
pub async fn request_handler(
  request: Request<BoxBody<Bytes, std::io::Error>>,
  client_address: SocketAddr,
  server_address: SocketAddr,
  encrypted: bool,
  configurations: Arc<ServerConfigurations>,
  loggers: Loggers,
  http3_alt_port: Option<u16>,
  acme_http_01_resolvers: Arc<Vec<crate::acme::Http01DataLock>>,
  proxy_protocol_client_address: Option<SocketAddr>,
  proxy_protocol_server_address: Option<SocketAddr>,
) -> Result<Response<BoxBody<Bytes, std::io::Error>>, anyhow::Error> {
  let global_configuration = configurations.find_global_configuration();
  let timeout_from_config = global_configuration
    .as_deref()
    .and_then(|c| get_entry!("timeout", c))
    .and_then(|e| e.values.last());
  let request_handler_future = request_handler_wrapped(
    request,
    client_address,
    server_address,
    encrypted,
    configurations,
    loggers,
    http3_alt_port,
    acme_http_01_resolvers,
    proxy_protocol_client_address,
    proxy_protocol_server_address,
  );
  if timeout_from_config.is_some_and(|v| v.is_null()) {
    request_handler_future.await.map_err(|e| anyhow::anyhow!(e))
  } else {
    let timeout_millis = timeout_from_config
      .and_then(|v| v.as_i128())
      .unwrap_or(300000) as u64;
    match timeout(
      Duration::from_millis(timeout_millis),
      request_handler_future,
    )
    .await
    {
      Ok(response) => response.map_err(|e| anyhow::anyhow!(e)),
      Err(_) => Err(anyhow::anyhow!("The client or server has timed out")),
    }
  }
}
