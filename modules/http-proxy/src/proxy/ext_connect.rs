//! Handling logic for extended CONNECT (HTTP/2, HTTP/3) requests.

use std::future::Future;

use ferron_http::{trace_context::current_event_trace_context, HttpContext};
use futures_util::TryStreamExt;
use http_body_util::{combinators::UnsyncBoxBody, BodyExt};

use crate::{connections::PooledConnection, types::error::ProxyError};

pub struct ExtendedConnectRequestBody {
    rx: tokio::sync::oneshot::Receiver<UnsyncBoxBody<bytes::Bytes, std::io::Error>>,
    inner: Option<UnsyncBoxBody<bytes::Bytes, std::io::Error>>,
}

impl ExtendedConnectRequestBody {
    #[inline]
    pub fn new() -> (
        tokio::sync::oneshot::Sender<UnsyncBoxBody<bytes::Bytes, std::io::Error>>,
        Self,
    ) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let body = Self { rx, inner: None };
        (tx, body)
    }
}

impl hyper::body::Body for ExtendedConnectRequestBody {
    type Data = bytes::Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        loop {
            if let Some(inner) = this.inner.as_mut() {
                return std::pin::Pin::new(inner).poll_frame(cx);
            }

            match std::pin::Pin::new(&mut this.rx).poll(cx) {
                std::task::Poll::Ready(Ok(body)) => {
                    this.inner = Some(body);
                }
                std::task::Poll::Ready(Err(_)) => {
                    return std::task::Poll::Ready(Some(Err(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "upgraded request stream error",
                    ))));
                }
                std::task::Poll::Pending => {
                    return std::task::Poll::Pending;
                }
            }
        }
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        self.inner.as_ref().map_or_default(|i| i.size_hint())
    }

    fn is_end_stream(&self) -> bool {
        self.inner.as_ref().is_some_and(|i| i.is_end_stream())
    }
}

/// Handle HTTP/2 extended CONNECT (RFC 8441) upgrade. This is similar to `handle_upgrade`
#[inline]
pub async fn handle_extended_connect(
    upgraded_body_tx: hyper::body::Incoming,
    upgraded_body_rx: tokio::sync::oneshot::Sender<UnsyncBoxBody<bytes::Bytes, std::io::Error>>,
    req_extensions: http::Extensions,
    ctx: &mut HttpContext,
    mut item: PooledConnection,
    is_unix: bool,
) -> Result<(), ProxyError> {
    let mut upgrade_request = http::Request::new(http_body_util::Empty::<bytes::Bytes>::new());
    *upgrade_request.extensions_mut() = req_extensions;

    let events = ctx.events.clone();
    let upstream = item.key().map(|k| &*k.0);
    let trace_context = current_event_trace_context(ctx);
    let mut upstream_attrs = Vec::with_capacity(2);
    if let Some(backend) = upstream {
        upstream_attrs.push((
            "ferron.proxy.backend_url",
            ferron_observability::LogAttributeValue::String(backend.proxy_to.clone()),
        ));
        if let Some(ref unix_path) = backend.proxy_unix {
            upstream_attrs.push((
                "ferron.proxy.backend_unix_path",
                ferron_observability::LogAttributeValue::String(unix_path.clone()),
            ));
        }
    }

    // Take the inner value to prevent Drop from returning the connection to pool earlier.
    let wrapper = item.inner_mut().take();

    let upgrade_future = zincio_http::prepare_upgrade(&mut upgrade_request);
    zincio::spawn_detached(async move {
        let _return_info = wrapper
            .filter(|w| w.multiplexable())
            .map(|srw| crate::send_request::PoolReturnInfo::from_item(item, srw, is_unix));
        if let Some(upgraded_future) = upgrade_future {
            match upgraded_future.await {
                Some(upgraded_client) => {
                    let mut backend_tx = tokio_util::io::StreamReader::new(
                        upgraded_body_tx
                            .into_data_stream()
                            .map_err(std::io::Error::other),
                    );
                    let (client_rx, mut client_tx) = tokio::io::split(upgraded_client);
                    let backend_rx = http_body_util::StreamBody::new(
                        tokio_util::io::ReaderStream::new(client_rx)
                            .map_ok(hyper::body::Frame::data),
                    )
                    .boxed_unsync();

                    let upgrade_fut = async {
                        upgraded_body_rx.send(backend_rx).map_err(|_| {
                            std::io::Error::new(
                                std::io::ErrorKind::BrokenPipe,
                                "upgraded request stream error",
                            )
                        })?;
                        tokio::io::copy(&mut backend_tx, &mut client_tx).await
                    };

                    if let Err(e) = upgrade_fut.await {
                        let mut upstream_attrs = upstream_attrs;
                        upstream_attrs.push((
                            "error.message",
                            ferron_observability::LogAttributeValue::String(e.to_string()),
                        ));
                        events.emit(ferron_observability::Event::Log(
                            ferron_observability::LogEvent {
                                level: ferron_observability::LogLevel::Warn,
                                message: format!(
                                    "Reverse proxy: HTTP upgrade tunneling failed: {}",
                                    e
                                ),
                                summary: "Reverse proxy: HTTP upgrade tunneling failed".into(),
                                target: "ferron-http-proxy",
                                attributes: upstream_attrs,
                                trace_context,
                            },
                        ));
                    }
                    // Connection may bereturned to pool for upgrades,
                    // since it could be HTTP/2 not HTTP/1.x
                }
                None => {
                    events.emit(ferron_observability::Event::Log(
                        ferron_observability::LogEvent {
                            level: ferron_observability::LogLevel::Warn,
                            message: "Reverse proxy: backend HTTP upgrade failed".to_string(),
                            summary: "Reverse proxy: backend HTTP upgrade failed".into(),
                            target: "ferron-http-proxy",
                            attributes: upstream_attrs,
                            trace_context,
                        },
                    ));
                }
            }
        }
    });

    Ok(())
}
