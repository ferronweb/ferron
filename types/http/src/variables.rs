use std::collections::HashMap;

use crate::HttpRequest;

pub fn resolve_variable(
    name: &str,
    request: &HttpRequest,
    variables: &HashMap<String, String>,
) -> Option<String> {
    match name {
        "request.method" => Some(request.method().to_string()),
        "request.uri.path" => Some(request.uri().path().to_string()),
        "request.uri.query" => Some(request.uri().query().unwrap_or("").to_string()),
        "request.uri" => Some(request.uri().to_string()),
        "request.version" => Some(match request.version() {
            http::Version::HTTP_09 => "HTTP/0.9".to_string(),
            http::Version::HTTP_10 => "HTTP/1.0".to_string(),
            http::Version::HTTP_11 => "HTTP/1.1".to_string(),
            http::Version::HTTP_2 => "HTTP/2.0".to_string(),
            http::Version::HTTP_3 => "HTTP/3.0".to_string(),
            _ => "unknown".to_string(),
        }),
        n if n.starts_with("request.header.") => {
            let header_name = n
                .trim_start_matches("request.header.")
                .to_ascii_lowercase()
                .replace("_", "-");
            request
                .headers()
                .get(&header_name)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        }
        n => variables.get(n).cloned().or_else(|| Some(name.to_string())),
    }
}
