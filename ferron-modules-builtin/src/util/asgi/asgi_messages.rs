use std::path::PathBuf;

use hyper::http::request::Parts;
use pyo3::{prelude::*, types::PyDict};

use ferron_common::logging::ErrorLogger;
use ferron_common::modules::SocketData;

/// Represents incoming ASGI messages from the Python application.
///
/// This enum handles both initialization data and regular messages during
/// the ASGI application lifecycle.
pub enum IncomingAsgiMessage {
  /// Initialization data for starting an ASGI connection
  Init(AsgiInitData),
  /// Regular message during the connection lifecycle
  Message(IncomingAsgiMessageInner),
}

/// Represents outgoing ASGI messages sent to the Python application.
///
/// This enum handles messages being sent from Rust to the Python ASGI app,
/// including regular messages, completion signals, and errors.
pub enum OutgoingAsgiMessage {
  /// Regular message being sent to the ASGI application
  Message(OutgoingAsgiMessageInner),
  /// Indicates the connection has finished successfully
  Finished,
  /// An error occurred during processing
  Error(PyErr),
}

/// Initialization data for different types of ASGI connections.
///
/// Contains the initial setup information needed to establish
/// different types of ASGI protocol connections.
pub enum AsgiInitData {
  /// Lifespan protocol initialization (application startup/shutdown)
  Lifespan,
  /// HTTP protocol initialization with request data
  Http(AsgiHttpInitData),
  /// WebSocket protocol initialization with connection data
  Websocket(AsgiWebsocketInitData),
}

/// Initialization data for HTTP ASGI connections.
///
/// Contains all the necessary information to process an HTTP request
/// through the ASGI protocol, including request metadata and server configuration.
pub struct AsgiHttpInitData {
  /// Parsed HTTP request parts from Hyper
  pub request_parts: Parts,
  /// Socket connection information
  pub socket_data: SocketData,
  /// Error logging interface
  #[allow(dead_code)]
  pub error_logger: ErrorLogger,
  /// Web root directory path
  pub wwwroot: PathBuf,
  /// Path to the executable directory
  pub execute_pathbuf: PathBuf,
}

/// Initialization data for WebSocket ASGI connections.
///
/// Contains all the necessary information to establish a WebSocket connection
/// through the ASGI protocol, including connection metadata and server configuration.
pub struct AsgiWebsocketInitData {
  /// Parsed HTTP request parts from Hyper
  pub request_parts: Parts,
  /// Socket connection information
  pub socket_data: SocketData,
  /// Error logging interface
  #[allow(dead_code)]
  pub error_logger: ErrorLogger,
  /// Web root directory path
  pub wwwroot: PathBuf,
  /// Path to the executable directory
  pub execute_pathbuf: PathBuf,
}

/// Specific types of incoming ASGI messages during connection lifecycle.
///
/// Represents the various message types that can be received from
/// a Python ASGI application during different protocol interactions.
pub enum IncomingAsgiMessageInner {
  /// Application startup event for lifespan protocol
  LifespanStartup,
  /// Application shutdown event for lifespan protocol
  LifespanShutdown,
  /// HTTP request body data
  HttpRequest(AsgiHttpBody),
  /// HTTP client disconnection
  HttpDisconnect,
  /// WebSocket connection establishment
  WebsocketConnect,
  /// Incoming WebSocket message from client
  WebsocketReceive(AsgiWebsocketMessage),
  /// WebSocket connection termination
  WebsocketDisconnect(AsgiWebsocketClose),
}

/// Specific types of outgoing ASGI messages sent to Python applications.
///
/// Represents the various message types that can be sent to
/// a Python ASGI application in response to different protocol events.
pub enum OutgoingAsgiMessageInner {
  /// Successful application startup completion
  LifespanStartupComplete,
  /// Failed application startup with error details
  #[allow(dead_code)]
  LifespanStartupFailed(LifespanFailed),
  /// Successful application shutdown completion
  LifespanShutdownComplete,
  /// Failed application shutdown with error details
  #[allow(dead_code)]
  LifespanShutdownFailed(LifespanFailed),
  /// HTTP response headers and status
  HttpResponseStart(AsgiHttpResponseStart),
  /// HTTP response body data
  HttpResponseBody(AsgiHttpBody),
  /// HTTP response trailers
  HttpResponseTrailers(AsgiHttpTrailers),
  /// WebSocket connection acceptance
  #[allow(dead_code)]
  WebsocketAccept(AsgiWebsocketAccept),
  /// Outgoing WebSocket message to client
  WebsocketSend(AsgiWebsocketMessage),
  /// WebSocket connection closure
  #[allow(dead_code)]
  WebsocketClose(AsgiWebsocketClose),
  /// Unknown or unhandled message type
  Unknown,
}

/// Error information for failed lifespan events.
///
/// Contains details about why a lifespan startup or shutdown event failed.
#[allow(dead_code)]
pub struct LifespanFailed {
  /// Human-readable error message describing the failure
  pub message: String,
}

