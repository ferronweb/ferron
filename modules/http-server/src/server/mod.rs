//! HTTP server implementation

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use arc_swap::ArcSwap;
use ferron_core::config::{ServerConfigurationDirectiveEntry, ServerConfigurationValue};
use ferron_core::runtime::Runtime;
use ferron_core::Module;
use ferron_core::{config::ServerConfigurationBlock, pipeline::Pipeline};
use ferron_http::{HttpContext, HttpErrorContext, HttpFileContext};
use ferron_observability::{EventSink, ObservabilityContext, ObservabilityProviderEventSink};
use ferron_tls::TcpTlsContext;
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;

use crate::server::tls_resolve::RadixTree;
use crate::{
    config::{prepare_host_config, ThreeStageResolver},
    server::tls_resolve::TlsResolverRadixTree,
};

mod tcp;
mod tls_resolve;

/// Configuration that can be atomically swapped during reload.
/// Contains all reloadable state for the HTTP server module.
pub struct HttpServerConfig {
    pub pipeline: Arc<Pipeline<HttpContext>>,
    pub file_pipeline: Arc<Pipeline<HttpFileContext>>,
    pub error_pipeline: Arc<Pipeline<HttpErrorContext>>,
    pub global_config: Arc<ferron_core::config::ServerConfigurationBlock>,
    pub config_resolver: Arc<crate::config::ThreeStageResolver>,
    pub tls_resolver: Option<Arc<self::tls_resolve::TlsResolverRadixTree>>,
    pub http_connection_options_resolver:
        Arc<self::tls_resolve::RadixTree<tcp::HttpConnectionOptions>>,
    pub observability_resolver: Arc<self::tls_resolve::RadixTree<Vec<Arc<dyn EventSink>>>>,
    /// Token that is cancelled when configuration is reloaded to gracefully shut down existing connections.
    pub reload_token: CancellationToken,
}

type ConfigArcSwap = Arc<ArcSwap<HttpServerConfig>>;

#[inline]
fn format_location(
    block_name: Option<&str>,
    span: Option<&ferron_core::config::ServerConfigurationSpan>,
) -> String {
    let mut location = String::new();
    if let Some(name) = block_name {
        location.push_str(&format!("block '{}'", name));
    } else {
        location.push_str("global configuration");
    }
    if let Some(span) = span {
        if let Some(file) = &span.file {
            location.push_str(&format!(" in file '{}'", file));
        }
        location.push_str(&format!(" at line {}", span.line));
        location.push_str(&format!(", column {}", span.column));
    }
    location
}

#[inline]
fn tcp_config(global_config: &ServerConfigurationBlock) -> Option<&ServerConfigurationBlock> {
    global_config
        .directives
        .get("tcp")
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.children.as_ref())
}

fn resolve_tcp_buffer_size(
    tcp_config: Option<&ServerConfigurationBlock>,
    directive: &str,
) -> anyhow::Result<Option<usize>> {
    let Some(value) = tcp_config.and_then(|config| config.get_value(directive)) else {
        return Ok(None);
    };

    let Some(size) = value.as_number() else {
        anyhow::bail!("tcp.{directive} must be a number");
    };

    Ok(Some(usize::try_from(size).map_err(|_| {
        anyhow::anyhow!("tcp.{directive} must be a non-negative integer")
    })?))
}

fn resolve_tcp_listener_options(
    global_config: &ServerConfigurationBlock,
    port: u16,
) -> anyhow::Result<tcp::TcpListenerOptions> {
    let tcp_config = tcp_config(global_config);
    let address = match tcp_config.and_then(|config| config.get_value("listen")) {
        Some(value) => {
            let listen = value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("tcp.listen must be a string"))?;

            if let Ok(address) = listen.parse::<SocketAddr>() {
                if address.port() != port {
                    anyhow::bail!(
                        "tcp.listen address port {} does not match the configured HTTP port {}",
                        address.port(),
                        port
                    );
                }
                address
            } else {
                SocketAddr::new(
                    listen
                        .parse::<IpAddr>()
                        .map_err(|_| anyhow::anyhow!("Invalid tcp.listen address '{listen}'"))?,
                    port,
                )
            }
        }
        None => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port),
    };

    Ok(tcp::TcpListenerOptions {
        address,
        send_buffer_size: resolve_tcp_buffer_size(tcp_config, "send_buf")?,
        recv_buffer_size: resolve_tcp_buffer_size(tcp_config, "recv_buf")?,
    })
}

#[inline]
fn http_config(config: &ServerConfigurationBlock) -> Option<&ServerConfigurationBlock> {
    config
        .directives
        .get("http")
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.children.as_ref())
}

