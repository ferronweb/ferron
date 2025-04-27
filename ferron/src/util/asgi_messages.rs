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

pub enum IncomingAsgiMessageInner {
  LifespanStartup,
  LifespanShutdown,
  HttpRequest(AsgiHttpBody),
  HttpDisconnect,
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
    };

    Ok(event.unbind())
  })
}
