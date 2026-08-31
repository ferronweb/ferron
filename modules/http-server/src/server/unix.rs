//! Unix domain socket listener and connection handling (Unix only)

use std::collections::HashMap;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ferron_core::runtime::Runtime;
use ferron_core::{log_error, log_info, log_warn};
use ferron_observability::sampler::TraceSampler;
use ferron_observability::{CompositeEventSink, LogAttributeValue};
use ferron_tls::observability::{
    emit_connections_active, emit_handshake_duration, emit_handshake_total,
};
use ferron_tls::TlsConnectionParams;
use rustls::server::Acceptor;
use tokio_util::sync::CancellationToken;

use crate::server::tls_resolve::RadixTree;

use super::common::*;
use super::native_sockets::*;

#[derive(Clone, Debug)]
pub struct UnixListenerOptions {
    pub path: PathBuf,
    pub backlog: Option<i32>,
    pub mode: Option<u32>,
    pub owner: Option<String>,
    pub group: Option<String>,
}

pub struct UnixListenerHandle {
    cancel_token: Arc<CancellationToken>,
    path: PathBuf,
}

impl UnixListenerHandle {
    pub fn new(
        options: UnixListenerOptions,
        http3_alt_svc: bool,
        config: ConfigArcSwap,
        runtime: &mut Runtime,
    ) -> Result<Self, io::Error> {
        let std_listener = build_unix_listener(&options)?;

        if config.load().tls_resolver.is_some() {
            log_info!("HTTPS server listening on unix:{}", options.path.display());
        } else {
            log_info!("HTTP server listening on unix:{}", options.path.display());
        }

        let cancel_token = Arc::new(CancellationToken::new());
        let config_clone = config.clone();
        let cancel_token_clone = cancel_token.clone();
        let path_clone = options.path.clone();

        runtime.spawn_primary_task(move || {
                let new_listener_result = std_listener.try_clone();
                let cancel_token = cancel_token_clone.clone();
                let config = config_clone.clone();
                let unix_path = path_clone.clone();
                Box::pin(async move {
                    let Ok(new_listener) = new_listener_result else {
                        log_error!("Failed to clone Unix listener");
                        return;
                    };
                    let Ok(listener) = zincio::net::UnixListener::from_std(new_listener) else {
                        log_error!("Failed to convert Unix listener to zincio");
                        return;
                    };

                    let mut handle_exhaustion_backoff = Duration::from_millis(10);
                    loop {
                        let accept_result = tokio::select! {
                            res = listener.accept() => res,
                            _ = cancel_token.cancelled() => {
                                let _ = std::fs::remove_file(&unix_path);
                                return;
                            }
                        };
                        let (socket, _addr) = match accept_result {
                            Ok(v) => {
                                handle_exhaustion_backoff = Duration::from_millis(10);
                                v
                            }
                            Err(err) => {
                                let global_observability = resolve_root_observability_sink(
                                    &config.load().observability_resolver,
                                    Some(&TraceSampler::new(&config.load().trace_sampling)),
                                );
                                emit_error(
                                    &global_observability,
                                    format!("Failed to accept Unix connection: {err}"),
                                    vec![
                                        ("error.type", LogAttributeValue::String("unix_accept_error".into())),
                                        ("error.message", LogAttributeValue::String(err.to_string())),
                                        ("server.address", LogAttributeValue::String(unix_path.display().to_string())),
                                    ],
                                );
                                emit_connection_error_metric(&global_observability, "unix", "accept");
                                if err.raw_os_error() == Some(24) {
                                    zincio::time::sleep(handle_exhaustion_backoff).await;
                                    handle_exhaustion_backoff = handle_exhaustion_backoff.saturating_mul(2);
                                    if handle_exhaustion_backoff > Duration::from_secs(1) {
                                        handle_exhaustion_backoff = Duration::from_secs(1);
                                    }
                                }
                                continue;
                            }
                        };

                        let Ok(socket) = socket.into_poll() else {
                            let global_observability = resolve_root_observability_sink(
                                &config.load().observability_resolver,
                                Some(&TraceSampler::new(&config.load().trace_sampling)),
                            );
                            emit_error(
                                &global_observability,
                                "Failed to convert Unix socket to poll-based I/O",
                                vec![("error.type", LogAttributeValue::String("unix_socket_setup_error".into()))],
                            );
                            emit_connection_error_metric(&global_observability, "unix", "socket_setup");
                            continue;
                        };

                        let server_config = config.load_full();
                        let connection_cancel_token = cancel_token.clone();
                        let unix_path_clone = unix_path.clone();
                        zincio::spawn_detached(async move {
                            let _conn_guard = ConnectionCountGuard::new();

                            let ip_observability = resolve_observability_sink(
                                &server_config.observability_resolver,
                                None,
                                None,
                                &CompositeEventSink::with_sampler(
                                    vec![],
                                    Some(TraceSampler::new(&server_config.trace_sampling)),
                                ),
                            );

                            if let Some(tls_resolver) = &server_config.tls_resolver {
                                // TLS over Unix socket
                                let start_handshake =
                                    match tokio_rustls::LazyConfigAcceptor::new(Acceptor::default(), socket.into()).await
                                    {
                                        Ok(h) => h,
                                        Err(e) => {
                                            emit_error(
                                                &ip_observability,
                                                format!("Failed to start TLS handshake on unix:{} {e}", unix_path_clone.display()),
                                                vec![
                                                    ("error.type", LogAttributeValue::String("unix_tls_handshake_error".into())),
                                                    ("error.message", LogAttributeValue::String(e.to_string())),
                                                    ("server.address", LogAttributeValue::String(unix_path_clone.display().to_string())),
                                                ],
                                            );
                                            emit_connection_error_metric(&ip_observability, "unix", "tls_handshake");
                                            return;
                                        }
                                    };
                                let sni = start_handshake
                                    .client_hello()
                                    .server_name()
                                    .map(ToOwned::to_owned);
                                let hinted_hostname = sni.as_deref().and_then(normalize_host_for_lookup);
                                let connection_options = resolve_http_connection_options_opt(
                                    &server_config.http_connection_options_resolver,
                                    None,
                                    hinted_hostname.as_deref(),
                                );
                                let resolver = match sni.as_deref() {
                                    Some(sni) => tls_resolver.lookup_hostname(sni),
                                    None => tls_resolver.root_data(),
                                };
                                // Also try ip+hostname variant if ip were Some: for unix ip is None, so lookup above is correct.
                                // For host-filtered UDS, lookup_hostname is sufficient.

                                if let Some(resolver) = resolver {
                                    let handshake_start = Instant::now();
                                    let tls_stream_option = match resolver.handshake(start_handshake).await {
                                        Ok(s) => s,
                                        Err(e) => {
                                            let handshake_duration = handshake_start.elapsed();
                                            let host = hinted_hostname.clone().unwrap_or_else(|| "_global".to_string());
                                            let tls_observability = resolve_observability_sink(
                                                &server_config.observability_resolver,
                                                None,
                                                hinted_hostname.as_deref(),
                                                &ip_observability,
                                            );
                                            let mut error_message = format!("Failed to start TLS handshake: {e}");
                                            let mut attrs = vec![
                                                ("error.type", LogAttributeValue::String("unix_tls_handshake_error".into())),
                                                ("error.message", LogAttributeValue::String(e.to_string())),
                                                ("server.address", LogAttributeValue::String(unix_path_clone.display().to_string())),
                                            ];
                                            if e.to_string().to_lowercase().contains("resolve")
                                                || e.to_string().to_lowercase().contains("resolution")
                                            {
                                                if let Some(cause) = resolver.get_tls_background_error() {
                                                    error_message.push_str(&format!("\nPossible cause: {cause}"));
                                                    attrs.push(("ferron.error.possible_cause", LogAttributeValue::String(cause.to_string())));
                                                }
                                            }
                                            emit_error(&tls_observability, error_message, attrs);
                                            emit_connection_error_metric(&tls_observability, "unix", "tls_handshake");
                                            emit_handshake_duration(&tls_observability, &host, handshake_duration, "unknown", "unknown", "error");
                                            emit_handshake_total(&tls_observability, &host, "error");
                                            return;
                                        }
                                    };
                                    let handshake_duration = handshake_start.elapsed();
                                    let host = hinted_hostname.clone().unwrap_or_else(|| "_global".to_string());
                                    let tls_observability = resolve_observability_sink(
                                        &server_config.observability_resolver,
                                        None,
                                        hinted_hostname.as_deref(),
                                        &ip_observability,
                                    );
                                    if let Some(tls_stream) = tls_stream_option {
                                        let peer_identity = tls_stream
                                            .get_ref()
                                            .1
                                            .peer_certificates()
                                            .filter(|c| !c.is_empty())
                                            .map(|c| c.to_vec());
                                        let negotiated_protocol = tls_stream
                                            .get_ref()
                                            .1
                                            .alpn_protocol()
                                            .map(|p| p.to_vec());
                                        let protocol_version_str = tls_stream
                                            .get_ref()
                                            .1
                                            .protocol_version()
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("unknown")
                                            .replace('_', ".");
                                        let cipher_suite_str = tls_stream
                                            .get_ref()
                                            .1
                                            .negotiated_cipher_suite()
                                            .map(|cs| cs.suite())
                                            .and_then(|cs| cs.as_str())
                                            .unwrap_or("unknown")
                                            .to_string();
                                        emit_handshake_duration(
                                            &tls_observability,
                                            &host,
                                            handshake_duration,
                                            &protocol_version_str,
                                            &cipher_suite_str,
                                            "success",
                                        );
                                        emit_handshake_total(&tls_observability, &host, "success");
                                        emit_connections_active(&tls_observability, &host, 1);
                                        let tls_params = TlsConnectionParams {
                                            protocol_version: protocol_version_str,
                                            cipher_suite: cipher_suite_str,
                                        };
                                        if negotiated_protocol.as_deref() == Some(b"h2".as_slice()) {
                                            handle_http2_connection(
                                                tls_stream,
                                                ConnectionAddr::Unix { unix_socket_path: unix_path_clone.clone() },
                                                server_config.pipeline.clone(),
                                                server_config.file_pipeline.clone(),
                                                server_config.error_pipeline.clone(),
                                                server_config.config_resolver.clone(),
                                                hinted_hostname.clone(),
                                                true,
                                                server_config.https_port,
                                                connection_options,
                                                server_config.observability_resolver.clone(),
                                                tls_observability.clone(),
                                                (*connection_cancel_token).clone(),
                                                server_config.reload_token.clone(),
                                                http3_alt_svc,
                                                peer_identity,
                                                Some(tls_params),
                                            )
                                            .await;
                                        } else if connection_options.protocols.http1 {
                                            handle_http1_connection(
                                                tls_stream,
                                                ConnectionAddr::Unix { unix_socket_path: unix_path_clone.clone() },
                                                server_config.pipeline.clone(),
                                                server_config.file_pipeline.clone(),
                                                server_config.error_pipeline.clone(),
                                                server_config.config_resolver.clone(),
                                                hinted_hostname.clone(),
                                                true,
                                                server_config.https_port,
                                                connection_options,
                                                server_config.observability_resolver.clone(),
                                                tls_observability.clone(),
                                                (*connection_cancel_token).clone(),
                                                server_config.reload_token.clone(),
                                                http3_alt_svc,
                                                peer_identity,
                                                Some(tls_params),
                                            )
                                            .await;
                                        } else {
                                            emit_error(
                                                &tls_observability,
                                                "TLS connection did not negotiate a supported HTTP protocol",
                                                vec![("error.type", LogAttributeValue::String("unix_tls_protocol_error".into()))],
                                            );
                                        }
                                        emit_connections_active(&tls_observability, &host, -1);
                                    }
                                } else {
                                    // No resolver for SNI, try anonymous config
                                    if let Ok(b) = rustls::ServerConfig::builder_with_provider(Arc::new(
                                        rustls::crypto::aws_lc_rs::default_provider(),
                                    ))
                                    .with_safe_default_protocol_versions()
                                    {
                                        let tls_config = b.with_no_client_auth().with_cert_resolver(Arc::new(NoCertResolver));
                                        if let Err(e) = start_handshake.into_stream(Arc::new(tls_config)).await {
                                            let tls_observability = resolve_observability_sink(
                                                &server_config.observability_resolver,
                                                None,
                                                hinted_hostname.as_deref(),
                                                &ip_observability,
                                            );
                                            emit_error(
                                                &tls_observability,
                                                format!("Failed to start TLS handshake: {e}"),
                                                vec![
                                                    ("error.type", LogAttributeValue::String("unix_tls_handshake_error".into())),
                                                    ("error.message", LogAttributeValue::String(e.to_string())),
                                                    ("server.address", LogAttributeValue::String(unix_path_clone.display().to_string())),
                                                ],
                                            );
                                        }
                                    }
                                }
                            } else {
                                let connection_options = resolve_http_connection_options_opt(
                                    &server_config.http_connection_options_resolver,
                                    None,
                                    None,
                                );
                                if connection_options.protocols.http2_cleartext {
                                handle_http2_connection(
                                    socket,
                                    ConnectionAddr::Unix { unix_socket_path: unix_path_clone.clone() },
                                    server_config.pipeline.clone(),
                                    server_config.file_pipeline.clone(),
                                    server_config.error_pipeline.clone(),
                                    server_config.config_resolver.clone(),
                                    None,
                                    false,
                                    server_config.https_port,
                                    connection_options,
                                    server_config.observability_resolver.clone(),
                                    ip_observability,
                                    (*connection_cancel_token).clone(),
                                    server_config.reload_token.clone(),
                                    http3_alt_svc,
                                    None,
                                    None
                                )
                                .await;
                                } else if connection_options.protocols.http1 {
                                handle_http1_connection_zerocopy(
                                    socket,
                                    ConnectionAddr::Unix { unix_socket_path: unix_path_clone.clone() },
                                    server_config.pipeline.clone(),
                                    server_config.file_pipeline.clone(),
                                    server_config.error_pipeline.clone(),
                                    server_config.config_resolver.clone(),
                                    None,
                                    false,
                                    server_config.https_port,
                                    connection_options,
                                    server_config.observability_resolver.clone(),
                                    ip_observability,
                                    (*connection_cancel_token).clone(),
                                    server_config.reload_token.clone(),
                                    http3_alt_svc,
                                    None
                                )
                                .await;
                                } else {
                                    emit_error(
                                        &ip_observability,
                                        "Unix listener requires HTTP/1.x or h2c support",
                                        vec![
                                            ("error.type", LogAttributeValue::String("unix_http1_required".into())),
                                            ("server.address", LogAttributeValue::String(unix_path_clone.display().to_string())),
                                        ],
                                    );
                                }
                            }
                        });
                    }
                })
            });

        Ok(Self {
            cancel_token,
            path: options.path,
        })
    }

    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }
}

