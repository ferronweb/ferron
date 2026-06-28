use std::str::FromStr;

use ferron_http::client_ip::ClientIpFromHeaderConfig;
use ferron_http::HttpContext;
use http::header::{HeaderName, HeaderValue};
use http::{Request, Uri};

use crate::config::{HeaderAction, ProxyConfig};
use crate::send_request::ProxyBody;
use crate::types::error::ProxyError;

/// Check whether `client_ip_from_header` is configured.
#[inline]
fn client_ip_from_header_enabled(ctx: &HttpContext) -> bool {
    ClientIpFromHeaderConfig::resolve_from_context(ctx)
        .is_some_and(|s| s.is_trusted_proxy(ctx.remote_address.ip()))
}

/// Construct proxy request with header transformations.
#[inline]
pub(super) fn construct_proxy_request(
    ctx: &mut HttpContext,
    config: &ProxyConfig,
    proxy_request_url: &Uri,
) -> Result<Request<ProxyBody>, ProxyError> {
    let req_ref = ctx.req.as_ref().ok_or(ProxyError::RequestConstructError(
        "no request in context".to_string(),
    ))?;

    let request_path = req_ref.uri().path();
    let path = if request_path.as_bytes().first() == Some(&b'/') {
        let mut proxy_request_path = proxy_request_url.path();
        while proxy_request_path.as_bytes().last().copied() == Some(b'/') {
            proxy_request_path = &proxy_request_path[..(proxy_request_path.len() - 1)];
        }
        let mut s = String::with_capacity(proxy_request_path.len() + request_path.len());
        s.push_str(proxy_request_path);
        s.push_str(request_path);
        s
    } else {
        request_path.to_string()
    };

    let final_uri = if let Some(query) = req_ref.uri().query() {
        let mut u = String::with_capacity(path.len() + 1 + query.len());
        u.push_str(&path);
        u.push('?');
        u.push_str(query);
        u
    } else {
        path.clone()
    };

    let mut replace_values: Vec<(HeaderName, HeaderValue)> =
        Vec::with_capacity(config.headers_to_replace.len());
    for (name, value) in &config.headers_to_replace {
        let hv = HeaderValue::from_str(value)?;
        replace_values.push((name.clone(), hv));
    }

    let mut add_values: Vec<(HeaderName, HeaderValue)> =
        Vec::with_capacity(config.headers_to_add.len());
    for action in &config.headers_to_add {
        let HeaderAction::Append(name, v) = action;
        let hv = HeaderValue::from_str(v)?;
        add_values.push((name.clone(), hv));
    }

    let req = ctx.req.take().ok_or(ProxyError::RequestConstructError(
        "no request in context".to_string(),
    ))?;
    let (mut parts, body) = req.into_parts();

    parts.uri = Uri::from_str(&final_uri)?;

    for name in &config.headers_to_remove {
        parts.headers.remove(name);
    }

    for (name, hv) in replace_values {
        parts.headers.insert(name, hv);
    }

    for (name, hv) in add_values {
        parts.headers.append(name, hv);
    }

    // Strip hop-by-hop headers per RFC 7230 §6.1
    // These must not be forwarded by a proxy.
    if let Some(c) = parts.headers.remove(http::header::CONNECTION) {
        // If the connection header contains "upgrade",
        // preserve it to avoid breaking the upgrade connection.
        if str::from_utf8(c.as_bytes()).is_ok_and(|s| s.to_lowercase().contains("upgrade")) {
            parts.headers.insert(
                http::header::CONNECTION,
                HeaderValue::from_static("upgrade"),
            );
        }
    }
    parts.headers.remove(HeaderName::from_static("keep-alive"));
    parts.headers.remove(http::header::TRANSFER_ENCODING);
    parts.headers.remove(http::header::TE);
    parts.headers.remove(http::header::TRAILER);
    parts.headers.remove("proxy-authorization");
    parts.headers.remove("proxy-authenticate");

    let client_ip = ctx.remote_address.ip();
    let local_ip = ctx.local_address.ip();
    let proto = if ctx.encrypted { "https" } else { "http" };
    let client_ip_str = client_ip.to_string();
    let local_ip_str = local_ip.to_string();

    if client_ip_from_header_enabled(ctx) {
        append_x_forwarded_for(&mut parts.headers, &client_ip_str);
        append_forwarded(&mut parts.headers, &client_ip_str, proto, &local_ip_str);
    } else {
        set_x_forwarded_for(&mut parts.headers, &client_ip_str);
        set_forwarded(&mut parts.headers, &client_ip_str, proto, &local_ip_str);
    }

    parts.headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static(proto),
    );
    parts.headers.insert(
        HeaderName::from_static("x-real-ip"),
        HeaderValue::from_str(&client_ip_str)?,
    );

    parts.version = http::Version::default();

    if let Some(tc) = ctx.get::<ferron_http::trace_context::TraceContextKey>() {
        ferron_http::trace_context::inject_trace_headers(&mut parts.headers, tc);
    }

    Ok(Request::from_parts(parts, body))
}

#[inline]
pub(super) fn set_x_forwarded_for(headers: &mut http::HeaderMap, client_ip_str: &str) {
    if let Ok(hv) = HeaderValue::from_str(client_ip_str) {
        headers.insert("x-forwarded-for", hv);
    }
}

#[inline]
pub(super) fn append_x_forwarded_for(headers: &mut http::HeaderMap, client_ip_str: &str) {
    if let Some(existing) = headers.get("x-forwarded-for") {
        if let Ok(existing_str) = existing.to_str() {
            let new_value = format!("{}, {}", existing_str, client_ip_str);
            if let Ok(hv) = HeaderValue::from_str(&new_value) {
                headers.insert("x-forwarded-for", hv);
                return;
            }
        }
    }
    if let Ok(hv) = HeaderValue::from_str(client_ip_str) {
        headers.insert("x-forwarded-for", hv);
    }
}

#[inline]
pub(super) fn set_forwarded(
    headers: &mut http::HeaderMap,
    client_ip_str: &str,
    proto: &'static str,
    local_ip_str: &str,
) {
    let element = build_forwarded_element(client_ip_str, proto, local_ip_str);
    if let Ok(hv) = HeaderValue::from_str(&element) {
        headers.insert("forwarded", hv);
    }
}

#[inline]
pub(super) fn append_forwarded(
    headers: &mut http::HeaderMap,
    client_ip_str: &str,
    proto: &'static str,
    local_ip_str: &str,
) {
    let element = build_forwarded_element(client_ip_str, proto, local_ip_str);
    if let Some(existing) = headers.get("forwarded") {
        if let Ok(existing_str) = existing.to_str() {
            let new_value = format!("{}, {}", existing_str, element);
            if let Ok(hv) = HeaderValue::from_str(&new_value) {
                headers.insert("forwarded", hv);
                return;
            }
        }
    }
    if let Ok(hv) = HeaderValue::from_str(&element) {
        headers.insert("forwarded", hv);
    }
}

pub(super) fn build_forwarded_element(
    client_ip_str: &str,
    proto: &str,
    local_ip_str: &str,
) -> String {
    let for_value = if client_ip_str.contains(':') {
        format!("\"[{}]\"", client_ip_str)
    } else {
        client_ip_str.to_string()
    };
    let by_value = if local_ip_str.contains(':') {
        format!("\"[{}]\"", local_ip_str)
    } else {
        local_ip_str.to_string()
    };
    format!("for={};proto={};by={}", for_value, proto, by_value)
}
