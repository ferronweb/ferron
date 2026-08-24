//! Module loader implementation

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use ferron_core::builtin::BuiltinConfigurationValidator;
use ferron_core::config::ServerConfigurationPort;
use ferron_core::directives::{Directive, DirectiveRegistry, DirectiveSubblock};
use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;
use ferron_http::HttpContext;

use crate::server::BasicHttpModule;
#[cfg(unix)]
use crate::server::UnixHttpModule;
use crate::stages::{ClientIpFromHeaderStage, HttpsRedirectStage};
use crate::validator::HttpConfigurationValidator;

/// Default HTTP port when not explicitly configured.
const DEFAULT_HTTP_PORT: u16 = 80;
/// Default HTTPS port when not explicitly configured.
const DEFAULT_HTTPS_PORT: u16 = 443;

/// Resolve the default HTTP port from global configuration.
/// Returns `None` if `default_http_port false` is set.
fn resolve_default_http_port(config: &ferron_core::config::ServerConfiguration) -> Option<u16> {
    match config
        .global_config
        .directives
        .get("default_http_port")
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.args.first())
    {
        Some(v) => {
            if let Some(b) = v.as_boolean() {
                // `false` means disabled, `true` would be odd but use default
                if b {
                    Some(DEFAULT_HTTP_PORT)
                } else {
                    None
                }
            } else {
                v.as_number().and_then(|n| u16::try_from(n).ok())
            }
        }
        None => Some(DEFAULT_HTTP_PORT),
    }
}

/// Resolve the default HTTPS port from global configuration.
/// Returns `None` if `default_https_port false` is set.
fn resolve_default_https_port(config: &ferron_core::config::ServerConfiguration) -> Option<u16> {
    match config
        .global_config
        .directives
        .get("default_https_port")
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.args.first())
    {
        Some(v) => {
            if let Some(b) = v.as_boolean() {
                if b {
                    Some(DEFAULT_HTTPS_PORT)
                } else {
                    None
                }
            } else {
                v.as_number().and_then(|n| u16::try_from(n).ok())
            }
        }
        None => Some(DEFAULT_HTTPS_PORT),
    }
}

#[derive(Default)]
pub struct BasicHttpModuleLoader {
    cache: HashMap<u16, Arc<BasicHttpModule>>,
    #[cfg(unix)]
    unix_cache: Option<Arc<UnixHttpModule>>,
}