impl Drop for UnixListenerHandle {
    fn drop(&mut self) {
        // Best-effort removal is done in the spawned task on cancel; also try here
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn resolve_unix_listener_options(
    global_config: &ferron_core::config::ServerConfigurationBlock,
) -> anyhow::Result<Vec<UnixListenerOptions>> {
    let Some(entries) = global_config.directives.get("unix") else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let raw = entry
            .args
            .first()
            .and_then(|v| v.as_string_with_interpolations(&HashMap::<String, String>::new()))
            .ok_or_else(|| anyhow::anyhow!("unix directive requires a string path"))?;
        let raw = raw.trim();
        if raw.is_empty() {
            anyhow::bail!("unix directive path cannot be empty");
        }
        if raw.contains('\0') {
            anyhow::bail!("unix socket path contains NUL byte");
        }

        let mut path = PathBuf::from(raw);
        if !path.is_absolute() {
            if let Ok(new_path) = path.canonicalize() {
                path = new_path;
            }
        }

        // Length check (sun_path max ~108)
        let path_str = path.to_string_lossy();
        if path_str.len() >= 108 {
            anyhow::bail!("unix socket path too long ({} >= 108)", path_str.len());
        }

        let children = entry.children.as_ref();
        let backlog = children
            .and_then(|c| c.get_value("backlog"))
            .and_then(|v| v.as_number())
            .map(|n| i32::try_from(n).unwrap_or(-1));
        let mode = children
            .and_then(|c| c.get_value("mode"))
            .map(parse_mode_value)
            .transpose()
            .map_err(|e| anyhow::anyhow!("unix mode: {e}"))?;
        let owner = children
            .and_then(|c| c.get_value("owner"))
            .and_then(|v| v.as_string_with_interpolations(&HashMap::<String, String>::new()));
        let group = children
            .and_then(|c| c.get_value("group"))
            .and_then(|v| v.as_string_with_interpolations(&HashMap::<String, String>::new()));

        out.push(UnixListenerOptions {
            path,
            backlog,
            mode,
            owner: owner
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            group: group
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        });
    }
    // Deduplicate check
    let mut seen = std::collections::HashSet::new();
    for opt in &out {
        if !seen.insert(opt.path.clone()) {
            anyhow::bail!("duplicate unix socket path '{}'", opt.path.display());
        }
    }
    Ok(out)
}

fn parse_mode_value(val: &ferron_core::config::ServerConfigurationValue) -> anyhow::Result<u32> {
    match val {
        ferron_core::config::ServerConfigurationValue::Number(n, _) => {
            if *n < 0 || *n > 0o777 {
                anyhow::bail!("mode number must be 0..0o777 (0..511), got {n}");
            }
            Ok(*n as u32)
        }
        ferron_core::config::ServerConfigurationValue::String(s, _) => parse_mode_str(s),
        ferron_core::config::ServerConfigurationValue::InterpolatedString(parts, _) => {
            // For interpolated, we cannot resolve at this point without variables; treat as string and try parse its literal parts
            // Reconstruct as string of literal parts only
            let mut s = String::new();
            for p in parts {
                match p {
                    ferron_core::config::ServerConfigurationInterpolatedStringPart::String(lit) => {
                        s.push_str(lit)
                    }
                    ferron_core::config::ServerConfigurationInterpolatedStringPart::Variable(
                        _v,
                    ) => {
                        // Cannot validate variable mode at config build time; skip check
                        return Ok(0o666);
                    }
                }
            }
            parse_mode_str(&s)
        }
        _ => anyhow::bail!("mode must be a string like \"0660\" or number"),
    }
}

fn parse_mode_str(s: &str) -> anyhow::Result<u32> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        anyhow::bail!("empty mode");
    }
    let octal_str = if trimmed.starts_with("0o") || trimmed.starts_with("0O") {
        &trimmed[2..]
    } else {
        trimmed
    };
    if octal_str.is_empty() || !octal_str.chars().all(|c| ('0'..='7').contains(&c)) {
        anyhow::bail!("mode must be octal digits (0-7), got '{s}'");
    }
    let mode =
        u32::from_str_radix(octal_str, 8).map_err(|e| anyhow::anyhow!("invalid octal: {e}"))?;
    if mode > 0o777 {
        anyhow::bail!("mode must be <= 0o777, got {mode:o}");
    }
    Ok(mode)
}