/// HTTP request or response body data for ASGI protocol.
///
/// Represents a chunk of HTTP body data, which may be sent in multiple
/// parts for streaming or large payloads.
pub struct AsgiHttpBody {
  /// Raw body data as bytes
  pub body: Vec<u8>,
  /// Whether more body chunks will follow
  pub more_body: bool,
}

/// HTTP response start information for ASGI protocol.
///
/// Contains the initial HTTP response data including status code and headers,
/// sent before any response body data.
pub struct AsgiHttpResponseStart {
  /// HTTP status code (e.g., 200, 404, 500)
  pub status: u16,
  /// HTTP headers as byte pairs (name, value)
  pub headers: Vec<(Vec<u8>, Vec<u8>)>,
  /// Whether HTTP trailers will be sent after the body
  pub trailers: bool,
}

/// HTTP trailers sent after the response body for ASGI protocol.
///
/// Represents additional headers that are sent after the response body,
/// useful for metadata that's only available after processing the response.
pub struct AsgiHttpTrailers {
  /// Trailer headers as byte pairs (name, value)
  pub headers: Vec<(Vec<u8>, Vec<u8>)>,
  /// Whether more trailer chunks will follow
  pub more_trailers: bool,
}

/// WebSocket connection acceptance data for ASGI protocol.
///
/// Contains the response data for accepting a WebSocket connection,
/// including optional subprotocol selection and additional headers.
#[allow(dead_code)]
pub struct AsgiWebsocketAccept {
  /// Selected WebSocket subprotocol, if any
  pub subprotocol: Option<String>,
  /// Additional headers to send with the acceptance
  pub headers: Vec<(Vec<u8>, Vec<u8>)>,
}

/// WebSocket connection closure information for ASGI protocol.
///
/// Contains the details about why and how a WebSocket connection is being closed,
/// following the WebSocket protocol specifications.
pub struct AsgiWebsocketClose {
  /// WebSocket close code (e.g., 1000 for normal closure)
  pub code: u16,
  /// Human-readable reason for closure
  pub reason: String,
}

/// WebSocket message data for ASGI protocol.
///
/// Represents a WebSocket message that can contain either binary data or text.
/// Only one of the fields should be populated for any given message.
pub struct AsgiWebsocketMessage {
  /// Binary message data, if this is a binary message
  pub bytes: Option<Vec<u8>>,
  /// Text message data, if this is a text message
  pub text: Option<String>,
}