fn resolve_http_u32(
    http_config: Option<&ServerConfigurationBlock>,
    directive: &str,
) -> anyhow::Result<Option<u32>> {
    let Some(value) = http_config.and_then(|config| config.get_value(directive)) else {
        return Ok(None);
    };

    let Some(size) = value.as_number() else {
        anyhow::bail!("http.{directive} must be a number");
    };

    Ok(Some(u32::try_from(size).map_err(|_| {
        anyhow::anyhow!("http.{directive} must be a non-negative integer")
    })?))
}

fn resolve_http_protocols(
    http_config: Option<&ServerConfigurationBlock>,
) -> anyhow::Result<tcp::HttpProtocols> {
    let Some(protocols_entry) = http_config
        .and_then(|config| config.directives.get("protocols"))
        .and_then(|entries| entries.first())
    else {
        return Ok(tcp::HttpProtocols::default());
    };

    let mut protocols = tcp::HttpProtocols::empty();
    for value in &protocols_entry.args {
        let protocol = value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("http.protocols values must be strings"))?;
        match protocol {
            "h1" => protocols.http1 = true,
            "h2" => protocols.http2 = true,
            unsupported => anyhow::bail!("Unsupported HTTP protocol '{unsupported}'"),
        }
    }

    if !protocols.http1 && !protocols.http2 {
        anyhow::bail!("http.protocols must enable at least one supported protocol");
    }

    Ok(protocols)
}

fn resolve_http_connection_options(
    config: &ServerConfigurationBlock,
) -> anyhow::Result<tcp::HttpConnectionOptions> {
    let http_config = http_config(config);
    Ok(tcp::HttpConnectionOptions {
        protocols: resolve_http_protocols(http_config)?,
        h1_enable_early_hints: http_config
            .and_then(|config| config.get_value("h1_enable_early_hints"))
            .and_then(|value| value.as_boolean())
            .unwrap_or(false),
        h2: tcp::Http2Settings {
            initial_window_size: resolve_http_u32(http_config, "h2_initial_window_size")?,
            max_frame_size: resolve_http_u32(http_config, "h2_max_frame_size")?,
            max_concurrent_streams: resolve_http_u32(http_config, "h2_max_concurrent_streams")?,
            max_header_list_size: resolve_http_u32(http_config, "h2_max_header_list_size")?,
            enable_connect_protocol: http_config
                .and_then(|config| config.get_value("h2_enable_connect_protocol"))
                .and_then(|value| value.as_boolean())
                .unwrap_or(false),
        },
    })
}

pub struct BasicHttpModule {
    config: ConfigArcSwap,
    listeners: Mutex<Vec<tcp::TcpListenerHandle>>,
    port: u16,
}

