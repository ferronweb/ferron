//! Session affinity (sticky session) implementation for the proxy.

use crate::types::affinity::AffinityType;

/// Extract the affinity key from the request.
pub fn extract_affinity_key(
    affinity: &Option<crate::config::AffinityConfig>,
    ctx: &ferron_http::HttpContext,
) -> Option<Vec<u8>> {
    let affinity = affinity.as_ref()?;

    let key = match &affinity.affinity_type {
        AffinityType::Cookie(cfg) => {
            // Read cookie from request headers
            let req = ctx.req.as_ref()?;
            let cookie_header = req.headers().get(http::header::COOKIE)?;
            let cookie_str = cookie_header.to_str().ok()?;
            parse_cookie_value(cookie_str, &cfg.name)?
                .as_bytes()
                .to_vec()
        }
        AffinityType::Header(header_name) => {
            let req = ctx.req.as_ref()?;
            let header_value = req.headers().get(header_name)?;
            header_value.as_bytes().to_vec()
        }
        AffinityType::Ip => ctx.remote_address.ip().to_string().into_bytes(),
        AffinityType::Hash { variable, .. } => {
            // For hash affinity, use the variable value as the key
            // Variables are resolved from the request context
            resolve_variable(variable, ctx)?.into_bytes()
        }
    };

    if key.is_empty() {
        return None;
    }

    Some(key)
}

/// Parse a specific cookie value from a Cookie header string.
fn parse_cookie_value(cookie_header: &str, name: &str) -> Option<String> {
    for pair in cookie_header.split(';') {
        let pair = pair.trim();
        if let Some(eq_pos) = pair.find('=') {
            let cookie_name = &pair[..eq_pos];
            if cookie_name == name {
                return Some(pair[eq_pos + 1..].to_string());
            }
        }
    }
    None
}

/// Resolve a variable name to its value from the request context.
fn resolve_variable(variable: &str, ctx: &ferron_http::HttpContext) -> Option<String> {
    // Support common built-in variables
    let req = ctx.req.as_ref()?;
    match variable {
        "request.uri" => Some(req.uri().to_string()),
        "request.uri.path" => Some(req.uri().path().to_string()),
        "request.host" => req.uri().host().map(|s| s.to_string()),
        "request.method" => Some(req.method().as_str().to_string()),
        "remote.ip" => Some(ctx.remote_address.ip().to_string()),
        _ => {
            // Try to parse as a header variable: request.header.<name>
            if let Some(header_name) = variable.strip_prefix("request.header.") {
                let header_name = header_name.replace('_', "-");
                req.headers()
                    .get(header_name.as_str())
                    .map(|v| v.to_str().unwrap_or("").to_string())
            } else {
                None
            }
        }
    }
}

/// Set the affinity cookie on the response if using cookie affinity
/// and no valid cookie was present in the request.
pub fn maybe_set_affinity_cookie(
    resp: ferron_http::HttpResponse,
    affinity: &Option<crate::config::AffinityConfig>,
    backend_id: Option<String>,
) -> ferron_http::HttpResponse {
    let affinity = match affinity {
        Some(a) => a,
        None => return resp,
    };

    let cookie_cfg = match &affinity.affinity_type {
        AffinityType::Cookie(cfg) => cfg,
        _ => return resp,
    };

    // Only set cookie if we have a valid affinity key
    let Some(backend_id) = backend_id else {
        return resp;
    };

    // Build Set-Cookie header value
    let mut cookie_value = format!(
        "{}={}; Path={}",
        cookie_cfg.name, backend_id, cookie_cfg.path
    );

    if let Some(ttl) = cookie_cfg.ttl {
        // Format as Max-Age in seconds
        cookie_value.push_str(&format!("; Max-Age={}", ttl.as_secs()));
    }

    if let Some(ref domain) = cookie_cfg.domain {
        cookie_value.push_str(&format!("; Domain={domain}"));
    }

    if cookie_cfg.secure {
        cookie_value.push_str("; Secure");
    }

    if cookie_cfg.httponly {
        cookie_value.push_str("; HttpOnly");
    }

    cookie_value.push_str(&format!("; SameSite={}", cookie_cfg.samesite.as_str()));

    // Set the cookie on the response
    match resp {
        ferron_http::HttpResponse::Custom(mut resp) => {
            if let Ok(header_value) = http::HeaderValue::from_str(&cookie_value) {
                resp.headers_mut()
                    .insert(http::header::SET_COOKIE, header_value);
            }
            ferron_http::HttpResponse::Custom(resp)
        }
        other => other,
    }
}
