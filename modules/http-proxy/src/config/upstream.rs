use std::error::Error;
#[cfg(feature = "srv-lookup")]
use std::net::IpAddr;
use std::time::Duration;

use ferron_core::config::{ServerConfigurationDirectiveEntry, ServerConfigurationValue};
use rustls::pki_types::pem::PemObject;

use super::resilience::parse_active_health_check;
use super::types::ProxyConfig;
use super::{DEFAULT_CONNECTION_TIMEOUT_MS, DEFAULT_KEEPALIVE_IDLE_TIMEOUT_MS, MTLS_FILE_CACHE};
use crate::types::health::UpstreamHealthCheckConfig;
#[cfg(feature = "srv-lookup")]
use crate::types::upstream::SrvUpstreamData;
use crate::types::upstream::{MtlsCredentials, Upstream, UpstreamConfig};

pub(super) fn parse_upstream_entry(
    entry: &ServerConfigurationDirectiveEntry,
    cfg: &mut ProxyConfig,
    ctx: &ferron_http::HttpContext,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let url = entry
        .args
        .first()
        .and_then(|v| v.as_string_with_interpolations(ctx))
        .ok_or("upstream requires a URL argument")?;

    let mut limit: Option<usize> = None;
    let mut idle_timeout: Option<Duration> = None;
    let mut connection_timeout: Option<Duration> = None;
    let mut connection_timeout_disabled: bool = false;
    let mut unix_socket: Option<String> = None;
    let mut health_check_config = UpstreamHealthCheckConfig::default();
    let mut weight: u32 = 1;
    let mut priority: u16 = 0;
    let mut mtls_cert: Option<Vec<rustls::pki_types::CertificateDer<'static>>> = None;
    let mut mtls_key: Option<rustls::pki_types::PrivateKeyDer<'static>> = None;
    let mut logical_dns: bool = false;
    let mut dns_servers: Vec<std::net::IpAddr> = Vec::new();

    if let Some(block) = &entry.children {
        for (name, entries) in block.directives.iter() {
            match name.as_str() {
                "cert" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v| v.as_string_with_interpolations(ctx))
                    {
                        mtls_cert = Some(
                            rustls::pki_types::CertificateDer::pem_slice_iter(
                                &read_mtls_data(&val).map_err(|e| {
                                    let e: Box<dyn Error + Send + Sync> = format!(
                                        "Can't read mTLS certificate for reverse proxy: {e}"
                                    )
                                    .into();
                                    e
                                })?,
                            )
                            .collect::<Result<Vec<_>, _>>()
                            .map_err(|e| {
                                let e: Box<dyn Error + Send + Sync> =
                                    format!("Can't read mTLS certificate for reverse proxy: {e}")
                                        .into();
                                e
                            })?,
                        );
                    }
                }
                "key" => {
                    if let Some(val) = entries.first().and_then(|e| e.args.first()).and_then(
                        |v: &ServerConfigurationValue| v.as_string_with_interpolations(ctx),
                    ) {
                        mtls_key = Some(
                            rustls::pki_types::PrivateKeyDer::from_pem_slice(
                                &read_mtls_data(&val).map_err(|e| {
                                    let e: Box<dyn Error + Send + Sync> = format!(
                                        "Can't read mTLS private key for reverse proxy: {e}"
                                    )
                                    .into();
                                    e
                                })?,
                            )
                            .map_err(|e| {
                                let e: Box<dyn Error + Send + Sync> =
                                    format!("Can't read mTLS private key for reverse proxy: {e}")
                                        .into();
                                e
                            })?,
                        );
                    }
                }
                "limit" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v: &ServerConfigurationValue| v.as_number())
                    {
                        if val > 0 {
                            limit = Some(val as usize);
                        }
                    }
                }
                "idle_timeout" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v| v.as_duration())
                    {
                        idle_timeout = Some(val);
                    }
                }
                "connection_timeout" => {
                    if let Some(val) = entries.first().and_then(|e| e.args.first()) {
                        if val.as_boolean() == Some(false) {
                            connection_timeout_disabled = true;
                        } else if let Some(duration) = val.as_duration() {
                            connection_timeout = Some(duration);
                        }
                    }
                }
                "unix" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v| v.as_string_with_interpolations(ctx))
                    {
                        unix_socket = Some(val);
                    }
                }
                "weight" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v: &ServerConfigurationValue| v.as_number())
                    {
                        if val > 0 {
                            weight = val as u32;
                        }
                    }
                }
                "priority" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v: &ServerConfigurationValue| v.as_number())
                    {
                        priority = val as u16;
                    }
                }
                "active_check" => {
                    if let Some(val) = entries.first().map(|e| e.get_flag()) {
                        health_check_config.enabled = val;
                        if val {
                            if let Some(children) =
                                entries.first().and_then(|e| e.children.as_ref())
                            {
                                parse_active_health_check(children, &mut health_check_config)?;
                            }
                        }
                    }
                }
                "logical_dns" => {
                    if let Some(val) = entries.first().map(|e| e.get_flag()) {
                        logical_dns = val;
                    }
                }
                "dns_servers" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v| v.as_str())
                    {
                        dns_servers = val
                            .split(',')
                            .filter_map(|s| s.trim().parse().ok())
                            .collect();
                    }
                }
                _ => {}
            }
        }
    }

    if idle_timeout.is_none() {
        idle_timeout = Some(Duration::from_millis(DEFAULT_KEEPALIVE_IDLE_TIMEOUT_MS));
    }

    if connection_timeout.is_none() && !connection_timeout_disabled {
        connection_timeout = Some(Duration::from_millis(DEFAULT_CONNECTION_TIMEOUT_MS));
    }

    let mtls = if let (Some(certs), Some(key)) = (mtls_cert, mtls_key) {
        Some(std::sync::Arc::new(MtlsCredentials { certs, key }))
    } else {
        None
    };
    cfg.upstreams.push(Upstream::Static(UpstreamConfig {
        url: url.clone(),
        unix_socket,
        limit,
        health_check_config,
        weight,
        mtls,
        priority,
        logical_dns,
        dns_servers,
        connection_timeout,
        idle_timeout: idle_timeout
            .unwrap_or(Duration::from_millis(DEFAULT_KEEPALIVE_IDLE_TIMEOUT_MS)),
    }));

    Ok(())
}

