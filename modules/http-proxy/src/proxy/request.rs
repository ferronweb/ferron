use std::borrow::Cow;
use std::fmt::Write;

use arrayvec::ArrayString;
use ferron_http::client_ip::ClientIpFromHeaderConfig;
use ferron_http::HttpContext;
use http::header::{HeaderName, HeaderValue};
use http::{Request, Uri};

use crate::config::ProxyConfig;
use crate::send_request::ProxyBody;
use crate::types::error::ProxyError;

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

    let request_path = req_ref.uri().path().to_owned();
    let request_query = req_ref.uri().query().map(|q| q.to_owned());

    // Use Cow to avoid allocation when upstream path is "/" (common case)
    let path: Cow<str> = if request_path.as_bytes().first() == Some(&b'/') {
        let mut proxy_request_path = proxy_request_url.path();
        while proxy_request_path.as_bytes().last().copied() == Some(b'/') {
            proxy_request_path = &proxy_request_path[..(proxy_request_path.len() - 1)];
        }
        if proxy_request_path == "/" {
            Cow::Owned(request_path)
        } else {
            let mut s = String::with_capacity(proxy_request_path.len() + request_path.len());
            s.push_str(proxy_request_path);
            s.push_str(&request_path);
            Cow::Owned(s)
        }
    } else {
        Cow::Owned(request_path)
    };

    let final_uri: Cow<str> = if let Some(query) = &request_query {
        let mut u = String::with_capacity(path.len() + 1 + query.len());
        u.push_str(&path);
        u.push('?');
        u.push_str(query);
        Cow::Owned(u)
    } else {
        path
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
        let (name, v) = action;
        let hv = HeaderValue::from_str(v)?;
        add_values.push((name.clone(), hv));
    }

    let req = ctx.req.take().ok_or(ProxyError::RequestConstructError(
        "no request in context".to_string(),
    ))?;
    let (mut parts, body) = req.into_parts();

    let final_uri_buf = bytes::Bytes::from_owner(final_uri.into_owned());
    parts.uri = Uri::from_maybe_shared(final_uri_buf)?;

    for name in &config.headers_to_remove {
        parts.headers.remove(name);
    }

    for (name, hv) in replace_values {
        parts.headers.insert(name, hv);
    }

    for (name, hv) in add_values {
        parts.headers.append(name, hv);
    }

    // Strip hop-by-hop headers per RFC 7230 section 6.1
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

    let client_ip = ctx.remote_address.map(|a| a.ip());
    let local_ip = ctx.local_address.map(|a| a.ip());
    let proto = if ctx.encrypted { "https" } else { "http" };

    let client_ip_from_header_enabled = ClientIpFromHeaderConfig::resolve_from_context(ctx)
        .is_some_and(|s| {
            ctx.remote_address
                .is_some_and(|a| s.is_trusted_proxy(a.ip()))
        });

    // Only inject client/forwarded headers when both local and remote addresses
    // are known (not the case for Unix socket listeners).
    if let (Some(client_ip), Some(local_ip)) = (client_ip, local_ip) {
        // Format IP addresses into stack-allocated buffers to avoid heap allocation
        let mut client_ip_buf = ArrayString::<45>::new();
        let _ = write!(client_ip_buf, "{}", client_ip);
        let mut local_ip_buf = ArrayString::<45>::new();
        let _ = write!(local_ip_buf, "{}", local_ip);

        if client_ip_from_header_enabled {
            append_x_forwarded_for(&mut parts.headers, &client_ip_buf);
            append_forwarded(&mut parts.headers, &client_ip_buf, proto, &local_ip_buf);
        } else {
            set_x_forwarded_for(&mut parts.headers, &client_ip_buf);
            set_forwarded(&mut parts.headers, &client_ip_buf, proto, &local_ip_buf);
        }

        parts.headers.insert(
            HeaderName::from_static("x-real-ip"),
            HeaderValue::from_str(&client_ip_buf)?,
        );
    }

    parts.headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static(proto),
    );

    parts.version = http::Version::default();

    if let Some(tc) = ctx.get::<ferron_http::trace_context::TraceContextKey>() {
        ferron_http::trace_context::inject_trace_headers(&mut parts.headers, tc);
    }

    Ok(Request::from_parts(parts, ProxyBody::new(body)))
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
            let mut buf = String::with_capacity(existing_str.len() + client_ip_str.len() + 2);
            buf.push_str(existing_str);
            buf.push(',');
            buf.push(' ');
            buf.push_str(client_ip_str);
            // Convert to Bytes to avoid additional heap allocations in HeaderValue::from_str
            let buf = bytes::Bytes::from_owner(buf);
            if let Ok(hv) = HeaderValue::from_maybe_shared(buf) {
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
    let buf = build_forwarded_element(client_ip_str, proto, local_ip_str);
    // Convert to Bytes to avoid additional heap allocations in HeaderValue::from_str
    let buf = bytes::Bytes::from_owner(buf);
    if let Ok(hv) = HeaderValue::from_maybe_shared(buf) {
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
    let element_buf = build_forwarded_element(client_ip_str, proto, local_ip_str);
    if let Some(existing) = headers.get("forwarded") {
        if let Ok(existing_str) = existing.to_str() {
            let mut buf = String::with_capacity(existing_str.len() + element_buf.len() + 2);
            buf.push_str(existing_str);
            buf.push(',');
            buf.push(' ');
            buf.push_str(&element_buf);
            // Convert to Bytes to avoid additional heap allocations in HeaderValue::from_str
            let buf = bytes::Bytes::from_owner(buf);
            if let Ok(hv) = HeaderValue::from_maybe_shared(buf) {
                headers.insert("forwarded", hv);
                return;
            }
        }
    }
    // Convert to Bytes to avoid additional heap allocations in HeaderValue::from_str
    let buf = bytes::Bytes::from_owner(element_buf);
    if let Ok(hv) = HeaderValue::from_maybe_shared(buf) {
        headers.insert("forwarded", hv);
    }
}

/// Build a Forwarded header element into a `String`.
#[inline]
fn build_forwarded_element(client_ip_str: &str, proto: &str, local_ip_str: &str) -> String {
    // for="[<client_ip_str>];proto=<proto>;by="[<local_ip_str>]"
    // 012345               67890123       456789              01
    //
    // 21 additional characters in the worst case, so reserve 21 for safety
    let mut buf =
        String::with_capacity(client_ip_str.len() + proto.len() + local_ip_str.len() + 21);
    buf.push_str("for=");
    if client_ip_str.contains(':') {
        buf.push('\"');
        buf.push('[');
        buf.push_str(client_ip_str);
        buf.push(']');
        buf.push('\"');
    } else {
        buf.push_str(client_ip_str);
    }
    buf.push_str(";proto=");
    buf.push_str(proto);
    buf.push_str(";by=");
    if local_ip_str.contains(':') {
        buf.push('\"');
        buf.push('[');
        buf.push_str(local_ip_str);
        buf.push(']');
        buf.push('\"');
    } else {
        buf.push_str(local_ip_str);
    }
    buf
}