impl BasicHttpModule {
    /// Build the HTTP server configuration from the given port and global config.
    /// This is used by both `new()` and `reload()`.
    fn build_config(
        registry: &ferron_core::registry::Registry,
        port_config: &ferron_core::config::ServerConfigurationPort,
        global_config: Arc<ferron_core::config::ServerConfigurationBlock>,
    ) -> Result<HttpServerConfig, Box<dyn std::error::Error>> {
        let mut enable_tls = false;
        let mut http_connection_options_resolver = RadixTree::new();
        let mut observability_resolver = RadixTree::new();
        let mut tls_resolver = TlsResolverRadixTree::new();
        for host_config in &port_config.hosts {
            let http_connection_options = resolve_http_connection_options(&host_config.1)?;
            match (&host_config.0.host, host_config.0.ip) {
                (Some(host), Some(ip)) => {
                    http_connection_options_resolver.insert_ip_and_hostname(
                        ip,
                        host,
                        http_connection_options.clone(),
                        false,
                    );
                }
                (Some(host), None) => {
                    http_connection_options_resolver.insert_hostname(
                        host,
                        http_connection_options.clone(),
                        false,
                    );
                }
                (None, Some(ip)) => {
                    http_connection_options_resolver.insert_ip(ip, http_connection_options.clone());
                }
                (None, None) => {
                    http_connection_options_resolver.set_root_data(http_connection_options.clone());
                }
            }

            if let Some(tls) = host_config.1.directives.get("tls") {
                for tls1 in tls {
                    // TODO: implicit automatic TLS
                    if tls1
                        .args
                        .first()
                        .and_then(|a| a.as_boolean())
                        .unwrap_or(true)
                    {
                        enable_tls = true;
                        let mut children = tls1.children.clone().unwrap_or(Default::default());
                        if let (Some(cert), Some(key)) = (
                            tls1.args
                                .first()
                                .and_then(|v| v.as_string_with_interpolations(&HashMap::new())),
                            tls1.args
                                .get(1)
                                .and_then(|v| v.as_string_with_interpolations(&HashMap::new())),
                        ) {
                            let mut directives = (*children.directives).clone();
                            directives.insert(
                                "provider".to_string(),
                                vec![ServerConfigurationDirectiveEntry {
                                    args: vec![ServerConfigurationValue::String(
                                        "manual".to_string(),
                                        None,
                                    )],
                                    ..Default::default()
                                }],
                            );
                            directives.insert(
                                "cert".to_string(),
                                vec![ServerConfigurationDirectiveEntry {
                                    args: vec![ServerConfigurationValue::String(cert, None)],
                                    ..Default::default()
                                }],
                            );
                            directives.insert(
                                "key".to_string(),
                                vec![ServerConfigurationDirectiveEntry {
                                    args: vec![ServerConfigurationValue::String(key, None)],
                                    ..Default::default()
                                }],
                            );
                            children.directives = Arc::new(directives);
                        }

                        let tls_provider_name = children
                            .get_value("provider")
                            .ok_or(anyhow::anyhow!(
                                "TLS provider not specified ({})",
                                format_location(None, tls1.span.as_ref())
                            ))?
                            .as_str()
                            .ok_or(anyhow::anyhow!(
                                "TLS provider must be a string ({})",
                                format_location(None, tls1.span.as_ref())
                            ))?;

                        if let Some(tls_registry) =
                            registry.get_provider_registry::<TcpTlsContext>()
                        {
                            let tls_provider =
                                tls_registry.get(tls_provider_name).ok_or(anyhow::anyhow!(
                                    "TLS provider not found ({})",
                                    format_location(None, tls1.span.as_ref())
                                ))?;

                            let mut tls_resolver_ctx = TcpTlsContext {
                                // SAFETY: We know that the lifetime of the config is longer
                                //         than the lifetime of the resolver. but "'static"
                                //         is the only lifetime we can use here. This
                                //         constraint is enforced by the provider registry.
                                config: unsafe {
                                    std::mem::transmute::<
                                        &ServerConfigurationBlock,
                                        &'static ServerConfigurationBlock,
                                    >(&children)
                                },
                                alpn: {
                                    let alpn_protocols = http_connection_options.alpn_protocols();
                                    (!alpn_protocols.is_empty()).then_some(alpn_protocols)
                                },
                                resolver: None,
                            };
                            tls_provider.execute(&mut tls_resolver_ctx)?;
                            let tls_resolver_sub =
                                tls_resolver_ctx.resolver.ok_or(anyhow::anyhow!(
                                    "TLS resolver not found ({})",
                                    format_location(None, tls1.span.as_ref())
                                ))?;

                            match (&host_config.0.host, host_config.0.ip) {
                                (Some(host), Some(ip)) => {
                                    tls_resolver.insert_ip_and_hostname(
                                        ip,
                                        host,
                                        tls_resolver_sub,
                                        false,
                                    );
                                }
                                (Some(host), None) => {
                                    tls_resolver.insert_hostname(host, tls_resolver_sub, false);
                                }
                                (None, Some(ip)) => {
                                    tls_resolver.insert_ip(ip, tls_resolver_sub);
                                }
                                (None, None) => {
                                    tls_resolver.set_root_data(tls_resolver_sub);
                                }
                            }
                        }
                    }
                }
            }
            if let Some(observability) = host_config.1.directives.get("observability") {
                let mut observability_to_insert: Vec<Arc<dyn EventSink>> = Vec::new();
                for observability1 in observability {
                    if observability1
                        .args
                        .first()
                        .and_then(|a| a.as_boolean())
                        .unwrap_or(true)
                    {
                        let observability_provider_name = observability1
                            .children
                            .as_ref()
                            .and_then(|c| c.get_value("provider"))
                            .ok_or(anyhow::anyhow!(
                                "Observability provider not specified ({})",
                                format_location(None, observability1.span.as_ref())
                            ))?
                            .as_str()
                            .ok_or(anyhow::anyhow!(
                                "Observability provider must be a string ({})",
                                format_location(None, observability1.span.as_ref())
                            ))?;

                        if let Some(observability_registry) =
                            registry.get_provider_registry::<ObservabilityContext>()
                        {
                            let observability_provider = observability_registry
                                .get(observability_provider_name)
                                .ok_or(anyhow::anyhow!(
                                    "Observability provider not found ({})",
                                    format_location(None, observability1.span.as_ref())
                                ))?;

                            observability_to_insert.push(Arc::new(
                                ObservabilityProviderEventSink::new(
                                    observability_provider,
                                    Arc::new(
                                        observability1
                                            .children
                                            .as_ref()
                                            .cloned()
                                            .unwrap_or_default(),
                                    ),
                                ),
                            ));
                        }
                    }
                }
                match (&host_config.0.host, host_config.0.ip) {
                    (Some(host), Some(ip)) => {
                        observability_resolver.insert_ip_and_hostname(
                            ip,
                            host,
                            observability_to_insert,
                            false,
                        );
                    }
                    (Some(host), None) => {
                        observability_resolver.insert_hostname(
                            host,
                            observability_to_insert,
                            false,
                        );
                    }
                    (None, Some(ip)) => {
                        observability_resolver.insert_ip(ip, observability_to_insert);
                    }
                    (None, None) => {
                        observability_resolver.set_root_data(observability_to_insert);
                    }
                }
            }
        }
        let pipeline = registry
            .get_stage_registry::<HttpContext>()
            .expect("HTTP stage registry not found")
            .build_all();
        let file_pipeline = registry
            .get_stage_registry::<HttpFileContext>()
            .map(|registry| registry.build_all())
            .unwrap_or_else(Pipeline::new);
        let error_pipeline = registry
            .get_stage_registry::<HttpErrorContext>()
            .map(|registry| registry.build_all())
            .unwrap_or_else(Pipeline::new);

        Ok(HttpServerConfig {
            pipeline: Arc::new(pipeline),
            file_pipeline: Arc::new(file_pipeline),
            error_pipeline: Arc::new(error_pipeline),
            global_config: global_config.clone(),
            config_resolver: Arc::new(ThreeStageResolver::from_prepared_with_global(
                prepare_host_config(port_config.clone())?,
                global_config,
            )),
            tls_resolver: if enable_tls {
                Some(Arc::new(tls_resolver))
            } else {
                None
            },
            http_connection_options_resolver: Arc::new(http_connection_options_resolver),
            observability_resolver: Arc::new(observability_resolver),
            reload_token: CancellationToken::new(),
        })
    }