#[cfg(feature = "srv-lookup")]
pub(super) fn parse_srv_entry(
    entry: &ServerConfigurationDirectiveEntry,
    cfg: &mut ProxyConfig,
    ctx: &ferron_http::HttpContext,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let srv_name = entry
        .args
        .first()
        .and_then(|v| v.as_string_with_interpolations(ctx))
        .ok_or("srv requires an SRV record name argument")?;

    let mut limit: Option<usize> = None;
    let mut idle_timeout: Option<Duration> = None;
    let mut connection_timeout: Option<Duration> = None;
    let mut connection_timeout_disabled: bool = false;
    let mut dns_servers: Vec<IpAddr> = Vec::new();
    let mut weight: u32 = 1;
    let mut priority: Option<u16> = None;
    let mut health_check_config = UpstreamHealthCheckConfig::default();
    let mut mtls_cert: Option<Vec<rustls::pki_types::CertificateDer<'static>>> = None;
    let mut mtls_key: Option<rustls::pki_types::PrivateKeyDer<'static>> = None;

    if let Some(block) = &entry.children {
        for (name, entries) in block.directives.iter() {
            match name.as_str() {
                "cert" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v| v.as_string_with_interpolations(ctx))
                    {
                        mtls_cert = Some(
                            rustls::pki_types::CertificateDer::pem_slice_iter(
                                &read_mtls_data(&val).map_err(|e| {
                                    let e: Box<dyn Error + Send + Sync> = format!(
                                        "Can't read mTLS certificate for reverse proxy: {e}"
                                    )
                                    .into();
                                    e
                                })?,
                            )
                            .collect::<Result<Vec<_>, _>>()
                            .map_err(|e| {
                                let e: Box<dyn Error + Send + Sync> =
                                    format!("Can't read mTLS certificate for reverse proxy: {e}")
                                        .into();
                                e
                            })?,
                        );
                    }
                }
                "key" => {
                    if let Some(val) = entries.first().and_then(|e| e.args.first()).and_then(
                        |v: &ServerConfigurationValue| v.as_string_with_interpolations(ctx),
                    ) {
                        mtls_key = Some(
                            rustls::pki_types::PrivateKeyDer::from_pem_slice(
                                &read_mtls_data(&val).map_err(|e| {
                                    let e: Box<dyn Error + Send + Sync> = format!(
                                        "Can't read mTLS private key for reverse proxy: {e}"
                                    )
                                    .into();
                                    e
                                })?,
                            )
                            .map_err(|e| {
                                let e: Box<dyn Error + Send + Sync> =
                                    format!("Can't read mTLS private key for reverse proxy: {e}")
                                        .into();
                                e
                            })?,
                        );
                    }
                }
                "limit" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v: &ServerConfigurationValue| v.as_number())
                    {
                        if val > 0 {
                            limit = Some(val as usize);
                        }
                    }
                }
                "idle_timeout" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v| v.as_duration())
                    {
                        idle_timeout = Some(val);
                    }
                }
                "connection_timeout" => {
                    if let Some(val) = entries.first().and_then(|e| e.args.first()) {
                        if val.as_boolean() == Some(false) {
                            connection_timeout_disabled = true;
                        } else if let Some(duration) = val.as_duration() {
                            connection_timeout = Some(duration);
                        }
                    }
                }
                "dns_servers" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v| v.as_str())
                    {
                        dns_servers = val
                            .split(',')
                            .filter_map(|s| s.trim().parse::<IpAddr>().ok())
                            .collect();
                    }
                }
                "weight" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v: &ServerConfigurationValue| v.as_number())
                    {
                        if val > 0 {
                            weight = val as u32;
                        }
                    }
                }
                "priority" => {
                    if let Some(val) = entries
                        .first()
                        .and_then(|e| e.args.first())
                        .and_then(|v: &ServerConfigurationValue| v.as_number())
                    {
                        priority = Some(val as u16);
                    }
                }
                "active_check" => {
                    if let Some(val) = entries.first().map(|e| e.get_flag()) {
                        health_check_config.enabled = val;
                        if val {
                            if let Some(children) =
                                entries.first().and_then(|e| e.children.as_ref())
                            {
                                parse_active_health_check(children, &mut health_check_config)?;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if idle_timeout.is_none() {
        idle_timeout = Some(Duration::from_millis(DEFAULT_KEEPALIVE_IDLE_TIMEOUT_MS));
    }

    if connection_timeout.is_none() && !connection_timeout_disabled {
        connection_timeout = Some(Duration::from_millis(DEFAULT_CONNECTION_TIMEOUT_MS));
    }

    let mtls = if let (Some(certs), Some(key)) = (mtls_cert, mtls_key) {
        Some(std::sync::Arc::new(MtlsCredentials { certs, key }))
    } else {
        None
    };
    cfg.upstreams.push(Upstream::Srv(SrvUpstreamData {
        srv_name: srv_name.to_string(),
        dns_servers,
        limit,
        weight,
        health_check_config,
        mtls,
        priority,
        connection_timeout,
        idle_timeout: idle_timeout
            .unwrap_or(Duration::from_millis(DEFAULT_KEEPALIVE_IDLE_TIMEOUT_MS)),
    }));

    Ok(())
}

pub(super) fn read_mtls_data(path: &str) -> Result<std::sync::Arc<Vec<u8>>, std::io::Error> {
    if let Some(cached) = MTLS_FILE_CACHE.get(path) {
        return Ok(cached.clone());
    }

    Ok(MTLS_FILE_CACHE
        .entry(path.to_string())
        .or_try_insert_with(|| std::fs::read(path).map(std::sync::Arc::new))?
        .downgrade()
        .clone())
}
