//! HTTP server implementation

use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use ferron_core::providers::Provider;
use ferron_core::runtime::Runtime;
use ferron_core::Module;
use ferron_core::{config::ServerConfigurationBlock, pipeline::Pipeline};
use ferron_http::HttpContext;
use ferron_observability::{EventSink, ObservabilityContext, ObservabilityProviderEventSink};
use ferron_tls::TcpTlsContext;
use parking_lot::Mutex;

use crate::server::tls_resolve::RadixTree;
use crate::{
    config::{prepare_host_config, ThreeStageResolver},
    server::tls_resolve::TlsResolverRadixTree,
};

mod tcp;
mod tls_resolve;

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

pub struct BasicHttpModule {
    pipeline: Arc<Pipeline<HttpContext>>,
    global_config: Arc<ferron_core::config::ServerConfigurationBlock>,
    config_resolver: Arc<crate::config::ThreeStageResolver>,
    tls_resolver: Option<Arc<self::tls_resolve::TlsResolverRadixTree>>,
    observability_resolver: Arc<self::tls_resolve::RadixTree<Vec<Arc<dyn EventSink>>>>,
    listeners: Mutex<Vec<tcp::TcpListenerHandle>>,
    port: u16,
}

impl BasicHttpModule {
    pub fn new(
        registry: &ferron_core::registry::Registry,
        port_config: ferron_core::config::ServerConfigurationPort,
        global_config: Arc<ferron_core::config::ServerConfigurationBlock>,
        port: u16,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut enable_tls = false;
        let mut observability_resolver = RadixTree::new();
        let mut tls_resolver = TlsResolverRadixTree::new();
        for host_config in &port_config.hosts {
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
                        let tls_provider_name = tls1
                            .children
                            .as_ref()
                            .and_then(|c| c.get_value("provider"))
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
                                    >(
                                        &tls1.children.as_ref().expect("TLS config not found")
                                    )
                                },
                                // TODO: ALPN
                                alpn: None,
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
                    // TODO: implicit automatic TLS
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
                                ObservabilityProviderEventSink::new(observability_provider),
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
        Ok(Self {
            pipeline: Arc::new(pipeline),
            global_config,
            config_resolver: Arc::new(ThreeStageResolver::from_prepared(prepare_host_config(
                port_config,
            )?)),
            tls_resolver: if enable_tls {
                Some(Arc::new(tls_resolver))
            } else {
                None
            },
            observability_resolver: Arc::new(observability_resolver),
            listeners: Mutex::new(Vec::new()),
            port,
        })
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
            let pipeline = self.pipeline.clone();
            let listener_options = resolve_tcp_listener_options(&self.global_config, port)?;
            let listener = tcp::TcpListenerHandle::new(
                listener_options,
                pipeline,
                runtime,
                self.tls_resolver.clone(),
                self.observability_resolver.clone(),
            )?;
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

    fn number_directive(value: i64) -> ServerConfigurationDirectiveEntry {
        ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValueBuilder::number(value)],
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
}
