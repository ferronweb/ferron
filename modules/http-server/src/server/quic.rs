use std::collections::HashMap;
use std::fmt::Debug;
use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use std::time::Instant;

use ferron_core::pipeline::Pipeline;
use ferron_http::{HttpContext, HttpErrorContext, HttpFileContext};
use ferron_observability::CompositeEventSink;
use quinn::{AsyncTimer, AsyncUdpSocket, Incoming, Runtime};
use send_wrapper::SendWrapper;
use tokio_util::sync::CancellationToken;
use vibeio::time::Sleep;
use vibeio_http::{Http3, Http3Options, HttpProtocol};

use crate::config::ThreeStageResolver;
use crate::server::common::{
    build_request_handler, emit_error, normalize_host_for_lookup, resolve_observability_sink,
    ConfigArcSwap, ConnectionCountGuard, NoCertResolver, ObservabilityProviderEntry,
    RequestHandlerState,
};
use crate::server::sni::CustomSniResolver;
use crate::server::tls_resolve::RadixTree;

#[derive(Default)]
pub struct QuicTlsSniResolvers {
    pub host: HashMap<IpAddr, CustomSniResolver>,
    pub fallback: Option<CustomSniResolver>,
}

#[derive(Default)]
pub struct QuicTlsResolver {
    host: HashMap<IpAddr, Arc<quinn::ServerConfig>>,
    fallback: Option<Arc<quinn::ServerConfig>>,
}

impl QuicTlsResolver {
    #[inline]
    pub fn resolve(&self, ip: &IpAddr) -> Option<Arc<quinn::ServerConfig>> {
        self.host.get(ip).cloned().or_else(|| self.fallback.clone())
    }
}

impl TryFrom<QuicTlsSniResolvers> for QuicTlsResolver {
    type Error = Box<dyn std::error::Error>;

    #[inline]
    fn try_from(value: QuicTlsSniResolvers) -> Result<Self, Self::Error> {
        let host = value
            .host
            .into_iter()
            .map(|(ip, resolver)| {
                let mut rustls_config = rustls::ServerConfig::builder_with_provider(Arc::new(
                    rustls::crypto::aws_lc_rs::default_provider(),
                ))
                .with_safe_default_protocol_versions()?
                .with_no_client_auth()
                .with_cert_resolver(Arc::new(resolver));
                rustls_config.max_early_data_size = u32::MAX;
                rustls_config.alpn_protocols.insert(0, b"h3-29".to_vec());
                rustls_config.alpn_protocols.insert(0, b"h3".to_vec());

                let quinn_crypto_config: quinn::crypto::rustls::QuicServerConfig =
                    rustls_config.try_into()?;
                let server_config = quinn::ServerConfig::with_crypto(Arc::new(quinn_crypto_config));
                Ok((ip, Arc::new(server_config)))
            })
            .collect::<Result<HashMap<_, _>, Self::Error>>()?;

        let fallback = if let Some(resolver) = value.fallback {
            let mut rustls_config = rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::aws_lc_rs::default_provider(),
            ))
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(resolver));
            rustls_config.max_early_data_size = u32::MAX;
            rustls_config.alpn_protocols.insert(0, b"h3-29".to_vec());
            rustls_config.alpn_protocols.insert(0, b"h3".to_vec());

            let quinn_crypto_config: quinn::crypto::rustls::QuicServerConfig =
                rustls_config.try_into()?;
            Some(Arc::new(quinn::ServerConfig::with_crypto(Arc::new(
                quinn_crypto_config,
            ))))
        } else {
            None
        };

        Ok(Self { host, fallback })
    }
}

/// A timer for Quinn that utilizes `vibeio`'s timer.
struct CustomAsyncTimer {
    inner: SendWrapper<Pin<Box<Sleep>>>,
}

impl AsyncTimer for CustomAsyncTimer {
    fn reset(mut self: Pin<&mut Self>, t: Instant) {
        (*self.inner).as_mut().reset(t)
    }

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<()> {
        (*self.inner).as_mut().poll(cx)
    }
}

impl Debug for CustomAsyncTimer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustomAsyncTimer").finish()
    }
}

/// A runtime for Quinn that utilizes Tokio, if under Tokio runtime, and otherwise Monoio with async_io.
#[derive(Debug)]
struct EnterTokioRuntime;

impl Runtime for EnterTokioRuntime {
    fn new_timer(&self, t: Instant) -> Pin<Box<dyn AsyncTimer>> {
        if tokio::runtime::Handle::try_current().is_ok() {
            Box::pin(tokio::time::sleep_until(t.into()))
        } else {
            Box::pin(CustomAsyncTimer {
                inner: SendWrapper::new(Box::pin(vibeio::time::sleep_until(t))),
            })
        }
    }

    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(future);
        } else {
            vibeio::spawn(future);
        }
    }

    fn wrap_udp_socket(&self, sock: std::net::UdpSocket) -> io::Result<Arc<dyn AsyncUdpSocket>> {
        quinn::TokioRuntime::wrap_udp_socket(&quinn::TokioRuntime, sock)
    }
}