    pub fn new(
        registry: &ferron_core::registry::Registry,
        port_config: ferron_core::config::ServerConfigurationPort,
        global_config: Arc<ferron_core::config::ServerConfigurationBlock>,
        port: u16,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let config = Self::build_config(registry, &port_config, global_config)?;
        Ok(Self {
            config: Arc::new(ArcSwap::new(Arc::new(config))),
            listeners: Mutex::new(Vec::new()),
            port,
        })
    }

    /// Reload the module with new configuration.
    /// This method is called during configuration reload (SIGHUP).
    /// It atomically replaces all reloadable fields with new values.
    pub fn reload(
        &self,
        registry: &ferron_core::registry::Registry,
        port_config: ferron_core::config::ServerConfigurationPort,
        global_config: Arc<ferron_core::config::ServerConfigurationBlock>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Cancel the old reload token to trigger graceful shutdown of existing connections
        let old_config = self.config.load();

        // Build new configuration and atomically swap it
        let new_config = Self::build_config(registry, &port_config, global_config)?;
        self.config.store(Arc::new(new_config));

        old_config.reload_token.cancel();

        Ok(())
    }
}

impl Module for BasicHttpModule {
    fn name(&self) -> &str {
        "http"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn start(&self, runtime: &mut Runtime) -> Result<(), Box<dyn std::error::Error>> {
        let ports = if self.port != 0 {
            vec![self.port]
        } else {
            vec![80]
        };
        for port in ports {
            let config = self.config.load();
            let listener_options = resolve_tcp_listener_options(&config.global_config, port)?;
            let listener =
                tcp::TcpListenerHandle::new(listener_options, self.config.clone(), runtime)?;
            self.listeners.lock().push(listener);
            // TODO: QUIC
        }

        Ok(())
    }
}

impl Drop for BasicHttpModule {
    fn drop(&mut self) {
        for listener in &*self.listeners.lock() {
            listener.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationBlockBuilder,
        ServerConfigurationDirectiveEntry, ServerConfigurationValueBuilder,
    };

    use super::*;

    fn tcp_directive(children: ServerConfigurationBlock) -> ServerConfigurationDirectiveEntry {
        ServerConfigurationDirectiveEntry {
            args: vec![],
            children: Some(children),
            span: None,
        }
    }

    fn http_directive(children: ServerConfigurationBlock) -> ServerConfigurationDirectiveEntry {
        ServerConfigurationDirectiveEntry {
            args: vec![],
            children: Some(children),
            span: None,
        }
    }

    fn number_directive(value: i64) -> ServerConfigurationDirectiveEntry {
        ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValueBuilder::number(value)],
            children: None,
            span: None,
        }
    }

