use http::header::HeaderValue;
use http::HeaderMap;
use rand::Rng;

/// Minimal W3C trace context representation used by http-server and other modules.
#[derive(Debug, Clone)]
pub struct TraceContext {
    pub trace_id: String, // 32 hex chars
    pub span_id: String,  // 16 hex chars
    pub sampled: bool,
    pub tracestate: Option<String>,
}

fn is_hex(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Parse a `traceparent` header value into TraceContext (without tracestate).
pub fn parse_traceparent(s: &str) -> Option<TraceContext> {
    // Expected format: version-traceid-parentid-flags
    let parts: Vec<&str> = s.trim().split('-').collect();
    if parts.len() != 4 {
        return None;
    }
    let (_version, trace_id, parent_id, flags) = (parts[0], parts[1], parts[2], parts[3]);
    if trace_id.len() != 32 || parent_id.len() != 16 || flags.len() != 2 {
        return None;
    }
    if !is_hex(trace_id) || !is_hex(parent_id) || !is_hex(flags) {
        return None;
    }
    let flags_val = u8::from_str_radix(flags, 16).ok()?;
    let sampled = (flags_val & 0x01) == 0x01;
    Some(TraceContext {
        trace_id: trace_id.to_lowercase(),
        span_id: parent_id.to_lowercase(),
        sampled,
        tracestate: None,
    })
}

fn bytes_to_hex(buf: &[u8]) -> String {
    let mut s = String::with_capacity(buf.len() * 2);
    for b in buf {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// Generate a new traceparent TraceContext (version 00) with random ids.
pub fn generate_traceparent(sampled: bool) -> TraceContext {
    let mut rng = rand::rng();
    let mut trace_bytes = [0u8; 16];
    let mut span_bytes = [0u8; 8];
    rng.fill_bytes(&mut trace_bytes);
    rng.fill_bytes(&mut span_bytes);
    TraceContext {
        trace_id: bytes_to_hex(&trace_bytes),
        span_id: bytes_to_hex(&span_bytes),
        sampled,
        tracestate: None,
    }
}

/// Format a TraceContext into a traceparent header value.
pub fn format_traceparent(tc: &TraceContext) -> String {
    let flags = if tc.sampled { 1u8 } else { 0u8 };
    format!("00-{}-{}-{:02x}", tc.trace_id, tc.span_id, flags)
}

/// Inject trace headers into an http HeaderMap
pub fn inject_trace_headers(headers: &mut HeaderMap, tc: &TraceContext) {
    let tp = format_traceparent(tc);
    if let Ok(v) = HeaderValue::from_str(&tp) {
        headers.insert("traceparent", v);
    }
    if let Some(ts) = &tc.tracestate {
        if let Ok(v) = HeaderValue::from_str(ts) {
            headers.insert("tracestate", v);
        }
    }
}

use typemap_rev::TypeMapKey;

/// TypeMap key for storing TraceContext in HttpContext extensions.
pub struct TraceContextKey;
impl TypeMapKey for TraceContextKey {
    type Value = TraceContext;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_traceparent() {
        let tp = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        let tc = parse_traceparent(tp).unwrap();
        assert_eq!(tc.trace_id, "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(tc.span_id, "00f067aa0ba902b7");
        assert!(tc.sampled);
    }

    #[test]
    fn test_parse_invalid_traceparent() {
        assert!(parse_traceparent("invalid").is_none());
        assert!(parse_traceparent("00-short-00f067aa0ba902b7-01").is_none());
        assert!(parse_traceparent("00-4bf92f3577b34da6a3ce929d0e0e4736-short-01").is_none());
        assert!(
            parse_traceparent("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-xx").is_none()
        );
    }

    #[test]
    fn test_format_traceparent() {
        let tc = TraceContext {
            trace_id: "4bf92f3577b34da6a3ce929d0e0e4736".to_string(),
            span_id: "00f067aa0ba902b7".to_string(),
            sampled: true,
            tracestate: None,
        };
        let formatted = format_traceparent(&tc);
        assert_eq!(
            formatted,
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
        );
    }

    #[test]
    fn test_generate_traceparent() {
        let tc = generate_traceparent(true);
        assert_eq!(tc.trace_id.len(), 32);
        assert_eq!(tc.span_id.len(), 16);
        assert!(tc.sampled);
        assert!(is_hex(&tc.trace_id));
        assert!(is_hex(&tc.span_id));
    }

    #[test]
    fn test_inject_trace_headers() {
        let mut headers = HeaderMap::new();
        let tc = TraceContext {
            trace_id: "4bf92f3577b34da6a3ce929d0e0e4736".to_string(),
            span_id: "00f067aa0ba902b7".to_string(),
            sampled: false,
            tracestate: Some("conntype=1".to_string()),
        };
        inject_trace_headers(&mut headers, &tc);
        assert_eq!(
            headers.get("traceparent").unwrap().to_str().unwrap(),
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00"
        );
        assert_eq!(
            headers.get("tracestate").unwrap().to_str().unwrap(),
            "conntype=1"
        );
    }
}
