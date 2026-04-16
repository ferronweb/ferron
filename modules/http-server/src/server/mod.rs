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
use ferron_observability::{ObservabilityConfigExtractor, ObservabilityContext};
use ferron_tls::TcpTlsContext;
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;

use crate::server::quic::{QuicTlsResolver, QuicTlsSniResolvers};
use crate::server::sni::CustomSniResolver;
use crate::server::tls_resolve::RadixTree;
use crate::{
    config::{prepare_host_config, ThreeStageResolver},
    server::tls_resolve::TlsResolverRadixTree,
};

mod common;
mod quic;
mod sni;
mod tcp;
mod tls_resolve;

type ObservabilityProviderEntry = (
    Arc<dyn ferron_core::providers::Provider<ObservabilityContext>>,
    Arc<ferron_core::config::ServerConfigurationBlock>,
);

/// Configuration that can be atomically swapped during reload.
/// Contains all reloadable state for the HTTP server module.
pub struct HttpServerConfig {
    pub pipeline: Arc<Pipeline<HttpContext>>,
    pub file_pipeline: Arc<Pipeline<HttpFileContext>>,
    pub error_pipeline: Arc<Pipeline<HttpErrorContext>>,
    pub global_config: Arc<ferron_core::config::ServerConfigurationBlock>,
    pub config_resolver: Arc<crate::config::ThreeStageResolver>,
    pub tls_resolver: Option<Arc<self::tls_resolve::TlsResolverRadixTree>>,
    pub quic_tls_resolver: Option<Arc<QuicTlsResolver>>,
    pub http_connection_options_resolver:
        Arc<self::tls_resolve::RadixTree<common::HttpConnectionOptions>>,
    pub observability_resolver: Arc<self::tls_resolve::RadixTree<Vec<ObservabilityProviderEntry>>>,
    /// Token that is cancelled when configuration is reloaded to gracefully shut down existing connections.
    pub reload_token: CancellationToken,
    /// The canonical HTTPS port for this server (default: 443).
    /// Used for HTTP-to-HTTPS redirects and URL generation.
    /// `None` if HTTPS is disabled or not applicable for this listener.
    pub https_port: Option<u16>,
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
    }
    if let Some(span) = span {
        if let Some(file) = &span.file {
            location.push_str(&format!("in file '{}' ", file));
        }
        location.push_str(&format!("at line {}", span.line));
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

fn resolve_tcp_backlog(
    tcp_config: Option<&ServerConfigurationBlock>,
) -> anyhow::Result<Option<i32>> {
    let Some(value) = tcp_config.and_then(|config| config.get_value("backlog")) else {
        return Ok(None);
    };

    let Some(size) = value.as_number() else {
        anyhow::bail!("tcp.backlog must be a number");
    };

    Ok(Some(i32::try_from(size).map_err(|_| {
        anyhow::anyhow!("tcp.backlog must be a non-negative integer")
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
        backlog: resolve_tcp_backlog(tcp_config)?,
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
) -> anyhow::Result<common::HttpProtocols> {
    let Some(protocols_entry) = http_config
        .and_then(|config| config.directives.get("protocols"))
        .and_then(|entries| entries.first())
    else {
        return Ok(common::HttpProtocols::default());
    };

    let mut protocols = common::HttpProtocols::empty();
    for value in &protocols_entry.args {
        let protocol = value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("http.protocols values must be strings"))?;
        match protocol {
            "h1" => protocols.http1 = true,
            "h2" => protocols.http2 = true,
            "h3" => protocols.http3 = true,
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
) -> anyhow::Result<common::HttpConnectionOptions> {
    let http_config = http_config(config);
    Ok(common::HttpConnectionOptions {
        protocols: resolve_http_protocols(http_config)?,
        h1_enable_early_hints: http_config
            .and_then(|config| config.get_value("h1_enable_early_hints"))
            .and_then(|value| value.as_boolean())
            .unwrap_or(false),
        h2: common::Http2Settings {
            initial_window_size: resolve_http_u32(http_config, "h2_initial_window_size")?,
            max_frame_size: resolve_http_u32(http_config, "h2_max_frame_size")?,
            max_concurrent_streams: resolve_http_u32(http_config, "h2_max_concurrent_streams")?,
            max_header_list_size: resolve_http_u32(http_config, "h2_max_header_list_size")?,
            enable_connect_protocol: http_config
                .and_then(|config| config.get_value("h2_enable_connect_protocol"))
                .and_then(|value| value.as_boolean())
                .unwrap_or(false),
        },
        proxy_protocol_enabled: http_config
            .and_then(|config| config.get_value("protocol_proxy"))
            .and_then(|value| value.as_boolean())
            .unwrap_or(false),
    })
}

pub struct BasicHttpModule {
    config: ConfigArcSwap,
    listeners: Mutex<Vec<tcp::TcpListenerHandle>>,
    quic_listeners: Mutex<Vec<quic::QuicListenerHandle>>,
    port: u16,
}

impl BasicHttpModule {
    /// Build the HTTP server configuration from the given port and global config.
    /// This is used by both `new()` and `reload()`.
    fn build_config(
        registry: &ferron_core::registry::Registry,
        port_config: &ferron_core::config::ServerConfigurationPort,
        global_config: Arc<ferron_core::config::ServerConfigurationBlock>,
        https_port: Option<u16>,
    ) -> Result<HttpServerConfig, Box<dyn std::error::Error>> {
        let mut enable_tls = false;
        let mut http_connection_options_resolver = RadixTree::new();
        let mut observability_resolver = RadixTree::new();
        let mut tls_resolver = TlsResolverRadixTree::new();

        // Process global observability configuration (applies to all hosts)
        let global_observability_extractor = ObservabilityConfigExtractor::new(&global_config);
        let global_observability_entries: Vec<ObservabilityProviderEntry> =
            global_observability_extractor
                .extract_observability_blocks()?
                .into_iter()
                .filter_map(|observability_block| {
                    let observability_provider_name = match observability_block
                        .get_value("provider")
                        .and_then(|v| v.as_str())
                    {
                        Some(name) => name.to_string(),
                        None => {
                            // Error will be handled by host-level extraction if provider is missing there
                            return None;
                        }
                    };

                    registry
                        .get_provider_registry::<ObservabilityContext>()
                        .and_then(|observability_registry| {
                            observability_registry
                                .get(&observability_provider_name)
                                .map(|provider| {
                                    let observability_block_arc = Arc::new(observability_block);
                                    (provider, observability_block_arc)
                                })
                        })
                })
                .collect();

        // If global observability entries exist, set them as root data (fallback for all hosts)
        if !global_observability_entries.is_empty() {
            observability_resolver.set_root_data(global_observability_entries);
        }

        let mut quic_tls_resolver = None;
        for host_config in &port_config.hosts {
            let http_connection_options =
                resolve_http_connection_options(&host_config.1).map_err(|e| {
                    anyhow::anyhow!(
                        "Can't determine HTTP connection options ({}): {e}",
                        format_location(None, host_config.1.span.as_ref())
                    )
                })?;
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

            // Only process TLS directives on the HTTPS listener.
            // The plaintext HTTP listener (port != https_port) ignores all `tls` directives,
            // including explicit configurations and automatic ACME.
            if https_port.is_some() && port_config.port == https_port {
                if let Some(tls) = host_config.1.directives.get("tls") {
                    for tls1 in tls {
                        // Handle explicit `tls false` — skip TLS entirely
                        if tls1
                            .args
                            .first()
                            .and_then(|a| a.as_boolean())
                            .is_some_and(|v| !v)
                        {
                            continue;
                        }
                        let host_config_with_arc =
                            (host_config.0.clone(), Arc::new(host_config.1.clone()));
                        Self::process_tls_directive(
                            registry,
                            tls1,
                            &host_config_with_arc,
                            &http_connection_options,
                            port_config,
                            &mut tls_resolver,
                            &mut quic_tls_resolver,
                            &mut enable_tls,
                        )?;
                    }
                } else {
                    // No `tls` directive present — automatically select provider (ACME or Local)
                    let hostname = host_config.0.host.as_deref();
                    let ip = host_config.0.ip.map(|ip| ip.to_string());
                    let auto_selection = crate::tls_auto::select_auto_tls_provider(
                        registry,
                        hostname,
                        ip.as_deref(),
                    );

                    match auto_selection {
                        crate::tls_auto::TlsAutoSelection::Local => {
                            let synthetic_tls_entry =
                                crate::tls_auto::create_synthetic_tls_directive("local");
                            let host_config_with_arc =
                                (host_config.0.clone(), Arc::new(host_config.1.clone()));
                            Self::process_tls_directive(
                                registry,
                                &synthetic_tls_entry,
                                &host_config_with_arc,
                                &http_connection_options,
                                port_config,
                                &mut tls_resolver,
                                &mut quic_tls_resolver,
                                &mut enable_tls,
                            )?;
                        }
                        crate::tls_auto::TlsAutoSelection::Acme => {
                            let synthetic_tls_entry =
                                crate::tls_auto::create_synthetic_tls_directive("acme");
                            let host_config_with_arc =
                                (host_config.0.clone(), Arc::new(host_config.1.clone()));
                            Self::process_tls_directive(
                                registry,
                                &synthetic_tls_entry,
                                &host_config_with_arc,
                                &http_connection_options,
                                port_config,
                                &mut tls_resolver,
                                &mut quic_tls_resolver,
                                &mut enable_tls,
                            )?;
                        }
                        crate::tls_auto::TlsAutoSelection::None => {}
                    }
                }
            }

            // Process observability using the config extractor (handles both explicit blocks and aliases)
            let observability_extractor = ObservabilityConfigExtractor::new(&host_config.1);
            for observability_block in observability_extractor.extract_observability_blocks()? {
                let observability_provider_name = observability_block
                    .get_value("provider")
                    .ok_or(anyhow::anyhow!(
                        "Observability provider not specified ({})",
                        format_location(None, observability_block.span.as_ref())
                    ))?
                    .as_str()
                    .ok_or(anyhow::anyhow!(
                        "Observability provider must be a string ({})",
                        format_location(None, observability_block.span.as_ref())
                    ))?;

                if let Some(observability_registry) =
                    registry.get_provider_registry::<ObservabilityContext>()
                {
                    let observability_provider = observability_registry
                        .get(observability_provider_name)
                        .ok_or(anyhow::anyhow!(
                            "Observability provider '{observability_provider_name}' not found ({})",
                            format_location(None, observability_block.span.as_ref())
                        ))?;

                    let observability_block_arc = Arc::new(observability_block);

                    // Insert provider + config tuple into the resolver (sink initialization deferred)
                    let entry: ObservabilityProviderEntry =
                        (observability_provider, observability_block_arc);
                    match (&host_config.0.host, host_config.0.ip) {
                        (Some(host), Some(ip)) => {
                            observability_resolver.insert_ip_and_hostname(
                                ip,
                                host,
                                vec![entry],
                                false,
                            );
                        }
                        (Some(host), None) => {
                            observability_resolver.insert_hostname(host, vec![entry], false);
                        }
                        (None, Some(ip)) => {
                            observability_resolver.insert_ip(ip, vec![entry]);
                        }
                        (None, None) => {
                            // Merge with global observability entries if they exist
                            let existing_root = observability_resolver.root_data();
                            let mut merged_entries = existing_root.unwrap_or_default();
                            merged_entries.push(entry);
                            observability_resolver.set_root_data(merged_entries);
                        }
                    }
                }
            }
        }
        // Build a merged config from global + all host blocks so that
        // `is_applicable` includes a stage if *any* host uses its directive.
        let port_config_merged = ferron_core::config::ServerConfigurationBlock::merge_from(
            std::iter::once(global_config.as_ref())
                .chain(port_config.hosts.iter().map(|(_, block)| block)),
        );
        let merged_config = Some(&port_config_merged);
        let pipeline = registry
            .get_stage_registry::<HttpContext>()
            .expect("HTTP stage registry not found")
            .build_with_config(merged_config);
        let file_pipeline = registry
            .get_stage_registry::<HttpFileContext>()
            .map(|registry| registry.build_with_config(merged_config))
            .unwrap_or_else(Pipeline::new);
        let error_pipeline = registry
            .get_stage_registry::<HttpErrorContext>()
            .map(|registry| registry.build_with_config(merged_config))
            .unwrap_or_else(Pipeline::new);

        // Optional: allow configuring path resolve cache TTL via `http.path_resolve_cache_ttl_ms`
        if let Ok(Some(ms)) = resolve_http_u32(
            http_config(global_config.as_ref()),
            "path_resolve_cache_ttl_ms",
        ) {
            // Set global TTL (milliseconds) used by the path resolution cache.
            crate::handler::set_path_resolve_cache_ttl_millis(ms as u64);
        }

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
            quic_tls_resolver: if let Some(quic_resolver) = quic_tls_resolver {
                Some(Arc::new(quic_resolver.try_into()?))
            } else {
                None
            },
            http_connection_options_resolver: Arc::new(http_connection_options_resolver),
            observability_resolver: Arc::new(observability_resolver),
            reload_token: CancellationToken::new(),
            https_port,
        })
    }

    /// Process a single TLS directive entry (from either explicit config or synthetic ACME).
    #[allow(clippy::too_many_arguments)]
    fn process_tls_directive(
        registry: &ferron_core::registry::Registry,
        tls1: &ServerConfigurationDirectiveEntry,
        host_config: &(
            ferron_core::config::ServerConfigurationHostFilters,
            Arc<ferron_core::config::ServerConfigurationBlock>,
        ),
        http_connection_options: &common::HttpConnectionOptions,
        port_config: &ferron_core::config::ServerConfigurationPort,
        tls_resolver: &mut TlsResolverRadixTree,
        quic_tls_resolver: &mut Option<QuicTlsSniResolvers>,
        enable_tls: &mut bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        *enable_tls = true;
        let mut children = tls1.children.clone().unwrap_or_default();

        // Handle the shorthand form: `tls /path/cert.pem /path/key.pem`
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
                    args: vec![ServerConfigurationValue::String("manual".to_string(), None)],
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

        if let Some(tls_registry) = registry.get_provider_registry::<TcpTlsContext>() {
            let tls_provider = tls_registry.get(tls_provider_name).ok_or(anyhow::anyhow!(
                "TLS provider '{tls_provider_name}' not found ({})",
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
                domain: host_config.0.clone(),
                port: port_config.port.unwrap_or(443),
                resolver: None,
            };
            tls_provider.execute(&mut tls_resolver_ctx)?;
            let tls_resolver_sub = tls_resolver_ctx.resolver.ok_or(anyhow::anyhow!(
                "TLS resolver '{tls_provider_name}' not found ({})",
                format_location(None, tls1.span.as_ref())
            ))?;
            if http_connection_options.protocols.http3 {
                let quic_tls_resolver = quic_tls_resolver.get_or_insert_default();
                let tls_resolver_host = tls_resolver_sub.get_tls_config().cert_resolver.clone();
                match (&host_config.0.host, host_config.0.ip) {
                    (Some(host), Some(ip)) => {
                        quic_tls_resolver
                            .host
                            .entry(ip)
                            .or_insert_with(CustomSniResolver::new)
                            .load_host_resolver(host, tls_resolver_host);
                    }
                    (Some(host), None) => {
                        quic_tls_resolver
                            .fallback
                            .get_or_insert_with(CustomSniResolver::new)
                            .load_host_resolver(host, tls_resolver_host);
                    }
                    (None, Some(ip)) => {
                        quic_tls_resolver
                            .host
                            .entry(ip)
                            .or_insert_with(CustomSniResolver::new)
                            .load_fallback_resolver(tls_resolver_host);
                    }
                    (None, None) => {
                        quic_tls_resolver
                            .fallback
                            .get_or_insert_with(CustomSniResolver::new)
                            .load_fallback_resolver(tls_resolver_host);
                    }
                }
            }

            match (&host_config.0.host, host_config.0.ip) {
                (Some(host), Some(ip)) => {
                    tls_resolver.insert_ip_and_hostname(ip, host, tls_resolver_sub, false);
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

        Ok(())
    }

    pub fn new(
        registry: &ferron_core::registry::Registry,
        port_config: ferron_core::config::ServerConfigurationPort,
        global_config: Arc<ferron_core::config::ServerConfigurationBlock>,
        port: u16,
        https_port: Option<u16>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let config = Self::build_config(registry, &port_config, global_config, https_port)?;
        Ok(Self {
            config: Arc::new(ArcSwap::new(Arc::new(config))),
            listeners: Mutex::new(Vec::new()),
            quic_listeners: Mutex::new(Vec::new()),
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
        https_port: Option<u16>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Cancel the old reload token to trigger graceful shutdown of existing connections
        let old_config = self.config.load();

        // Build new configuration and atomically swap it
        let new_config = Self::build_config(registry, &port_config, global_config, https_port)?;
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
        let port = if self.port != 0 { self.port } else { 80 };

        let config = self.config.load();
        let has_http3 = config.quic_tls_resolver.is_some();
        let listener_options = resolve_tcp_listener_options(&config.global_config, port)?;
        let listener =
            tcp::TcpListenerHandle::new(listener_options, has_http3, self.config.clone(), runtime)
                .map_err(|e| anyhow::anyhow!("Failed to start HTTP server on port {port}: {e}"))?;
        self.listeners.lock().push(listener);

        if has_http3 {
            let quic_listener = quic::QuicListenerHandle::new(
                listener_options.address,
                self.config.clone(),
                runtime,
            )
            .map_err(|e| anyhow::anyhow!("Failed to start HTTP/3 server on port {port}: {e}"))?;
            self.quic_listeners.lock().push(quic_listener);
        }

        Ok(())
    }
}

impl Drop for BasicHttpModule {
    fn drop(&mut self) {
        for listener in &*self.listeners.lock() {
            listener.cancel();
        }
        for quic_listener in &*self.quic_listeners.lock() {
            quic_listener.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationBlockBuilder,
        ServerConfigurationDirectiveEntry, ServerConfigurationValueBuilder,
    };
    use ferron_observability::transform_observability_alias;

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

        assert_eq!(options.protocols, common::HttpProtocols::default());
        assert_eq!(
            options.alpn_protocols(),
            vec![b"h2".to_vec(), b"http/1.1".to_vec(), b"http/1.0".to_vec()]
        );
        assert!(!options.h1_enable_early_hints);
        assert_eq!(options.h2, common::Http2Settings::default());
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
            common::HttpProtocols {
                http1: true,
                http2: false,
                http3: false,
            }
        );
        assert_eq!(
            options.alpn_protocols(),
            vec![b"http/1.1".to_vec(), b"http/1.0".to_vec()]
        );
        assert!(options.h1_enable_early_hints);
        assert_eq!(
            options.h2,
            common::Http2Settings {
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
            .directive_str("protocols", vec!["unknown"])
            .build();
        let config = ServerConfigurationBlockBuilder::new()
            .directive("http", http_directive(http_block))
            .build();

        let error = resolve_http_connection_options(&config).unwrap_err();

        assert_eq!(error.to_string(), "Unsupported HTTP protocol 'unknown'");
    }

    #[test]
    fn transform_log_alias_with_path() {
        let log_directive = ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValueBuilder::string(
                "/var/log/access.log",
            )],
            children: Some(
                ServerConfigurationBlockBuilder::new()
                    .directive_str("format", vec!["combined"])
                    .build(),
            ),
            span: None,
        };

        let result = transform_observability_alias("log", &log_directive).unwrap();
        let block = result.expect("Should return a block");

        // Check provider is set to "file"
        assert_eq!(
            block.get_value("provider").and_then(|v| v.as_str()),
            Some("file")
        );

        // Check access_log is set
        assert_eq!(
            block.get_value("access_log").and_then(|v| v.as_str()),
            Some("/var/log/access.log")
        );

        // Check format is preserved
        assert_eq!(
            block.get_value("format").and_then(|v| v.as_str()),
            Some("combined")
        );
    }

    #[test]
    fn transform_log_alias_disabled() {
        let log_directive = ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValueBuilder::boolean(false)],
            children: None,
            span: None,
        };

        let result = transform_observability_alias("log", &log_directive).unwrap();
        assert!(result.is_none(), "Should return None for disabled alias");
    }

    #[test]
    fn transform_error_log_alias_with_path() {
        let error_log_directive = ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValueBuilder::string(
                "/var/log/error.log",
            )],
            children: None,
            span: None,
        };

        let result = transform_observability_alias("error_log", &error_log_directive).unwrap();
        let block = result.expect("Should return a block");

        // Check provider is set to "file"
        assert_eq!(
            block.get_value("provider").and_then(|v| v.as_str()),
            Some("file")
        );

        // Check error_log is set
        assert_eq!(
            block.get_value("error_log").and_then(|v| v.as_str()),
            Some("/var/log/error.log")
        );
    }

    #[test]
    fn transform_console_log_alias() {
        let console_log_directive = ServerConfigurationDirectiveEntry {
            args: vec![],
            children: Some(
                ServerConfigurationBlockBuilder::new()
                    .directive_str("format", vec!["json"])
                    .build(),
            ),
            span: None,
        };

        let result = transform_observability_alias("console_log", &console_log_directive).unwrap();
        let block = result.expect("Should return a block");

        // Check provider is set to "console"
        assert_eq!(
            block.get_value("provider").and_then(|v| v.as_str()),
            Some("console")
        );

        // Check format is preserved
        assert_eq!(
            block.get_value("format").and_then(|v| v.as_str()),
            Some("json")
        );
    }

    #[test]
    fn transform_console_log_alias_disabled() {
        let console_log_directive = ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValueBuilder::boolean(false)],
            children: None,
            span: None,
        };

        let result = transform_observability_alias("console_log", &console_log_directive).unwrap();
        assert!(result.is_none(), "Should return None for disabled alias");
    }

    #[test]
    fn global_observability_console_log_alias() {
        // Test that console_log in global config is transformed correctly
        let global_config = ServerConfigurationBlockBuilder::new()
            .directive(
                "console_log",
                ServerConfigurationDirectiveEntry {
                    args: vec![],
                    children: Some(
                        ServerConfigurationBlockBuilder::new()
                            .directive_str("format", vec!["json"])
                            .build(),
                    ),
                    span: None,
                },
            )
            .build();

        let extractor = ObservabilityConfigExtractor::new(&global_config);
        let blocks = extractor.extract_observability_blocks().unwrap();

        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].get_value("provider").and_then(|v| v.as_str()),
            Some("console")
        );
        assert_eq!(
            blocks[0].get_value("format").and_then(|v| v.as_str()),
            Some("json")
        );
    }

    #[test]
    fn global_observability_log_alias() {
        // Test that log in global config is transformed correctly
        let global_config = ServerConfigurationBlockBuilder::new()
            .directive(
                "log",
                ServerConfigurationDirectiveEntry {
                    args: vec![ServerConfigurationValueBuilder::string(
                        "/var/log/access.log",
                    )],
                    children: Some(
                        ServerConfigurationBlockBuilder::new()
                            .directive_str("format", vec!["combined"])
                            .build(),
                    ),
                    span: None,
                },
            )
            .build();

        let extractor = ObservabilityConfigExtractor::new(&global_config);
        let blocks = extractor.extract_observability_blocks().unwrap();

        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].get_value("provider").and_then(|v| v.as_str()),
            Some("file")
        );
        assert_eq!(
            blocks[0].get_value("access_log").and_then(|v| v.as_str()),
            Some("/var/log/access.log")
        );
        assert_eq!(
            blocks[0].get_value("format").and_then(|v| v.as_str()),
            Some("combined")
        );
    }

    #[test]
    fn global_observability_error_log_alias() {
        // Test that error_log in global config is transformed correctly
        let global_config = ServerConfigurationBlockBuilder::new()
            .directive(
                "error_log",
                ServerConfigurationDirectiveEntry {
                    args: vec![ServerConfigurationValueBuilder::string(
                        "/var/log/error.log",
                    )],
                    children: None,
                    span: None,
                },
            )
            .build();

        let extractor = ObservabilityConfigExtractor::new(&global_config);
        let blocks = extractor.extract_observability_blocks().unwrap();

        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].get_value("provider").and_then(|v| v.as_str()),
            Some("file")
        );
        assert_eq!(
            blocks[0].get_value("error_log").and_then(|v| v.as_str()),
            Some("/var/log/error.log")
        );
    }

    #[test]
    fn global_observability_explicit_block() {
        // Test that explicit observability block in global config works
        let global_config = ServerConfigurationBlockBuilder::new()
            .directive(
                "observability",
                ServerConfigurationDirectiveEntry {
                    args: vec![],
                    children: Some(
                        ServerConfigurationBlockBuilder::new()
                            .directive_str("provider", vec!["file"])
                            .directive_str("access_log", vec!["/var/log/http.log"])
                            .build(),
                    ),
                    span: None,
                },
            )
            .build();

        let extractor = ObservabilityConfigExtractor::new(&global_config);
        let blocks = extractor.extract_observability_blocks().unwrap();

        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].get_value("provider").and_then(|v| v.as_str()),
            Some("file")
        );
        assert_eq!(
            blocks[0].get_value("access_log").and_then(|v| v.as_str()),
            Some("/var/log/http.log")
        );
    }

    #[test]
    fn global_observability_multiple_aliases() {
        // Test that multiple alias directives in global config are all extracted
        let global_config = ServerConfigurationBlockBuilder::new()
            .directive(
                "log",
                ServerConfigurationDirectiveEntry {
                    args: vec![ServerConfigurationValueBuilder::string(
                        "/var/log/access.log",
                    )],
                    children: None,
                    span: None,
                },
            )
            .directive(
                "error_log",
                ServerConfigurationDirectiveEntry {
                    args: vec![ServerConfigurationValueBuilder::string(
                        "/var/log/error.log",
                    )],
                    children: None,
                    span: None,
                },
            )
            .build();

        let extractor = ObservabilityConfigExtractor::new(&global_config);
        let blocks = extractor.extract_observability_blocks().unwrap();

        assert_eq!(blocks.len(), 2);

        // Check first block (log)
        assert_eq!(
            blocks[0].get_value("provider").and_then(|v| v.as_str()),
            Some("file")
        );
        assert_eq!(
            blocks[0].get_value("access_log").and_then(|v| v.as_str()),
            Some("/var/log/access.log")
        );

        // Check second block (error_log)
        assert_eq!(
            blocks[1].get_value("provider").and_then(|v| v.as_str()),
            Some("file")
        );
        assert_eq!(
            blocks[1].get_value("error_log").and_then(|v| v.as_str()),
            Some("/var/log/error.log")
        );
    }
}
