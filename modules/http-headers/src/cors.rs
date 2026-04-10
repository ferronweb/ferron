//! CORS preflight handling and response header injection.

use bytes::Bytes;
use http::header::{
    ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
    ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_EXPOSE_HEADERS, ACCESS_CONTROL_MAX_AGE, VARY,
};
use http::{HeaderMap, HeaderValue, Method, Response, StatusCode};
use http_body_util::Full;

use crate::config::CorsConfig;

/// Check if the request is a CORS preflight request.
pub fn is_preflight(method: &Method, headers: &HeaderMap) -> bool {
    *method == Method::OPTIONS
        && headers.contains_key("origin")
        && headers.contains_key("access-control-request-method")
}

/// Build a CORS preflight response (204 No Content).
pub fn build_preflight_response(
    cors: &CorsConfig,
    origin: &str,
    request_method: &str,
    request_headers: Option<&str>,
) -> Response<Full<Bytes>> {
    let mut response = Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Full::new(Bytes::new()))
        .unwrap();

    apply_cors_headers(
        response.headers_mut(),
        cors,
        origin,
        request_method,
        request_headers,
    );
    response
}

/// Apply CORS headers to an existing response.
pub fn apply_cors_headers(
    headers: &mut HeaderMap,
    cors: &CorsConfig,
    origin: &str,
    _request_method: &str,
    _request_headers: Option<&str>,
) {
    // Access-Control-Allow-Origin
    let allow_origin = if cors.origins.contains(&"*".to_string()) {
        "*"
    } else if cors.origins.iter().any(|o| o == origin) {
        origin
    } else {
        return; // Origin not allowed, don't add CORS headers
    };

    if let Ok(val) = HeaderValue::from_str(allow_origin) {
        headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, val);
    }

    // Vary: Origin
    if !cors.origins.is_empty() && !cors.origins.iter().any(|o| o == "*") {
        headers.insert(VARY, HeaderValue::from_static("origin"));
    }

    // Access-Control-Allow-Credentials
    if cors.credentials && allow_origin != "*" {
        headers.insert(
            ACCESS_CONTROL_ALLOW_CREDENTIALS,
            HeaderValue::from_static("true"),
        );
    }

    // Access-Control-Allow-Methods (only for preflight)
    if !cors.methods.is_empty() {
        let methods = cors.methods.join(", ");
        if let Ok(val) = HeaderValue::from_str(&methods) {
            headers.insert(ACCESS_CONTROL_ALLOW_METHODS, val);
        }
    }

    // Access-Control-Allow-Headers (only for preflight)
    if !cors.headers.is_empty() {
        let allowed = cors.headers.join(", ");
        if let Ok(val) = HeaderValue::from_str(&allowed) {
            headers.insert(ACCESS_CONTROL_ALLOW_HEADERS, val);
        }
    }

    // Access-Control-Max-Age (only for preflight)
    if let Some(max_age) = cors.max_age {
        if let Ok(val) = HeaderValue::from_str(&max_age.to_string()) {
            headers.insert(ACCESS_CONTROL_MAX_AGE, val);
        }
    }

    // Access-Control-Expose-Headers (for actual responses)
    if !cors.expose_headers.is_empty() {
        let exposed = cors.expose_headers.join(", ");
        if let Ok(val) = HeaderValue::from_str(&exposed) {
            headers.insert(ACCESS_CONTROL_EXPOSE_HEADERS, val);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CorsConfig;

    #[test]
    fn preflight_detects_options_with_origin_and_request_method() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", HeaderValue::from_static("https://example.com"));
        headers.insert(
            "access-control-request-method",
            HeaderValue::from_static("POST"),
        );
        assert!(is_preflight(&Method::OPTIONS, &headers));
    }

    #[test]
    fn preflight_returns_false_for_non_options() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", HeaderValue::from_static("https://example.com"));
        assert!(!is_preflight(&Method::GET, &headers));
    }

    #[test]
    fn cors_allows_wildcard_origin() {
        let cors = CorsConfig {
            origins: vec!["*".to_string()],
            ..Default::default()
        };
        let mut headers = HeaderMap::new();
        apply_cors_headers(&mut headers, &cors, "https://any.com", "GET", None);
        assert_eq!(headers.get(ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(), "*");
    }

    #[test]
    fn cors_allows_explicit_origin() {
        let cors = CorsConfig {
            origins: vec!["https://allowed.com".to_string()],
            ..Default::default()
        };
        let mut headers = HeaderMap::new();
        apply_cors_headers(&mut headers, &cors, "https://allowed.com", "GET", None);
        assert_eq!(
            headers.get(ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
            "https://allowed.com"
        );
    }

    #[test]
    fn cors_denies_unlisted_origin() {
        let cors = CorsConfig {
            origins: vec!["https://allowed.com".to_string()],
            ..Default::default()
        };
        let mut headers = HeaderMap::new();
        apply_cors_headers(&mut headers, &cors, "https://not-allowed.com", "GET", None);
        assert!(!headers.contains_key(ACCESS_CONTROL_ALLOW_ORIGIN));
    }

    #[test]
    fn cors_sets_methods_and_headers_for_preflight() {
        let cors = CorsConfig {
            origins: vec!["*".to_string()],
            methods: vec!["GET".to_string(), "POST".to_string()],
            headers: vec!["Content-Type".to_string(), "Authorization".to_string()],
            max_age: Some(3600),
            ..Default::default()
        };
        let mut headers = HeaderMap::new();
        apply_cors_headers(
            &mut headers,
            &cors,
            "https://any.com",
            "POST",
            Some("Content-Type"),
        );

        assert_eq!(
            headers.get(ACCESS_CONTROL_ALLOW_METHODS).unwrap(),
            "GET, POST"
        );
        assert_eq!(
            headers.get(ACCESS_CONTROL_ALLOW_HEADERS).unwrap(),
            "Content-Type, Authorization"
        );
        assert_eq!(headers.get(ACCESS_CONTROL_MAX_AGE).unwrap(), "3600");
    }
}
