use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use super::common::resolve_host_control_plane_metadata;
use super::common::resolve_host_control_plane_span_links;
use ferron_core::pipeline::Pipeline;
use ferron_http::{HttpContext, HttpErrorContext, HttpFileContext};
use ferron_observability::{
    CompositeEventSink, Event, LogAttributeValue, MetricAttributeValue, MetricEvent, MetricType,
    MetricValue,
};
use quinn::Incoming;
use tokio_util::sync::CancellationToken;
use zincio_http::{Http3, Http3Options, HttpProtocol};

use crate::config::ThreeStageResolver;
use crate::server::common::{
    build_request_handler, emit_error, normalize_host_for_lookup, resolve_http_connection_options,
    resolve_observability_sink, ConfigArcSwap, ConnectionCountGuard, HttpConnectionOptions,
    NoCertResolver, ObservabilityProviderEntry, RequestHandlerState,
};
use crate::server::sni::CustomSniResolver;
use crate::server::tls_resolve::RadixTree;
use crate::util::quinn_mt::{QuinnMTChannels, QuinnMTRuntime};

fn emit_connection_error_metric(
    observability: &CompositeEventSink,
    transport: &'static str,
    stage: &'static str,
) {
    observability.emit(Event::Metric(MetricEvent {
        name: "ferron.http.server.connection_errors",
        attributes: vec![
            (
                "network.transport",
                MetricAttributeValue::StaticStr(transport),
            ),
            (
                "ferron.connection.stage",
                MetricAttributeValue::StaticStr(stage),
            ),
        ],
        ty: MetricType::Counter,
        value: MetricValue::U64(1),
        unit: Some("{error}"),
        description: Some("Number of connection lifecycle errors by transport and stage."),
        trace_context: None,
    }));
}

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

pub struct QuicListenerHandle {
    cancel_token: Arc<CancellationToken>,
}

