use std::collections::HashMap;

pub fn resolve_variable(
    name: &str,
    request: &http::request::Parts,
    variables: &HashMap<String, String>,
) -> Option<String> {
    match name {
        "request.method" => Some(request.method.to_string()),
        "request.uri.path" => Some(request.uri.path().to_string()),
        "request.uri.query" => Some(request.uri.query().unwrap_or("").to_string()),
        "request.uri" => Some(request.uri.to_string()),
        "request.version" => Some(format!("{:?}", request.version)),
        n if n.starts_with("env.") => {
            let env_name = n.trim_start_matches("env.");
            std::env::var(env_name).ok()
        }
        n if n.starts_with("request.header.") => {
            let header_name = n
                .trim_start_matches("request.header.")
                .to_ascii_lowercase()
                .replace("_", "-");
            request
                .headers
                .get(&header_name)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        }
        n => variables.get(n).cloned().or_else(|| Some(name.to_string())),
    }
}