pub struct QuicListenerHandle {
    cancel_token: Arc<CancellationToken>,
}

impl QuicListenerHandle {
    pub fn new(
        address: SocketAddr,
        config: ConfigArcSwap,
        runtime: &mut ferron_core::runtime::Runtime,
    ) -> Result<Self, std::io::Error> {
        let cancel_token = Arc::new(CancellationToken::new());

        let (tx, rx) = async_channel::unbounded::<Incoming>();
        let (listen_error_tx, listen_error_rx) = oneshot::channel::<Option<io::Error>>();
        let config_clone = config.clone();
        let cancel_token_clone = cancel_token.clone();
        let cancel_token_clone2 = cancel_token.clone();

        std::thread::spawn(move || {
            let tokio_runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    listen_error_tx
                        .send(Some(io::Error::other(format!(
                            "Failed to create Tokio runtime for QUIC listener: {error}"
                        ))))
                        .unwrap_or_default();
                    return;
                }
            };
            tokio_runtime.block_on(async move {
                let rustls_server_config = (match rustls::ServerConfig::builder_with_provider(
                    Arc::new(rustls::crypto::aws_lc_rs::default_provider()),
                )
                .with_safe_default_protocol_versions()
                {
                    Ok(builder) => builder,
                    Err(error) => {
                        listen_error_tx
                            .send(Some(io::Error::other(format!(
                                "Failed to create Rustls ServerConfig builder: {error}"
                            ))))
                            .unwrap_or_default();
                        return;
                    }
                })
                .with_no_client_auth()
                .with_cert_resolver(Arc::new(NoCertResolver));
                let quinn_crypto_config: quinn::crypto::rustls::QuicServerConfig =
                    match rustls_server_config.try_into() {
                        Ok(config) => config,
                        Err(error) => {
                            listen_error_tx
                                .send(Some(io::Error::other(format!(
                                    "Failed to create Quinn crypto config: {error}"
                                ))))
                                .unwrap_or_default();
                            return;
                        }
                    };
                let server_config = quinn::ServerConfig::with_crypto(Arc::new(quinn_crypto_config));

                let udp_socket;
                let mut tries: u64 = 0;
                loop {
                    if let Ok(socket) = bind_udp_socket(address) {
                        udp_socket = socket;
                        break;
                    }
                    tries += 1;
                    let duration = Duration::from_millis(1000);
                    if tries >= 10 {
                        ferron_core::log_warn!("HTTP/3 port is used at try #{tries}, skipping...");
                        listen_error_tx.send(None).unwrap_or_default();
                        return;
                    }
                    ferron_core::log_warn!(
                        "HTTP/3 port is used at try #{tries}, retrying in {duration:?}..."
                    );
                    if cancel_token_clone.is_cancelled() {
                        return;
                    }
                    tokio::time::sleep(duration).await;
                }
                let endpoint = match quinn::Endpoint::new(
                    quinn::EndpointConfig::default(),
                    Some(server_config),
                    udp_socket,
                    Arc::new(EnterTokioRuntime),
                ) {
                    Ok(endpoint) => endpoint,
                    Err(err) => {
                        listen_error_tx
                            .send(Some(std::io::Error::other(format!(
                                "Cannot listen to HTTP/3 port: {err}"
                            ))))
                            .unwrap_or_default();
                        return;
                    }
                };
                ferron_core::log_info!("HTTP/3 server listening on {address}");
                listen_error_tx.send(None).unwrap_or_default();

                while let Some(incoming) = tokio::select! {
                    incoming = endpoint.accept() => incoming,
                    _ = cancel_token_clone.cancelled() => None,
                } {
                    if tx.send(incoming).await.is_err() {
                        break;
                    }
                }

                endpoint.wait_idle().await;
            });
        });

        runtime.spawn_primary_task(move || {
            let rx = rx.clone();
            let cancel_token = cancel_token_clone2.clone();
            let config = config_clone.clone();
            Box::pin(async move {
                while let Some(incoming) = tokio::select! {
                    incoming = rx.recv() => incoming.ok(),
                    _ = cancel_token.cancelled() => None,
                } {
                    let config = config.clone();
                    let connection_cancel_token = cancel_token.clone();

                    vibeio::spawn(async move {
                        let _conn_guard = ConnectionCountGuard::new();

                        let server_config = config.load_full();

                        let local_ip = incoming.local_ip().unwrap_or(address.ip());
                        let local_addr = SocketAddr::new(local_ip, address.port());

                        let quic_resolver =
                            server_config.quic_tls_resolver.clone().unwrap_or_default();
                        let tls_config = quic_resolver.resolve(&address.ip());
                        let ip_observability = resolve_observability_sink(
                            &server_config.observability_resolver,
                            Some(local_addr.ip()),
                            None,
                            &CompositeEventSink::new(vec![]),
                        );

                        let connection = match accept_quic(incoming, tls_config.clone()).await {
                            Ok(conn) => conn,
                            Err(error) => {
                                emit_error(
                                    &ip_observability,
                                    format!("Failed to accept HTTP/3 connection: {error}"),
                                );
                                return;
                            }
                        };

                        let remote_addr = connection.remote_address();

                        let sni = connection.handshake_data().and_then(|data| {
                            data.downcast_ref::<quinn::crypto::rustls::HandshakeData>()
                                .and_then(|data| data.server_name.to_owned())
                        });
                        let hinted_hostname = sni.as_deref().and_then(normalize_host_for_lookup);

                        let tls_observability = resolve_observability_sink(
                            &server_config.observability_resolver,
                            Some(local_addr.ip()),
                            hinted_hostname.as_deref(),
                            &ip_observability,
                        );

                        handle_http3_connection(
                            connection,
                            remote_addr,
                            server_config.pipeline.clone(),
                            server_config.file_pipeline.clone(),
                            server_config.error_pipeline.clone(),
                            server_config.config_resolver.clone(),
                            local_addr,
                            hinted_hostname,
                            tls_config.is_some(),
                            server_config.https_port,
                            server_config.observability_resolver.clone(),
                            tls_observability,
                            (*connection_cancel_token).clone(),
                            server_config.reload_token.clone(),
                        )
                        .await;
                    });
                }
            })
        });

        if let Some(error) = listen_error_rx.recv().unwrap_or(None) {
            return Err(error);
        }

        Ok(Self { cancel_token })
    }

    #[inline]
    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }
}