impl QuicListenerHandle {
    pub fn new(
        address: &[SocketAddr],
        config: ConfigArcSwap,
        runtime: &mut ferron_core::runtime::Runtime,
    ) -> Result<Self, std::io::Error> {
        let cancel_token = Arc::new(CancellationToken::new());
        for address in address {
            let udp_socket = bind_udp_socket(*address)?;
            ferron_core::log_info!("HTTP/3 server listening on {address}");
            // Fan a single UDP socket out to one independent quinn endpoint per
            // primary (per-CPU) thread. QuinnMTRuntime routes each datagram to the
            // endpoint that owns the connection; the CID generator below makes sure
            // the server connection IDs it issues route back to the same endpoint.
            // `spawn_primary_task_on` pins exactly one endpoint to each primary
            // thread, so the number of endpoints must match the thread count.
            let endpoint_count = runtime.primary_thread_count();
            // Server connection ID length; must match what the CID generator issues
            // and what the router expects when parsing short-header packets.
            let cid_len = 8;
            let channels = QuinnMTChannels::new(endpoint_count, cid_len);

            let (listen_error_tx, mut listen_error_rx) =
                tokio::sync::mpsc::unbounded_channel::<Option<io::Error>>();

            let config_clone = config.clone();
            let cancel_token_clone = cancel_token.clone();

            for id in 0..endpoint_count {
                let udp_socket = match udp_socket.try_clone() {
                    Ok(udp_socket) => udp_socket,
                    Err(error) => {
                        listen_error_tx
                            .send(Some(io::Error::other(format!(
                                "Failed to clone UDP socket for HTTP/3 endpoint {id}: {error}"
                            ))))
                            .unwrap_or_default();
                        continue;
                    }
                };
                let quinn_runtime = Arc::new(QuinnMTRuntime::new(
                    zincio_quinn::ZincioRuntime,
                    channels.clone(),
                    id,
                ));
                let config = config_clone.clone();
                let cancel_token = cancel_token_clone.clone();
                let listen_error_tx = listen_error_tx.clone();
                let address = *address;

                runtime.spawn_primary_task_on(id, move || {
                    let config = config.clone();
                    let cancel_token = cancel_token.clone();
                    let listen_error_tx = listen_error_tx.clone();
                    let udp_socket = udp_socket.try_clone();
                    let quinn_runtime = quinn_runtime.clone();
                    Box::pin(async move {
                        let rustls_server_config =
                            (match rustls::ServerConfig::builder_with_provider(Arc::new(
                                rustls::crypto::aws_lc_rs::default_provider(),
                            ))
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
                        let mut server_config =
                            quinn::ServerConfig::with_crypto(Arc::new(quinn_crypto_config));

                        // Use BBR to optimize for high-latency network links
                        let mut transport_config = quinn::TransportConfig::default();
                        transport_config.congestion_controller_factory(Arc::new(
                            quinn::congestion::BbrConfig::default(),
                        ));
                        // See https://blog.litespeedtech.com/2020/10/19/improve-performance-with-dplpmtud/
                        // Quinn already supports DPLPMTUD, but we set an upper bound to avoid fragmentation,
                        // and because LiteSpeed's benchmarks demonstrate faster timing with upper bound
                        // of 4096 vs. the default of 1472.
                        let mut mtu_config = quinn::MtuDiscoveryConfig::default();
                        mtu_config.upper_bound(4096);
                        transport_config.mtu_discovery_config(Some(mtu_config));

                        server_config.transport_config(Arc::new(transport_config));

                        let mut endpoint_config = quinn::EndpointConfig::default();
                        let quinn_runtime_cl = quinn_runtime.clone();
                        endpoint_config
                            .cid_generator(move || Box::new(quinn_runtime_cl.cid_generator()));

                        let endpoint = match udp_socket.and_then(|udp_socket| {
                            quinn::Endpoint::new(
                                endpoint_config,
                                Some(server_config),
                                udp_socket,
                                quinn_runtime,
                            )
                        }) {
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

                        run_endpoint(endpoint, config, cancel_token, address).await;
                    })
                });
            }

            listen_error_tx.send(None).unwrap_or_default();
            if let Some(error) = listen_error_rx.blocking_recv().unwrap_or(None) {
                return Err(error);
            }
        }

        Ok(Self { cancel_token })
    }

    #[inline]
    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }
}

#[inline]
async fn run_endpoint(
    endpoint: quinn::Endpoint,
    config: ConfigArcSwap,
    cancel_token: Arc<CancellationToken>,
    address: SocketAddr,
) {
    while let Some(incoming) = tokio::select! {
        incoming = endpoint.accept() => incoming,
        _ = cancel_token.cancelled() => None,
    } {
        let config = config.clone();
        let connection_cancel_token = cancel_token.clone();

        zincio::spawn_detached(async move {
            let _conn_guard = ConnectionCountGuard::new();

            let server_config = config.load_full();

            let local_ip = incoming.local_ip().unwrap_or(address.ip());
            let local_addr = SocketAddr::new(local_ip, address.port());

            let quic_resolver = server_config.quic_tls_resolver.clone().unwrap_or_default();
            let tls_config = quic_resolver.resolve(&address.ip());
            let ip_observability = resolve_observability_sink(
                &server_config.observability_resolver,
                Some(local_addr.ip()),
                None,
                &CompositeEventSink::with_sampler(
                    vec![],
                    Some(ferron_observability::sampler::TraceSampler::new(
                        &server_config.trace_sampling,
                    )),
                ),
            );

            let connection = match accept_quic(incoming, tls_config.clone()).await {
                Ok(conn) => conn,
                Err(error) => {
                    emit_error(
                        &ip_observability,
                        format!("Failed to accept HTTP/3 connection: {error}"),
                        vec![
                            (
                                "error.type",
                                LogAttributeValue::String("quic_accept_error".into()),
                            ),
                            (
                                "error.message",
                                LogAttributeValue::String(error.to_string()),
                            ),
                        ],
                    );
                    emit_connection_error_metric(&ip_observability, "quic", "http3_accept");
                    return;
                }
            };

            let remote_addr = connection.remote_address();

            let sni = connection.handshake_data().and_then(|data| {
                data.downcast_ref::<quinn::crypto::rustls::HandshakeData>()
                    .and_then(|data| data.server_name.to_owned())
            });
            let peer_identity: Option<Vec<rustls::pki_types::CertificateDer<'static>>> =
                connection.peer_identity().and_then(|data| {
                    data.downcast_ref::<Vec<rustls::pki_types::CertificateDer>>()
                        .and_then(|v| {
                            if v.is_empty() {
                                None
                            } else {
                                Some(v.to_owned())
                            }
                        })
                });
            let hinted_hostname = sni.as_deref().and_then(normalize_host_for_lookup);

            let tls_observability = resolve_observability_sink(
                &server_config.observability_resolver,
                Some(local_addr.ip()),
                hinted_hostname.as_deref(),
                &ip_observability,
            );

            let connection_options = resolve_http_connection_options(
                &server_config.http_connection_options_resolver,
                local_addr.ip(),
                hinted_hostname.as_deref(),
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
                connection_options,
                peer_identity,
            )
            .await;
        });
    }

    endpoint.wait_idle().await;
}

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
    let listener_socket2 = socket2::Socket::new(
        if address.is_ipv6() {
            socket2::Domain::IPV6
        } else {
            socket2::Domain::IPV4
        },
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;

    listener_socket2
        .set_reuse_address(!cfg!(windows))
        .unwrap_or_default();
    if address.is_ipv6() {
        listener_socket2.set_only_v6(false).unwrap_or_default();
    }

    // Bind the socket to the address
    listener_socket2.bind(&address.into())?;

    // Wrap the socket into a UdpSocket
    Ok(listener_socket2.into())
}

#[inline]
fn build_http3_options(connection_options: &HttpConnectionOptions) -> Http3Options {
    let mut options = Http3Options::default();
    if let Some(qpack_max_table_capacity) = connection_options.h3.qpack_max_table_capacity {
        options = options.qpack_max_table_capacity(qpack_max_table_capacity);
    }
    if let Some(qpack_blocked_streams) = connection_options.h3.qpack_blocked_streams {
        options = options.qpack_blocked_streams(qpack_blocked_streams);
    }
    if let Some(max_field_section_size) = connection_options.h3.max_field_section_size {
        options = options.max_field_section_size(Some(max_field_section_size));
    }
    options = options.enable_connect_protocol(connection_options.h3.enable_connect_protocol);
    options
}

#[allow(clippy::too_many_arguments)]
#[inline]
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
    connection_options: HttpConnectionOptions,
    peer_identity: Option<Vec<rustls::pki_types::CertificateDer<'static>>>,
) {
    let graceful_shutdown = CancellationToken::new();
    let host_control_plane_metadata = resolve_host_control_plane_metadata(
        &observability_resolver,
        Some(local_address.ip()),
        hinted_hostname.as_deref(),
    );
    let host_control_plane_span_links = resolve_host_control_plane_span_links(
        &observability_resolver,
        Some(local_address.ip()),
        hinted_hostname.as_deref(),
    );
    let handler_state = Arc::new(RequestHandlerState {
        pipeline,
        file_pipeline,
        error_pipeline,
        config_resolver,
        connection_observability,
        observability_resolver,
        local_address: Some(local_address),
        remote_address: Some(remote_address),
        unix_socket_path: None,
        hinted_hostname,
        encrypted,
        https_port,
        http3_alt_svc: false,
        timeout_duration: connection_options.timeout,
        peer_identity,
        tls_params: None,
        host_control_plane_metadata,
        host_control_plane_span_links,
    });
    let mut connection_future = Box::pin(
        Http3::new(
            zincio_http::quinn::Connection::new(conn),
            build_http3_options(&connection_options),
        )
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
            vec![
                (
                    "error.type",
                    LogAttributeValue::String("quic_connection_error".into()),
                ),
                (
                    "error.message",
                    LogAttributeValue::String(error.to_string()),
                ),
                (
                    "client.address",
                    LogAttributeValue::String(
                        handler_state
                            .remote_address
                            .expect("QUIC should set remote address")
                            .ip()
                            .to_canonical()
                            .to_string(),
                    ),
                ),
                (
                    "client.port",
                    LogAttributeValue::I64(
                        handler_state
                            .remote_address
                            .expect("QUIC should set remote address")
                            .port() as i64,
                    ),
                ),
                (
                    "server.address",
                    LogAttributeValue::String(
                        handler_state
                            .local_address
                            .expect("QUIC should set local address")
                            .ip()
                            .to_canonical()
                            .to_string(),
                    ),
                ),
                (
                    "server.port",
                    LogAttributeValue::I64(
                        handler_state
                            .local_address
                            .expect("QUIC should set local address")
                            .port() as i64,
                    ),
                ),
            ],
        );
    }
}
