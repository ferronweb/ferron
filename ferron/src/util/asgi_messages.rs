use std::path::PathBuf;

use http::{request::Parts, Uri};
use pyo3::{prelude::*, types::PyDict};

use crate::ferron_common::{ErrorLogger, SocketData};

pub enum IncomingAsgiMessage {
  Init(AsgiInitData),
  Message(IncomingAsgiMessageInner),
}

pub enum OutgoingAsgiMessage {
  Message(OutgoingAsgiMessageInner),
  Finished,
  Error(PyErr),
}

pub enum AsgiInitData {
  Lifespan,
  Http(AsgiHttpInitData),
  Websocket(AsgiWebsocketInitData),
}

pub struct AsgiHttpInitData {
  pub hyper_request_parts: Parts,
  pub original_request_uri: Option<Uri>,
  pub socket_data: SocketData,
  #[allow(dead_code)]
  pub error_logger: ErrorLogger,
  pub wwwroot: PathBuf,
  pub execute_pathbuf: PathBuf,
}

pub struct AsgiWebsocketInitData {
  pub uri: Uri,
  pub socket_data: SocketData,
  #[allow(dead_code)]
  pub error_logger: ErrorLogger,
  pub wwwroot: PathBuf,
  pub execute_pathbuf: PathBuf,
}

pub enum IncomingAsgiMessageInner {
  LifespanStartup,
  LifespanShutdown,
  HttpRequest(AsgiHttpBody),
  HttpDisconnect,
  WebsocketConnect,
  WebsocketReceive(AsgiWebsocketMessage),
  WebsocketDisconnect(AsgiWebsocketClose),
}

pub enum OutgoingAsgiMessageInner {
  LifespanStartupComplete,
  #[allow(dead_code)]
  LifespanStartupFailed(LifespanFailed),
  LifespanShutdownComplete,
  #[allow(dead_code)]
  LifespanShutdownFailed(LifespanFailed),
  HttpResponseStart(AsgiHttpResponseStart),
  HttpResponseBody(AsgiHttpBody),
  HttpResponseTrailers(AsgiHttpTrailers),
  #[allow(dead_code)]
  WebsocketAccept(AsgiWebsocketAccept),
  WebsocketSend(AsgiWebsocketMessage),
  #[allow(dead_code)]
  WebsocketClose(AsgiWebsocketClose),
  Unknown,
}

#[allow(dead_code)]
pub struct LifespanFailed {
  pub message: String,
}

pub struct AsgiHttpBody {
  pub body: Vec<u8>,
  pub more_body: bool,
}

pub struct AsgiHttpResponseStart {
  pub status: u16,
  pub headers: Vec<(Vec<u8>, Vec<u8>)>,
  pub trailers: bool,
}

pub struct AsgiHttpTrailers {
  pub headers: Vec<(Vec<u8>, Vec<u8>)>,
  pub more_trailers: bool,
}

#[allow(dead_code)]
pub struct AsgiWebsocketAccept {
  pub subprotocol: Option<String>,
  pub headers: Vec<(Vec<u8>, Vec<u8>)>,
}

pub struct AsgiWebsocketClose {
  pub code: u16,
  pub reason: String,
}

pub struct AsgiWebsocketMessage {
  pub bytes: Option<Vec<u8>>,
  pub text: Option<String>,
}

pub fn asgi_event_to_outgoing_struct(
  event: Bound<'_, PyDict>,
) -> PyResult<OutgoingAsgiMessageInner> {
  let event_type = match event.get_item("type")? {
    Some(event_type) => event_type.extract::<String>()?,
    None => Err(anyhow::anyhow!("Cannot send event with no type specified"))?,
  };

  match event_type.as_str() {
    "lifespan.startup.complete" => Ok(OutgoingAsgiMessageInner::LifespanStartupComplete),
    "lifespan.shutdown.complete" => Ok(OutgoingAsgiMessageInner::LifespanShutdownComplete),
    "lifespan.startup.failed" => Ok(OutgoingAsgiMessageInner::LifespanStartupFailed(
      LifespanFailed {
        message: event
          .get_item("message")?
          .map_or(Ok("".to_string()), |x| x.extract())?,
      },
    )),
    "lifespan.shutdown.failed" => Ok(OutgoingAsgiMessageInner::LifespanShutdownFailed(
      LifespanFailed {
        message: event
          .get_item("message")?
          .map_or(Ok("".to_string()), |x| x.extract())?,
      },
    )),
    "http.response.start" => Ok(OutgoingAsgiMessageInner::HttpResponseStart(
      AsgiHttpResponseStart {
        status: match event.get_item("status")?.map(|x| x.extract()) {
          Some(status) => status?,
          None => Err(anyhow::anyhow!("The HTTP response must have a status code"))?,
        },
        headers: event.get_item("headers")?.map_or(
          Ok(Ok(Vec::new())),
          |header_list_py: Bound<'_, PyAny>| {
            header_list_py
              .extract::<Vec<Vec<Vec<u8>>>>()
              .map(|header_list| {
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
          },
        )??,
        trailers: event
          .get_item("trailers")?
          .map_or(Ok(false), |x| x.extract())?,
      },
    )),
    "http.response.body" => Ok(OutgoingAsgiMessageInner::HttpResponseBody(AsgiHttpBody {
      body: event
        .get_item("body")?
        .map_or(Ok(b"".to_vec()), |x| x.extract())?,
      more_body: event
        .get_item("more_body")?
        .map_or(Ok(false), |x| x.extract())?,
    })),
    "http.response.trailers" => Ok(OutgoingAsgiMessageInner::HttpResponseTrailers(
      AsgiHttpTrailers {
        headers: event.get_item("headers")?.map_or(
          Ok(Ok(Vec::new())),
          |header_list_py: Bound<'_, PyAny>| {
            header_list_py
              .extract::<Vec<Vec<Vec<u8>>>>()
              .map(|header_list| {
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
          },
        )??,
        more_trailers: event
          .get_item("more_trailers")?
          .map_or(Ok(false), |x| x.extract())?,
      },
    )),
    "websocket.accept" => Ok(OutgoingAsgiMessageInner::WebsocketAccept(
      AsgiWebsocketAccept {
        subprotocol: event
          .get_item("subprotocol")?
          .map_or(Ok(None), |x| x.extract())?,
        headers: event.get_item("headers")?.map_or(
          Ok(Ok(Vec::new())),
          |header_list_py: Bound<'_, PyAny>| {
            header_list_py
              .extract::<Vec<Vec<Vec<u8>>>>()
              .map(|header_list| {
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
          },
        )??,
      },
    )),
    "websocket.close" => Ok(OutgoingAsgiMessageInner::WebsocketClose(
      AsgiWebsocketClose {
        code: event.get_item("code")?.map_or(Ok(1000), |x| x.extract())?,
        reason: event
          .get_item("reason")?
          .map_or(Ok(None), |x| x.extract())?
          .unwrap_or("".to_string()),
      },
    )),
    "websocket.send" => Ok(OutgoingAsgiMessageInner::WebsocketSend(
      AsgiWebsocketMessage {
        bytes: event.get_item("bytes")?.map_or(Ok(None), |x| x.extract())?,
        text: event.get_item("text")?.map_or(Ok(None), |x| x.extract())?,
      },
    )),
    _ => Ok(OutgoingAsgiMessageInner::Unknown),
  }
}

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
        event.set_item("type", "lifespan.shutdown")?;
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