#[inline]
async fn accept_quic(
    incoming: Incoming,
    server_config: Option<Arc<quinn::ServerConfig>>,
) -> Result<quinn::Connection, Box<dyn std::error::Error>> {
    if let Some(server_config) = server_config {
        Ok(incoming.accept_with(server_config)?.await?)
    } else {
        Ok(incoming.accept()?.await?)
    }
}

#[inline]
fn bind_udp_socket(address: SocketAddr) -> io::Result<std::net::UdpSocket> {
    // Create a new socket
    let listener_socket2 = socket2::Socket::new(
        if address.is_ipv6() {
            socket2::Domain::IPV6
        } else {
            socket2::Domain::IPV4
        },
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;

    // Set socket options
    if address.is_ipv6() {
        listener_socket2.set_only_v6(false).unwrap_or_default();
    }

    // Bind the socket to the address
    listener_socket2.bind(&address.into())?;

    // Wrap the socket into a UdpSocket
    Ok(listener_socket2.into())
}

#[allow(clippy::too_many_arguments)]
async fn handle_http3_connection(
    conn: quinn::Connection,
    remote_address: SocketAddr,
    pipeline: Arc<Pipeline<HttpContext>>,
    file_pipeline: Arc<Pipeline<HttpFileContext>>,
    error_pipeline: Arc<Pipeline<HttpErrorContext>>,
    config_resolver: Arc<ThreeStageResolver>,
    local_address: SocketAddr,
    hinted_hostname: Option<String>,
    encrypted: bool,
    https_port: Option<u16>,
    observability_resolver: Arc<RadixTree<Vec<ObservabilityProviderEntry>>>,
    connection_observability: CompositeEventSink,
    shutdown_token: CancellationToken,
    reload_token: CancellationToken,
) {
    let graceful_shutdown = CancellationToken::new();
    let handler_state = Arc::new(RequestHandlerState {
        pipeline,
        file_pipeline,
        error_pipeline,
        config_resolver,
        connection_observability,
        observability_resolver,
        local_address,
        remote_address,
        hinted_hostname,
        encrypted,
        https_port,
        http3_alt_svc: false,
    });
    let mut connection_future = Box::pin(
        Http3::new(h3_quinn::Connection::new(conn), Http3Options::default())
            .graceful_shutdown_token(graceful_shutdown.clone())
            .handle(build_request_handler(handler_state.clone())),
    );
    let connection_result = tokio::select! {
        result = &mut connection_future => result,
        _ = shutdown_token.cancelled() => {
            graceful_shutdown.cancel();
            connection_future.await
        }
        _ = reload_token.cancelled() => {
            graceful_shutdown.cancel();
            connection_future.await
        }
    };

    if let Err(error) = connection_result {
        emit_error(
            &handler_state.connection_observability,
            format!("HTTP/3 connection error: {error}"),
        );
    }
}