impl ModuleLoader for BasicHttpModuleLoader {
    fn register_per_protocol_configuration_blocks<'a>(
        &mut self,
        config: &'a ferron_core::config::ServerConfiguration,
        registry: &mut HashMap<
            &'static str,
            Vec<(String, &'a ferron_core::config::ServerConfigurationBlock)>,
        >,
    ) {
        let default_port = resolve_default_http_port(config);
        let mut blocks = Vec::new();
        let mut pending_blocks = VecDeque::new();
        if let Some(ports) = config.ports.get("http") {
            for port in ports {
                // Skip host blocks that won't create any listeners
                let effective_port = port.port.or(default_port);
                let Some(effective_port) = effective_port else {
                    // Both defaults disabled and no explicit port, skip
                    continue;
                };

                for (filters, host) in &port.hosts {
                    let block_name = match (&filters.host, &filters.ip) {
                        (Some(hostname), Some(ip)) => {
                            format!("port {} host {} ip {}", effective_port, hostname, ip)
                        }
                        (Some(hostname), None) => {
                            format!("port {} host {}", effective_port, hostname)
                        }
                        (None, Some(ip)) => {
                            format!("port {} ip {}", effective_port, ip)
                        }
                        (None, None) => {
                            format!("port {}", effective_port)
                        }
                    };
                    blocks.push((block_name.clone(), host));
                    // Check "if", "if_not", "location", "handle_error" subblocks to pass into validator
                    let subblock_names: &'static [&'static str] =
                        &["if", "if_not", "location", "handle_error"];
                    for subblock_name in subblock_names {
                        if let Some(subblocks) = host.directives.get(*subblock_name) {
                            for subblock in subblocks {
                                if let Some(children) = &subblock.children {
                                    pending_blocks.push_back((
                                        format!("{} {}", block_name, subblock_name),
                                        children,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        while let Some((block_name, host)) = pending_blocks.pop_front() {
            blocks.push((block_name.clone(), host));
            let subblock_names: &'static [&'static str] =
                &["if", "if_not", "location", "handle_error"];
            for subblock_name in subblock_names {
                if let Some(subblocks) = host.directives.get(*subblock_name) {
                    for subblock in subblocks {
                        if let Some(children) = &subblock.children {
                            pending_blocks
                                .push_back((format!("{} {}", block_name, subblock_name), children));
                        }
                    }
                }
            }
        }
        registry.insert("http", blocks);
    }

    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
    ) {
        registry.push(Box::new(HttpConfigurationValidator));
    }

    fn register_per_protocol_configuration_validators(
        &mut self,
        registry: &mut std::collections::HashMap<
            &'static str,
            Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
        >,
    ) {
        registry
            .entry("http")
            .or_default()
            .push(Box::new(HttpConfigurationValidator));
        registry
            .entry("http")
            .or_default()
            .push(Box::new(BuiltinConfigurationValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry
            .with_stage::<HttpContext, _>(|| Arc::new(ClientIpFromHeaderStage))
            .with_stage::<HttpContext, _>(|| Arc::new(HttpsRedirectStage))
    }

    fn register_directives(&mut self, registry: &mut DirectiveRegistry) {
        use ferron_core::directives::DirectiveSubblock;

        register_http_server_base_directives(registry);
        register_http_server_conditional_directives(registry);
        register_http_server_http_protocol_directives(registry);
        register_http_server_trace_directives(registry);
        ferron_tls::directives::register_tls_common_directives(
            registry,
            DirectiveSubblock::custom("tls"),
            Some(&["http"]),
        );
    }

    fn register_modules(
        &mut self,
        registry: Arc<ferron_core::registry::Registry>,
        modules: &mut Vec<Arc<dyn ferron_core::Module>>,
        config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut new_cache = HashMap::new();
        #[cfg(unix)]
        let has_unix = config
            .global_config
            .directives
            .get("unix")
            .is_some_and(|v| !v.is_empty());
        if let Some(port_configs) = config.ports.get("http").cloned() {
            // Handle global Unix socket listeners (Unix only)
            #[cfg(unix)]
            {
                if has_unix {
                    // Merge all hosts from all http ports for Unix resolver
                    let mut all_hosts = Vec::new();
                    if let Some(port_configs) = config.ports.get("http") {
                        for pc in port_configs {
                            all_hosts.extend(pc.hosts.clone());
                        }
                    }
                    if let Some(cached) = self.unix_cache.take() {
                        cached.reload(&registry, config.global_config.clone(), all_hosts)?;
                        modules.push(cached.clone());
                        self.unix_cache = Some(cached);
                    } else {
                        let unix_module = Arc::new(UnixHttpModule::new(
                            &registry,
                            config.global_config.clone(),
                            all_hosts,
                        )?);
                        modules.push(unix_module.clone());
                        self.unix_cache = Some(unix_module);
                    }
                    self.cache.clear();
                    return Ok(());
                } else {
                    self.unix_cache = None;
                }
            }

            let mut port_configs_new: Vec<ServerConfigurationPort> = Vec::new();

            let default_port = resolve_default_http_port(&config);
            let default_https = resolve_default_https_port(&config);

            // Expand port configs: when no port is specified, create both HTTP and HTTPS
            // entries. Localhost-like hostnames are excluded from the HTTPS listener.
            let mut expanded: Vec<ServerConfigurationPort> = Vec::new();
            for port_config in &port_configs {
                if port_config.port.is_some() {
                    // Explicit port, use as-is (no automatic TLS expansion)
                    expanded.push(port_config.clone());
                } else {
                    // No explicit port, expand based on default port settings
                    let mut http_hosts = Vec::new();
                    let mut https_hosts = Vec::new();

                    for (filters, block) in &port_config.hosts {
                        let hostname = filters.host.as_deref();
                        let ip = filters.ip.map(|s| s.to_string());
                        let auto_selection = crate::tls_auto::select_auto_tls_provider(
                            &registry,
                            hostname,
                            ip.as_deref(),
                        );

                        http_hosts.push((filters.clone(), block.clone()));
                        if auto_selection != crate::tls_auto::TlsAutoSelection::None
                            || block.directives.contains_key("tls")
                        {
                            https_hosts.push((filters.clone(), block.clone()));
                        }
                    }

                    // HTTP listener gets all hosts (including localhost), only if default HTTP port is enabled
                    if let Some(http_port) = default_port {
                        if !http_hosts.is_empty() {
                            let mut http_config = port_config.clone();
                            http_config.port = Some(http_port);
                            http_config.hosts = http_hosts;
                            expanded.push(http_config);
                        }
                    }

                    // HTTPS listener only gets non-localhost hosts, only if default HTTPS port is enabled
                    if let Some(https_port) = default_https {
                        if !https_hosts.is_empty() {
                            let mut https_config = port_config.clone();
                            https_config.port = Some(https_port);
                            https_config.hosts = https_hosts;
                            expanded.push(https_config);
                        }
                    }

                    // Warn if neither default is enabled and no explicit port was set
                    if default_port.is_none() && default_https.is_none() {
                        ferron_core::log_warn!(
                            "Host block without explicit port will be skipped because both default_http_port and default_https_port are disabled"
                        );
                    }
                }
            }

            // Merge port configurations with the same port number.
            for mut port_config in expanded {
                let port = port_config
                    .port
                    .expect("port should be set after expansion");
                if let Some(existing) = port_configs_new.iter_mut().find(|c| c.port == Some(port)) {
                    // Merge hosts
                    let mut new_hosts = Vec::new();
                    for existing_host in existing.hosts.iter_mut() {
                        if let Some((_, new_block)) = port_config
                            .hosts
                            .iter_mut()
                            .find(|(filters, _)| filters == &existing_host.0)
                        {
                            // Merge the configuration blocks
                            let mut merged_block = HashMap::new();
                            merged_block.extend(
                                existing_host
                                    .1
                                    .directives
                                    .iter()
                                    .map(|(k, v)| (k.clone(), v.clone())),
                            );
                            merged_block.extend(
                                new_block
                                    .directives
                                    .iter()
                                    .map(|(k, v)| (k.clone(), v.clone())),
                            );
                            new_block.directives = Arc::new(merged_block);
                        } else {
                            new_hosts.push(existing_host.clone());
                        }
                    }
                    existing.hosts.extend(new_hosts);
                } else {
                    port_configs_new.push(port_config);
                }
            }

            for port_config in port_configs_new {
                let port = port_config.port.expect("invalid HTTP server module state");
                let is_explicit_port = port_configs.iter().any(|pc| pc.port == Some(port));
                let https_port = if is_explicit_port {
                    Some(port) // Same port, redirect stage will skip
                } else {
                    default_https // May be None if default_https_port false
                };

                if let Some(cached) = self.cache.get(&port) {
                    // Configuration reload: update the cached module with new configuration
                    cached.reload(
                        &registry,
                        port_config,
                        config.global_config.clone(),
                        https_port,
                        is_explicit_port,
                    )?;
                    new_cache.insert(port, cached.clone());
                } else {
                    let http_module = Arc::new(BasicHttpModule::new(
                        &registry,
                        port_config,
                        config.global_config.clone(),
                        port,
                        https_port,
                        is_explicit_port,
                    )?);
                    modules.push(http_module.clone());
                    new_cache.insert(port, http_module);
                }
            }
        }
        self.cache = new_cache;
        #[cfg(unix)]
        {
            self.unix_cache = None; // Handled in a separate `#[cfg(unix)]` branch
        }

        Ok(())
    }
}

fn reg(
    registry: &mut DirectiveRegistry,
    name: &'static str,
    usage: &'static str,
    description: &'static str,
    global_only: bool,
    link: Option<DirectiveSubblock>,
    parent: DirectiveSubblock,
) {
    registry.register(
        Directive {
            name,
            usage,
            description,
            applicable_protocols: Some(&["http"]),
            global_only,
            subblock_link: link,
        },
        parent,
    );
}

fn register_http_server_base_directives(registry: &mut DirectiveRegistry) {
    let d = DirectiveSubblock::default();
    reg(registry, "default_http_port", "default_http_port <port>", "This directive specifies the default HTTP port when no port is specified in a host block. Must be a positive integer <= 65535, or false to disable the default HTTP listener entirely. Default: 80", true, None, d);
    reg(registry, "default_https_port", "default_https_port <port>", "This directive specifies the default HTTPS port used for HTTP-to-HTTPS redirects and URL generation. Must be a positive integer <= 65535, or false to disable. Default: 443", true, None, d);
    reg(registry, "tls", "tls [bool] | tls <cert> <key> | tls { ... }", "This directive configures TLS for the host. Accepts a boolean to disable, cert and key paths as a shorthand for the manual provider, or a block with provider-specific configuration.", false, Some(DirectiveSubblock::custom("tls")), d);
    reg(
        registry,
        "root",
        "root <path>",
        "This directive specifies the webroot directory for static file serving.",
        false,
        None,
        d,
    );
    reg(
        registry,
        "disable_symlinks",
        "disable_symlinks <false|true|disable_not_owner>",
        "This directive specifies whether to disable following symlinks in static file serving.",
        false,
        None,
        d,
    );
    reg(registry, "admin_email", "admin_email <email>", "This directive specifies the server administrator's email address. Used in built-in error responses. Interpolation is supported.", false, None, d);
    reg(
        registry,
        "index",
        "index <files>...",
        "This directive specifies the index file names for directory requests.",
        false,
        None,
        d,
    );
    reg(registry, "trailing_slash_redirect", "trailing_slash_redirect [bool]", "This directive specifies whether requests for directories without a trailing slash are redirected to include one.", false, None, d);
    reg(registry, "https_redirect", "https_redirect [bool]", "This directive specifies whether automatic HTTP-to-HTTPS redirects are enabled. Uses 308 Permanent Redirect. Default: true (when TLS is enabled)", false, None, d);
    reg(registry, "client_ip_from_header", "client_ip_from_header <header> { trusted_proxy ... }", "This directive specifies the header to read the client IP from. Supported values: x-forwarded-for, forwarded.", false, Some(DirectiveSubblock::custom("client_ip")), d);
}

fn register_http_server_conditional_directives(registry: &mut DirectiveRegistry) {
    let d = DirectiveSubblock::default();
    reg(registry, "if", "if <condition> { ... }", "This directive defines a conditional block that is evaluated when the given condition matches.", false, Some(d), d);
    reg(registry, "if_not", "if_not <condition> { ... }", "This directive defines a conditional block that is evaluated when the given condition does not match.", false, Some(d), d);
    reg(
        registry,
        "location",
        "location <path> { ... }",
        "This directive defines a location block that matches request path patterns.",
        false,
        Some(d),
        d,
    );
    reg(
        registry,
        "handle_error",
        "handle_error <codes>... { ... }",
        "This directive defines custom error page handling for specific HTTP status codes.",
        false,
        Some(d),
        d,
    );
}

fn register_http_server_http_protocol_directives(registry: &mut DirectiveRegistry) {
    let http = DirectiveSubblock::custom("http");
    reg(
        registry,
        "http",
        "http { ... }",
        "This directive specifies per-host HTTP protocol settings in a nested block.",
        false,
        Some(http),
        DirectiveSubblock::default(),
    );
    reg(registry, "protocols", "protocols <name>...", "This directive specifies the enabled HTTP protocols. Supported values: h1 (HTTP/1.1), h2 (HTTP/2), h3 (HTTP/3). Default: h1 h2 h3", false, None, http);
    reg(registry, "options_allowed_methods", "options_allowed_methods <methods>", "This directive specifies the HTTP methods advertised in the Allow header for OPTIONS * requests. Default: GET, HEAD, POST, OPTIONS", false, None, http);
    reg(registry, "timeout", "timeout <duration>", "This directive specifies the pipeline execution timeout. Accepts a duration string (e.g. 30m, 90s), a number in milliseconds, or false to disable. Default: 5m", false, None, http);
    reg(registry, "url_sanitize", "url_sanitize [bool]", "This directive specifies whether URL path sanitization is enabled. When enabled, dangerous sequences such as path traversal attempts, null bytes, and invalid percent-encodings are removed or normalized. Default: enabled", true, Some(http), http);
    reg(registry, "url_reject_backslash", "url_reject_backslash [bool]", "This directive specifies whether URLs containing backslashes are rejected. When enabled, requests with literal \\ or percent-encoded backslashes in the path are rejected with 400. Default: enabled", true, Some(http), http);
    reg(registry, "h1_enable_early_hints", "h1_enable_early_hints [bool]", "This directive specifies whether HTTP/1.1 early hints (103 Early Hints) support is enabled. Default: disabled", false, None, http);
    reg(
        registry,
        "early_hints",
        "early_hints { ... }",
        "This directive configures 103 Early Hints for HTTP/1.x.",
        false,
        Some(http),
        http,
    );
    reg(
        registry,
        "h2_initial_window_size",
        "h2_initial_window_size <size>",
        "This directive specifies the HTTP/2 initial flow-control window size.",
        false,
        None,
        http,
    );
    reg(
        registry,
        "h2_max_frame_size",
        "h2_max_frame_size <size>",
        "This directive specifies the HTTP/2 maximum frame size.",
        false,
        None,
        http,
    );
    reg(
        registry,
        "h2_max_concurrent_streams",
        "h2_max_concurrent_streams <count>",
        "This directive specifies the HTTP/2 maximum concurrent streams.",
        false,
        None,
        http,
    );
    reg(
        registry,
        "h2_max_header_list_size",
        "h2_max_header_list_size <size>",
        "This directive specifies the HTTP/2 maximum header list size.",
        false,
        None,
        http,
    );
    reg(registry, "h2_enable_connect_protocol", "h2_enable_connect_protocol [bool]", "This directive specifies whether the HTTP/2 extended CONNECT protocol is enabled. Default: disabled", false, None, http);
    reg(
        registry,
        "h3_qpack_max_table_capacity",
        "h3_qpack_max_table_capacity <size>",
        "This directive specifies the maximum QPACK table capacity for HTTP/3.",
        false,
        None,
        http,
    );
    reg(
        registry,
        "h3_qpack_blocked_streams",
        "h3_qpack_blocked_streams [bool]",
        "This directive specifies whether blocked streams are enabled in HTTP/3.",
        false,
        None,
        http,
    );
    reg(
        registry,
        "h3_max_field_section_size",
        "h3_max_field_section_size <size>",
        "This directive specifies the maximum field section size for HTTP/3.",
        false,
        None,
        http,
    );
    reg(registry, "h3_enable_connect_protocol", "h3_enable_connect_protocol [bool]", "This directive specifies whether the HTTP/3 extended CONNECT protocol is enabled. Default: disabled", false, None, http);
    reg(registry, "protocol_proxy", "protocol_proxy [bool]", "This directive specifies whether PROXY protocol v1/v2 parsing is enabled for incoming TCP connections. When enabled, Ferron reads the PROXY protocol header before processing the HTTP request. Default: disabled", false, None, http);
    reg(registry, "trusted_proxy", "trusted_proxy <ip-or-cidr>...", "This directive specifies trusted reverse-proxy IPs or CIDR ranges allowed to supply forwarded client IP headers. Repeatable — each occurrence adds one entry.", false, None, DirectiveSubblock::custom("client_ip"));
}

fn register_http_server_trace_directives(registry: &mut DirectiveRegistry) {
    let trace = DirectiveSubblock::custom("trace");
    reg(
        registry,
        "trace",
        "trace { generate ...; trust_request ... }",
        "This directive configures W3C Trace Context generation and trust settings.",
        false,
        Some(trace),
        DirectiveSubblock::custom("http"),
    );
    reg(
        registry,
        "trace_sampling",
        "trace_sampling <trace_sampling_mode> { ... }",
        "This directive configures trace sampling behavior.",
        false,
        None,
        DirectiveSubblock::custom("http"),
    );
    reg(registry, "generate", "generate [bool]", "This directive specifies whether trace IDs are generated for requests that do not carry a W3C traceparent header. Default: enabled", false, None, trace);
    reg(registry, "trust_request", "trust_request [bool]", "This directive specifies whether the incoming W3C traceparent header is trusted and propagated through the pipeline.", false, None, trace);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{
        ServerConfiguration, ServerConfigurationBlock, ServerConfigurationDirectiveEntry,
        ServerConfigurationHostFilters, ServerConfigurationPort, ServerConfigurationValue,
    };
    use std::collections::{BTreeMap, HashMap as StdHashMap};
    use std::sync::Arc;

    fn make_config_with_directives(
        directives: StdHashMap<String, Vec<ServerConfigurationDirectiveEntry>>,
        ports: BTreeMap<String, Vec<ServerConfigurationPort>>,
    ) -> Arc<ServerConfiguration> {
        Arc::new(ServerConfiguration {
            global_config: Arc::new(ServerConfigurationBlock {
                directives: Arc::new(directives),
                matchers: StdHashMap::new(),
                span: None,
            }),
            ports,
        })
    }

    fn make_host_block(
        hostname: Option<&str>,
        directives: StdHashMap<String, Vec<ServerConfigurationDirectiveEntry>>,
    ) -> (ServerConfigurationHostFilters, ServerConfigurationBlock) {
        let filters = ServerConfigurationHostFilters {
            host: hostname.map(|s| s.to_string()),
            ip: None,
        };
        let block = ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: StdHashMap::new(),
            span: None,
        };
        (filters, block)
    }

    #[test]
    fn test_resolve_default_http_port_number() {
        let mut directives = StdHashMap::new();
        directives.insert(
            "default_http_port".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Number(8080, None)],
                children: None,
                span: None,
            }],
        );
        let config = make_config_with_directives(directives, BTreeMap::new());
        assert_eq!(resolve_default_http_port(&config), Some(8080));
    }

    #[test]
    fn test_resolve_default_http_port_false() {
        let mut directives = StdHashMap::new();
        directives.insert(
            "default_http_port".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(false, None)],
                children: None,
                span: None,
            }],
        );
        let config = make_config_with_directives(directives, BTreeMap::new());
        assert_eq!(resolve_default_http_port(&config), None);
    }

    #[test]
    fn test_resolve_default_http_port_true() {
        let mut directives = StdHashMap::new();
        directives.insert(
            "default_http_port".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(true, None)],
                children: None,
                span: None,
            }],
        );
        let config = make_config_with_directives(directives, BTreeMap::new());
        assert_eq!(resolve_default_http_port(&config), Some(DEFAULT_HTTP_PORT));
    }

    #[test]
    fn test_resolve_default_http_port_missing() {
        let config = make_config_with_directives(StdHashMap::new(), BTreeMap::new());
        assert_eq!(resolve_default_http_port(&config), Some(DEFAULT_HTTP_PORT));
    }

    #[test]
    fn test_resolve_default_https_port_number() {
        let mut directives = StdHashMap::new();
        directives.insert(
            "default_https_port".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Number(8443, None)],
                children: None,
                span: None,
            }],
        );
        let config = make_config_with_directives(directives, BTreeMap::new());
        assert_eq!(resolve_default_https_port(&config), Some(8443));
    }

    #[test]
    fn test_resolve_default_https_port_false() {
        let mut directives = StdHashMap::new();
        directives.insert(
            "default_https_port".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(false, None)],
                children: None,
                span: None,
            }],
        );
        let config = make_config_with_directives(directives, BTreeMap::new());
        assert_eq!(resolve_default_https_port(&config), None);
    }

    #[test]
    fn test_register_blocks_with_disabled_defaults() {
        // Test that host blocks without explicit ports are skipped when both defaults are false
        let mut directives = StdHashMap::new();
        directives.insert(
            "default_http_port".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(false, None)],
                children: None,
                span: None,
            }],
        );
        directives.insert(
            "default_https_port".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(false, None)],
                children: None,
                span: None,
            }],
        );

        let mut ports = BTreeMap::new();
        let host = make_host_block(Some("example.com"), StdHashMap::new());
        ports.insert(
            "http".to_string(),
            vec![ServerConfigurationPort {
                port: None,
                hosts: vec![host],
            }],
        );

        let config = make_config_with_directives(directives, ports);
        let mut loader = BasicHttpModuleLoader::default();
        let mut registry = StdHashMap::new();

        loader.register_per_protocol_configuration_blocks(&config, &mut registry);

        // Should be empty because both defaults are disabled and no explicit port
        assert!(registry.is_empty() || registry.get("http").is_none_or(|v| v.is_empty()));
    }

    #[test]
    fn test_register_blocks_with_explicit_port_and_disabled_defaults() {
        // Test that explicit ports still work when defaults are disabled
        let mut directives = StdHashMap::new();
        directives.insert(
            "default_http_port".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(false, None)],
                children: None,
                span: None,
            }],
        );
        directives.insert(
            "default_https_port".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(false, None)],
                children: None,
                span: None,
            }],
        );

        let mut ports = BTreeMap::new();
        let host = make_host_block(Some("example.com"), StdHashMap::new());
        ports.insert(
            "http".to_string(),
            vec![ServerConfigurationPort {
                port: Some(9090),
                hosts: vec![host],
            }],
        );

        let config = make_config_with_directives(directives, ports);
        let mut loader = BasicHttpModuleLoader::default();
        let mut registry = StdHashMap::new();

        loader.register_per_protocol_configuration_blocks(&config, &mut registry);

        // Should have one block for the explicit port
        let blocks = registry.get("http").expect("http key should exist");
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].0.contains("port 9090"));
    }

    #[test]
    fn test_register_blocks_with_http_enabled_https_disabled() {
        // Test that only HTTP listener is created when HTTPS is disabled
        let mut directives = StdHashMap::new();
        directives.insert(
            "default_https_port".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(false, None)],
                children: None,
                span: None,
            }],
        );

        let mut ports = BTreeMap::new();
        let host = make_host_block(Some("example.com"), StdHashMap::new());
        ports.insert(
            "http".to_string(),
            vec![ServerConfigurationPort {
                port: None,
                hosts: vec![host],
            }],
        );

        let config = make_config_with_directives(directives, ports);
        let mut loader = BasicHttpModuleLoader::default();
        let mut registry = StdHashMap::new();

        loader.register_per_protocol_configuration_blocks(&config, &mut registry);

        // Should have one block for HTTP on default port 80
        let blocks = registry.get("http").expect("http key should exist");
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].0.contains("port 80"));
    }
}