/// Converts a Python ASGI event dictionary to a Rust outgoing message struct.
///
/// This function parses a Python dictionary representing an ASGI event and
/// converts it to the corresponding Rust enum variant for further processing.
///
/// # Arguments
///
/// * `event` - A Python dictionary containing the ASGI event data
///
/// # Returns
///
/// Returns a `PyResult<OutgoingAsgiMessageInner>` containing the parsed message
/// or an error if the event format is invalid.
///
/// # Errors
///
/// Returns an error if:
/// - The event has no "type" field
/// - Required fields for specific event types are missing
/// - Field values cannot be extracted to the expected Rust types
pub fn asgi_event_to_outgoing_struct(event: Bound<'_, PyDict>) -> PyResult<OutgoingAsgiMessageInner> {
  let event_type = match event.get_item("type")? {
    Some(event_type) => event_type.extract::<String>()?,
    None => Err(anyhow::anyhow!("Cannot send event with no type specified"))?,
  };

  match event_type.as_str() {
    "lifespan.startup.complete" => Ok(OutgoingAsgiMessageInner::LifespanStartupComplete),
    "lifespan.shutdown.complete" => Ok(OutgoingAsgiMessageInner::LifespanShutdownComplete),
    "lifespan.startup.failed" => Ok(OutgoingAsgiMessageInner::LifespanStartupFailed(LifespanFailed {
      message: event.get_item("message")?.map_or(Ok("".to_string()), |x| x.extract())?,
    })),
    "lifespan.shutdown.failed" => Ok(OutgoingAsgiMessageInner::LifespanShutdownFailed(LifespanFailed {
      message: event.get_item("message")?.map_or(Ok("".to_string()), |x| x.extract())?,
    })),
    "http.response.start" => Ok(OutgoingAsgiMessageInner::HttpResponseStart(AsgiHttpResponseStart {
      status: match event.get_item("status")?.map(|x| x.extract()) {
        Some(status) => status?,
        None => Err(anyhow::anyhow!("The HTTP response must have a status code"))?,
      },
      headers: event
        .get_item("headers")?
        .map_or(Ok(Ok(Vec::new())), |header_list_py: Bound<'_, PyAny>| {
          header_list_py.extract::<Vec<Vec<Vec<u8>>>>().map(|header_list| {
            let mut new_header_list = Vec::new();
            for header in header_list {
              if header.len() != 2 {
                return Err(anyhow::anyhow!("Headers must be two-item iterables"));
              }
              let mut header_iter = header.into_iter();
              new_header_list.push((
                header_iter.next().unwrap_or(b"".to_vec()),
                header_iter.next().unwrap_or(b"".to_vec()),
              ));
            }
            Ok(new_header_list)
          })
        })??,
      trailers: event.get_item("trailers")?.map_or(Ok(false), |x| x.extract())?,
    })),
    "http.response.body" => Ok(OutgoingAsgiMessageInner::HttpResponseBody(AsgiHttpBody {
      body: event.get_item("body")?.map_or(Ok(b"".to_vec()), |x| x.extract())?,
      more_body: event.get_item("more_body")?.map_or(Ok(false), |x| x.extract())?,
    })),
    "http.response.trailers" => Ok(OutgoingAsgiMessageInner::HttpResponseTrailers(AsgiHttpTrailers {
      headers: event
        .get_item("headers")?
        .map_or(Ok(Ok(Vec::new())), |header_list_py: Bound<'_, PyAny>| {
          header_list_py.extract::<Vec<Vec<Vec<u8>>>>().map(|header_list| {
            let mut new_header_list = Vec::new();
            for header in header_list {
              if header.len() != 2 {
                return Err(anyhow::anyhow!("Headers must be two-item iterables"));
              }
              let mut header_iter = header.into_iter();
              new_header_list.push((
                header_iter.next().unwrap_or(b"".to_vec()),
                header_iter.next().unwrap_or(b"".to_vec()),
              ));
            }
            Ok(new_header_list)
          })
        })??,
      more_trailers: event.get_item("more_trailers")?.map_or(Ok(false), |x| x.extract())?,
    })),
    "websocket.accept" => Ok(OutgoingAsgiMessageInner::WebsocketAccept(AsgiWebsocketAccept {
      subprotocol: event.get_item("subprotocol")?.map_or(Ok(None), |x| x.extract())?,
      headers: event
        .get_item("headers")?
        .map_or(Ok(Ok(Vec::new())), |header_list_py: Bound<'_, PyAny>| {
          header_list_py.extract::<Vec<Vec<Vec<u8>>>>().map(|header_list| {
            let mut new_header_list = Vec::new();
            for header in header_list {
              if header.len() != 2 {
                return Err(anyhow::anyhow!("Headers must be two-item iterables"));
              }
              let mut header_iter = header.into_iter();
              new_header_list.push((
                header_iter.next().unwrap_or(b"".to_vec()),
                header_iter.next().unwrap_or(b"".to_vec()),
              ));
            }
            Ok(new_header_list)
          })
        })??,
    })),
    "websocket.close" => Ok(OutgoingAsgiMessageInner::WebsocketClose(AsgiWebsocketClose {
      code: event.get_item("code")?.map_or(Ok(1000), |x| x.extract())?,
      reason: event
        .get_item("reason")?
        .map_or(Ok(None), |x| x.extract())?
        .unwrap_or("".to_string()),
    })),
    "websocket.send" => Ok(OutgoingAsgiMessageInner::WebsocketSend(AsgiWebsocketMessage {
      bytes: event.get_item("bytes")?.map_or(Ok(None), |x| x.extract())?,
      text: event.get_item("text")?.map_or(Ok(None), |x| x.extract())?,
    })),
    _ => Ok(OutgoingAsgiMessageInner::Unknown),
  }
}

/// Converts a Rust incoming message struct to a Python ASGI event dictionary.
///
/// This function takes a Rust enum representing an incoming ASGI message and
/// converts it to a Python dictionary that can be passed to the ASGI application.
///
/// # Arguments
///
/// * `incoming` - The incoming ASGI message to convert
///
/// # Returns
///
/// Returns a `PyResult<Py<PyDict>>` containing the Python dictionary representation
/// of the ASGI event, or an error if conversion fails.
///
/// # Errors
///
/// Returns an error if Python object creation or field setting fails.
pub fn incoming_struct_to_asgi_event(incoming: IncomingAsgiMessageInner) -> PyResult<Py<PyDict>> {
  Python::with_gil(move |py| -> PyResult<_> {
    let event = PyDict::new(py);

    match incoming {
      IncomingAsgiMessageInner::LifespanStartup => {
        event.set_item("type", "lifespan.startup")?;
      }
      IncomingAsgiMessageInner::LifespanShutdown => {
        event.set_item("type", "lifespan.shutdown")?;
      }
      IncomingAsgiMessageInner::HttpRequest(http_request) => {
        event.set_item("type", "http.request")?;
        event.set_item("body", http_request.body)?;
        event.set_item("more_body", http_request.more_body)?;
      }
      IncomingAsgiMessageInner::HttpDisconnect => {
        event.set_item("type", "http.disconnect")?;
      }
      IncomingAsgiMessageInner::WebsocketConnect => {
        event.set_item("type", "websocket.connect")?;
      }
      IncomingAsgiMessageInner::WebsocketDisconnect(websocket_close) => {
        event.set_item("type", "websocket.disconnect")?;
        event.set_item("code", websocket_close.code)?;
        event.set_item("reason", websocket_close.reason)?;
      }
      IncomingAsgiMessageInner::WebsocketReceive(websocket_message) => {
        event.set_item("type", "websocket.receive")?;
        event.set_item("bytes", websocket_message.bytes)?;
        event.set_item("text", websocket_message.text)?;
      }
    };

    Ok(event.unbind())
  })
}