fn build_unix_listener(
    options: &UnixListenerOptions,
) -> io::Result<std::os::unix::net::UnixListener> {
    let path = &options.path;
    // Remove stale socket file if it's a socket
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt;
            if meta.file_type().is_socket() {
                let _ = std::fs::remove_file(path);
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("path {} already exists and is not a socket", path.display()),
                ));
            }
        }
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    // Use socket2 to allow backlog customization
    let socket = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)?;
    let addr = socket2::SockAddr::unix(path)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    // Allow reuse? not needed for unix
    socket.bind(&addr)?;
    let backlog = options.backlog.unwrap_or(-1);
    socket.listen(backlog)?;
    // Convert to std listener
    let std_listener: std::os::unix::net::UnixListener = socket.into();

    // Apply permissions
    if let Some(mode) = options.mode {
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(mode);
        std::fs::set_permissions(path, perms)?;
    }
    if options.owner.is_some() || options.group.is_some() {
        if let Err(e) = apply_ownership(path, options.owner.as_deref(), options.group.as_deref()) {
            log_warn!("Failed to chown {}: {}", path.display(), e);
        }
    }
    Ok(std_listener)
}

fn apply_ownership(path: &Path, owner: Option<&str>, group: Option<&str>) -> io::Result<()> {
    let uid = match owner {
        Some(s) => Some(resolve_uid(s)?),
        None => None,
    };
    let gid = match group {
        Some(s) => Some(resolve_gid(s)?),
        None => None,
    };
    if uid.is_none() && gid.is_none() {
        return Ok(());
    }
    let uid_raw = uid.unwrap_or(u32::MAX);
    let gid_raw = gid.unwrap_or(u32::MAX);
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let ret = unsafe { libc::chown(c_path.as_ptr(), uid_raw, gid_raw) };
    if ret != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn resolve_uid(s: &str) -> io::Result<u32> {
    if let Ok(num) = s.parse::<u32>() {
        return Ok(num);
    }
    let cname = std::ffi::CString::new(s)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "owner contains NUL"))?;
    unsafe {
        let pwd = libc::getpwnam(cname.as_ptr());
        if pwd.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("user '{s}' not found"),
            ));
        }
        Ok((*pwd).pw_uid)
    }
}

fn resolve_gid(s: &str) -> io::Result<u32> {
    if let Ok(num) = s.parse::<u32>() {
        return Ok(num);
    }
    let cname = std::ffi::CString::new(s)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "group contains NUL"))?;
    unsafe {
        let grp = libc::getgrnam(cname.as_ptr());
        if grp.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("group '{s}' not found"),
            ));
        }
        Ok((*grp).gr_gid)
    }
}

#[inline]
pub(crate) fn resolve_http_connection_options_opt(
    resolver: &RadixTree<HttpConnectionOptions>,
    ip: Option<std::net::IpAddr>,
    hostname: Option<&str>,
) -> HttpConnectionOptions {
    let normalized_hostname = hostname.and_then(normalize_host_for_lookup);
    let opt = match (ip, normalized_hostname.as_deref()) {
        (Some(ip), Some(host)) => resolver
            .lookup_ip_and_hostname(ip, host)
            .or_else(|| resolver.lookup_ip(ip)),
        (Some(ip), None) => resolver.lookup_ip(ip),
        (None, Some(host)) => resolver.lookup_hostname(host),
        (None, None) => resolver.root_data(),
    };
    opt.or_else(|| resolver.root_data()).unwrap_or_default()
}
