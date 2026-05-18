use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, LazyLock};

use ferron_http::client_ip::ClientIpFromHeaderConfig;
use ferron_http::HttpContext;
use http::header::{HeaderName, HeaderValue};
use http::{Request, Uri};

use crate::config::{HeaderAction, ProxyConfig};
use crate::send_request::ProxyBody;

/// Check whether `client_ip_from_header` is configured.
#[inline]
fn client_ip_from_header_enabled(ctx: &HttpContext) -> bool {
    ClientIpFromHeaderConfig::resolve_from_context(ctx)
        .is_some_and(|s| s.is_trusted_proxy(ctx.remote_address.ip()))
}

/// Interpolate header value with HTTP request variables.
///
/// Scans for `{{...}}` syntax and resolves variables using the context's
/// `Variables` implementation. Plain strings without `{{` are returned as-is.
///
/// For performance, templates are compiled into segments and cached globally so
/// repeated requests don't re-parse the same template string.
#[derive(Clone, Debug)]
enum Segment {
    Literal(String),
    Var(String),
}

static TEMPLATE_CACHE: LazyLock<parking_lot::RwLock<HashMap<String, Arc<Vec<Segment>>>>> =
    LazyLock::new(|| parking_lot::RwLock::new(HashMap::new()));

fn compile_template(value: &str) -> Vec<Segment> {
    let mut segs: Vec<Segment> = Vec::new();
    let mut chars = value.chars().peekable();
    let mut literal = String::new();

    while let Some(ch) = chars.next() {
        if ch == '{' && chars.peek() == Some(&'{') {
            chars.next();
            if !literal.is_empty() {
                segs.push(Segment::Literal(std::mem::take(&mut literal)));
            }
            let mut var_name = String::new();
            loop {
                match chars.next() {
                    Some('}') if chars.peek() == Some(&'}') => {
                        chars.next();
                        break;
                    }
                    Some(c) => var_name.push(c),
                    None => return vec![Segment::Literal(value.to_string())],
                }
            }
            segs.push(Segment::Var(var_name));
        } else {
            literal.push(ch);
        }
    }

    if !literal.is_empty() {
        segs.push(Segment::Literal(literal));
    }
    segs
}

fn interpolate_header_value(value: &str, ctx: &HttpContext) -> String {
    if !value.contains("{{") {
        return value.to_string();
    }

    let segs_arc = {
        let guard = TEMPLATE_CACHE.read();
        if let Some(found) = guard.get(value) {
            Arc::clone(found)
        } else {
            drop(guard);
            let compiled = Arc::new(compile_template(value));
            let mut guard = TEMPLATE_CACHE.write();
            let entry = guard
                .entry(value.to_string())
                .or_insert_with(|| Arc::clone(&compiled));
            Arc::clone(entry)
        }
    };

    let mut result = String::with_capacity(value.len());
    for seg in segs_arc.iter() {
        match seg {
            Segment::Literal(s) => result.push_str(s),
            Segment::Var(var_name) => {
                if let Some(env_var) = var_name.strip_prefix("env.") {
                    if let Ok(env_value) = std::env::var(env_var) {
                        result.push_str(&env_value);
                    } else {
                        result.push_str(&format!("{{{{{}}}}}", var_name));
                    }
                } else if let Some(resolved) =
                    <dyn ferron_core::config::Variables>::resolve(ctx, var_name)
                {
                    result.push_str(&resolved);
                } else {
                    result.push_str(&format!("{{{{{}}}}}", var_name));
                }
            }
        }
    }
    result
}

/// Construct proxy request with header transformations.
pub(super) fn construct_proxy_request(
    ctx: &mut HttpContext,
    config: &ProxyConfig,
    proxy_request_url: &Uri,
) -> Result<Request<ProxyBody>, Box<dyn std::error::Error + Send + Sync>> {
    let req_ref = ctx.req.as_ref().ok_or("no request in context")?;

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
        let resolved = interpolate_header_value(value, ctx);
        let hv = HeaderValue::from_str(&resolved).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Invalid header value: {e}"),
            )
        })?;
        replace_values.push((name.clone(), hv));
    }

    let mut add_values: Vec<(HeaderName, HeaderValue)> =
        Vec::with_capacity(config.headers_to_add.len());
    for action in &config.headers_to_add {
        let HeaderAction::Append(name, v) = action;
        let resolved = interpolate_header_value(v, ctx);
        let hv = HeaderValue::from_str(&resolved).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Invalid header value: {e}"),
            )
        })?;
        add_values.push((name.clone(), hv));
    }

    let req = ctx.req.take().ok_or("no request in context")?;
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

pub(super) fn set_x_forwarded_for(headers: &mut http::HeaderMap, client_ip_str: &str) {
    if let Ok(hv) = HeaderValue::from_str(client_ip_str) {
        headers.insert("x-forwarded-for", hv);
    }
}

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