    fn boolean_directive(value: bool) -> ServerConfigurationDirectiveEntry {
        ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValueBuilder::boolean(value)],
            children: None,
            span: None,
        }
    }

    #[test]
    fn tcp_listener_options_use_dual_stack_defaults() {
        let global_config = ServerConfigurationBlockBuilder::new().build();

        let options = resolve_tcp_listener_options(&global_config, 8080).unwrap();

        assert_eq!(
            options.address,
            SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 8080)
        );
        assert_eq!(options.send_buffer_size, None);
        assert_eq!(options.recv_buffer_size, None);
    }

    #[test]
    fn tcp_listener_options_read_ip_and_buffer_sizes() {
        let tcp_block = ServerConfigurationBlockBuilder::new()
            .directive_str("listen", vec!["127.0.0.1"])
            .directive("send_buf", number_directive(65536))
            .directive("recv_buf", number_directive(131072))
            .build();
        let global_config = ServerConfigurationBlockBuilder::new()
            .directive("tcp", tcp_directive(tcp_block))
            .build();

        let options = resolve_tcp_listener_options(&global_config, 8080).unwrap();

        assert_eq!(
            options.address,
            SocketAddr::new(IpAddr::from([127, 0, 0, 1]), 8080)
        );
        assert_eq!(options.send_buffer_size, Some(65536));
        assert_eq!(options.recv_buffer_size, Some(131072));
    }

    #[test]
    fn tcp_listener_options_reject_negative_buffer_sizes() {
        let tcp_block = ServerConfigurationBlockBuilder::new()
            .directive("send_buf", number_directive(-1))
            .build();
        let global_config = ServerConfigurationBlockBuilder::new()
            .directive("tcp", tcp_directive(tcp_block))
            .build();

        let error = resolve_tcp_listener_options(&global_config, 8080).unwrap_err();

        assert_eq!(
            error.to_string(),
            "tcp.send_buf must be a non-negative integer"
        );
    }

    #[test]
    fn http_connection_options_default_to_h1_and_h2() {
        let config = ServerConfigurationBlockBuilder::new().build();

        let options = resolve_http_connection_options(&config).unwrap();

        assert_eq!(options.protocols, tcp::HttpProtocols::default());
        assert_eq!(
            options.alpn_protocols(),
            vec![b"h2".to_vec(), b"http/1.1".to_vec(), b"http/1.0".to_vec()]
        );
        assert!(!options.h1_enable_early_hints);
        assert_eq!(options.h2, tcp::Http2Settings::default());
    }

    #[test]
    fn http_connection_options_read_protocols_and_h2_settings() {
        let http_block = ServerConfigurationBlockBuilder::new()
            .directive_str("protocols", vec!["h1"])
            .directive("h1_enable_early_hints", boolean_directive(true))
            .directive("h2_initial_window_size", number_directive(65_535))
            .directive("h2_max_frame_size", number_directive(32_768))
            .directive("h2_max_concurrent_streams", number_directive(128))
            .directive("h2_max_header_list_size", number_directive(16_384))
            .directive("h2_enable_connect_protocol", boolean_directive(true))
            .build();
        let config = ServerConfigurationBlockBuilder::new()
            .directive("http", http_directive(http_block))
            .build();

        let options = resolve_http_connection_options(&config).unwrap();

        assert_eq!(
            options.protocols,
            tcp::HttpProtocols {
                http1: true,
                http2: false,
            }
        );
        assert_eq!(
            options.alpn_protocols(),
            vec![b"http/1.1".to_vec(), b"http/1.0".to_vec()]
        );
        assert!(options.h1_enable_early_hints);
        assert_eq!(
            options.h2,
            tcp::Http2Settings {
                initial_window_size: Some(65_535),
                max_frame_size: Some(32_768),
                max_concurrent_streams: Some(128),
                max_header_list_size: Some(16_384),
                enable_connect_protocol: true,
            }
        );
    }

    #[test]
    fn http_connection_options_reject_unknown_protocols() {
        let http_block = ServerConfigurationBlockBuilder::new()
            .directive_str("protocols", vec!["h3"])
            .build();
        let config = ServerConfigurationBlockBuilder::new()
            .directive("http", http_directive(http_block))
            .build();

        let error = resolve_http_connection_options(&config).unwrap_err();

        assert_eq!(error.to_string(), "Unsupported HTTP protocol 'h3'");
    }
}
