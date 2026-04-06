//! Configuration parsing and validation for the HTTP headers module.

use std::collections::HashSet;
use std::error::Error;
use std::str::FromStr;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    Variables,
};
use http::header::HeaderName;

/// A response header action.
#[derive(Clone)]
pub enum HeaderAction {
    /// Append the given value to the header (allows duplicates).
    Append(HeaderName, String),
    /// Replace the header value (removes existing, sets new value).
    Replace(HeaderName, String),
    /// Remove all instances of the header.
    Remove(HeaderName),
}

/// CORS configuration.
#[derive(Clone, Default)]
pub struct CorsConfig {
    /// Allowed origins (empty means disabled, `["*"]` means any origin).
    pub origins: Vec<String>,
    /// Allowed HTTP methods.
    pub methods: Vec<String>,
    /// Allowed request headers.
    pub headers: Vec<String>,
    /// Whether credentials (cookies, auth) are allowed.
    pub credentials: bool,
    /// Preflight cache duration in seconds.
    pub max_age: Option<u32>,
    /// Headers exposed to the browser.
    pub expose_headers: Vec<String>,
}

/// Parsed HTTP headers configuration.
#[derive(Clone, Default)]
pub struct HeadersConfig {
    pub header_actions: Vec<HeaderAction>,
    pub cors: Option<CorsConfig>,
}

/// Resolve a config value as a string, interpolating `{{env.*}}` variables only.
fn resolve_config_value_with_env(value: &ServerConfigurationValue) -> Option<String> {
    struct EnvResolver;
    impl Variables for EnvResolver {
        fn resolve(&self, _name: &str) -> Option<String> {
            None
        }
    }
    value.as_string_with_interpolations(&EnvResolver)
}

/// Parse header actions from a directive entry.
fn parse_header_entry(
    entry: &ServerConfigurationDirectiveEntry,
    cfg: &mut HeadersConfig,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    if entry.args.is_empty() {
        return Err("header requires at least one argument".into());
    }

    let first_arg = entry.args[0]
        .as_str()
        .ok_or("header name must be a string")?;

    match first_arg.chars().next() {
        Some('+') => {
            let name = &first_arg[1..];
            let value = entry
                .args
                .get(1)
                .and_then(resolve_config_value_with_env)
                .ok_or("header +Name requires a value")?;
            let header_name = HeaderName::from_str(name)
                .map_err(|e| format!("Invalid header name '{name}': {e}"))?;
            cfg.header_actions
                .push(HeaderAction::Append(header_name, value));
        }
        Some('-') => {
            let name = &first_arg[1..];
            let header_name = HeaderName::from_str(name)
                .map_err(|e| format!("Invalid header name '{name}': {e}"))?;
            cfg.header_actions.push(HeaderAction::Remove(header_name));
        }
        _ => {
            let name = first_arg;
            let value = entry
                .args
                .get(1)
                .and_then(resolve_config_value_with_env)
                .ok_or("header Name requires a value")?;
            let header_name = HeaderName::from_str(name)
                .map_err(|e| format!("Invalid header name '{name}': {e}"))?;
            cfg.header_actions
                .push(HeaderAction::Replace(header_name, value));
        }
    }

    Ok(())
}

/// Parse a single CORS directive block.
fn parse_cors_block(
    block: &ServerConfigurationBlock,
    cors: &mut CorsConfig,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    for (name, entries) in block.directives.iter() {
        match name.as_str() {
            "origins" => {
                for entry in entries {
                    for arg in &entry.args {
                        if let Some(val) = resolve_config_value_with_env(arg) {
                            cors.origins.push(val);
                        }
                    }
                }
            }
            "methods" => {
                for entry in entries {
                    for arg in &entry.args {
                        if let Some(val) = arg.as_str() {
                            cors.methods.push(val.to_string());
                        }
                    }
                }
            }
            "headers" => {
                for entry in entries {
                    for arg in &entry.args {
                        if let Some(val) = arg.as_str() {
                            cors.headers.push(val.to_string());
                        }
                    }
                }
            }
            "credentials" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_boolean())
                {
                    cors.credentials = val;
                }
            }
            "max_age" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_number())
                {
                    if val >= 0 {
                        cors.max_age = Some(val as u32);
                    }
                }
            }
            "expose_headers" => {
                for entry in entries {
                    for arg in &entry.args {
                        if let Some(val) = arg.as_str() {
                            cors.expose_headers.push(val.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Parse headers configuration from an HttpContext.
pub fn parse_headers_config(
    ctx: &ferron_http::HttpContext,
) -> Result<Option<HeadersConfig>, Box<dyn Error + Send + Sync>> {
    let header_entries = ctx.configuration.get_entries("header", true);
    let cors_entries = ctx.configuration.get_entries("cors", true);

    if header_entries.is_empty() && cors_entries.is_empty() {
        return Ok(None);
    }

    let mut cfg = HeadersConfig::default();

    for entry in &header_entries {
        parse_header_entry(entry, &mut cfg)?;
    }

    for entry in &cors_entries {
        let mut cors = CorsConfig::default();
        if let Some(block) = &entry.children {
            parse_cors_block(block, &mut cors)?;
        }
        cfg.cors = Some(cors);
    }

    Ok(Some(cfg))
}

/// Configuration validator for the HTTP headers module.
pub struct HttpHeadersConfigurationValidator;

impl ConfigurationValidator for HttpHeadersConfigurationValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        used_directives: &mut HashSet<String>,
        _is_global: bool,
    ) -> Result<(), Box<dyn Error>> {
        // Validate header directives
        if let Some(entries) = config.directives.get("header") {
            used_directives.insert("header".to_string());
            for e in entries {
                if e.args.is_empty() {
                    return Err("header requires at least one argument".into());
                }
                let first = e.args[0]
                    .as_str()
                    .ok_or("The header name must be a string")?;
                let (name, needs_value) = match first.chars().next() {
                    Some('+') => (&first[1..], true),
                    Some('-') => (&first[1..], false),
                    _ => (first, true),
                };
                HeaderName::from_str(name)
                    .map_err(|e| format!("Invalid header name '{name}': {e}"))?;
                if needs_value && e.args.get(1).and_then(|v| v.as_str()).is_none() {
                    return Err("header requires a value for add/replace operations".into());
                }
            }
        }

        // Validate cors directive
        if let Some(entries) = config.directives.get("cors") {
            used_directives.insert("cors".to_string());
            for e in entries {
                if let Some(block) = &e.children {
                    validate_cors_block(block, used_directives)?;
                }
            }
        }

        Ok(())
    }
}

fn validate_cors_block(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get("origins") {
        used.insert("origins".to_string());
        for e in entries {
            for arg in &e.args {
                if arg.as_str().is_none() {
                    return Err("Invalid `origins` — expected a string".into());
                }
            }
        }
    }
    if let Some(entries) = block.directives.get("methods") {
        used.insert("methods".to_string());
        for e in entries {
            for arg in &e.args {
                if arg.as_str().is_none() {
                    return Err("Invalid `methods` — expected a string".into());
                }
            }
        }
    }
    if let Some(entries) = block.directives.get("headers") {
        used.insert("headers".to_string());
        for e in entries {
            for arg in &e.args {
                if arg.as_str().is_none() {
                    return Err("Invalid `headers` — expected a string".into());
                }
            }
        }
    }
    if let Some(entries) = block.directives.get("credentials") {
        used.insert("credentials".to_string());
        for e in entries {
            if e.args.first().and_then(|v| v.as_boolean()).is_none() {
                return Err("Invalid `credentials` — expected a boolean".into());
            }
        }
    }
    if let Some(entries) = block.directives.get("max_age") {
        used.insert("max_age".to_string());
        for e in entries {
            if let Some(val) = e.args.first().and_then(|v| v.as_number()) {
                if val < 0 {
                    return Err("Invalid `max_age` — must be non-negative".into());
                }
            } else {
                return Err("Invalid `max_age` — expected a number".into());
            }
        }
    }
    if let Some(entries) = block.directives.get("expose_headers") {
        used.insert("expose_headers".to_string());
        for e in entries {
            for arg in &e.args {
                if arg.as_str().is_none() {
                    return Err("Invalid `expose_headers` — expected a string".into());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::layer::LayeredConfiguration;
    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    };
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_context_with_layer(block: ServerConfigurationBlock) -> ferron_http::HttpContext {
        let mut layered = LayeredConfiguration::new();
        layered.add_layer(Arc::new(block));
        ferron_http::HttpContext {
            req: None,
            res: None,
            events: ferron_observability::CompositeEventSink::new(Vec::new()),
            configuration: layered,
            hostname: None,
            variables: Default::default(),
            previous_error: None,
            original_uri: None,
            encrypted: false,
            local_address: "127.0.0.1:8080".parse().unwrap(),
            remote_address: "127.0.0.1:12345".parse().unwrap(),
            auth_user: None,
            https_port: None,
            extensions: typemap_rev::TypeMap::new(),
        }
    }

    fn make_block(directives: Vec<(&str, Vec<Vec<&str>>)>) -> ServerConfigurationBlock {
        let mut map: HashMap<String, Vec<ServerConfigurationDirectiveEntry>> = HashMap::new();
        for (name, entries) in directives {
            let parsed: Vec<ServerConfigurationDirectiveEntry> = entries
                .into_iter()
                .map(|args| {
                    let parsed_args: Vec<ServerConfigurationValue> = args
                        .into_iter()
                        .map(|s| ServerConfigurationValue::String(s.to_string(), None))
                        .collect();
                    ServerConfigurationDirectiveEntry {
                        args: parsed_args,
                        children: None,
                        span: None,
                    }
                })
                .collect();
            map.insert(name.to_string(), parsed);
        }
        ServerConfigurationBlock {
            directives: Arc::new(map),
            matchers: Default::default(),
            span: None,
        }
    }

    #[test]
    fn parses_header_append() {
        let ctx = make_context_with_layer(make_block(vec![(
            "header",
            vec![vec!["+X-Custom", "value-{{remote_address}}"]],
        )]));

        let cfg = parse_headers_config(&ctx).unwrap().unwrap();
        assert_eq!(cfg.header_actions.len(), 1);
        match &cfg.header_actions[0] {
            HeaderAction::Append(name, value) => {
                assert_eq!(name.as_str(), "x-custom");
                assert!(value.contains("{{remote_address}}"));
            }
            _ => panic!("expected Append action"),
        }
    }

    #[test]
    fn parses_header_remove() {
        let ctx = make_context_with_layer(make_block(vec![("header", vec![vec!["-X-Sensitive"]])]));

        let cfg = parse_headers_config(&ctx).unwrap().unwrap();
        assert_eq!(cfg.header_actions.len(), 1);
        match &cfg.header_actions[0] {
            HeaderAction::Remove(name) => {
                assert_eq!(name.as_str(), "x-sensitive");
            }
            _ => panic!("expected Remove action"),
        }
    }

    #[test]
    fn parses_cors_block() {
        let mut cors_map: HashMap<String, Vec<ServerConfigurationDirectiveEntry>> = HashMap::new();
        cors_map.insert(
            "origins".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(
                    "https://example.com".to_string(),
                    None,
                )],
                children: None,
                span: None,
            }],
        );
        cors_map.insert(
            "methods".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![
                    ServerConfigurationValue::String("GET".to_string(), None),
                    ServerConfigurationValue::String("POST".to_string(), None),
                ],
                children: None,
                span: None,
            }],
        );
        cors_map.insert(
            "credentials".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(true, None)],
                children: None,
                span: None,
            }],
        );
        let cors_block = ServerConfigurationBlock {
            directives: Arc::new(cors_map),
            matchers: Default::default(),
            span: None,
        };

        let mut outer_map: HashMap<String, Vec<ServerConfigurationDirectiveEntry>> = HashMap::new();
        outer_map.insert(
            "cors".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![],
                children: Some(cors_block.clone()),
                span: None,
            }],
        );
        let outer_block = ServerConfigurationBlock {
            directives: Arc::new(outer_map),
            matchers: Default::default(),
            span: None,
        };

        let ctx = make_context_with_layer(outer_block);
        let cfg = parse_headers_config(&ctx).unwrap().unwrap();
        let cors = cfg.cors.expect("cors should be present");
        assert_eq!(cors.origins, vec!["https://example.com"]);
        assert_eq!(cors.methods, vec!["GET", "POST"]);
        assert!(cors.credentials);
    }

    #[test]
    fn parses_header_replace() {
        let ctx = make_context_with_layer(make_block(vec![(
            "header",
            vec![vec!["X-Powered-By", "Titanium"]],
        )]));

        let cfg = parse_headers_config(&ctx).unwrap().unwrap();
        assert_eq!(cfg.header_actions.len(), 1);
        match &cfg.header_actions[0] {
            HeaderAction::Replace(name, value) => {
                assert_eq!(name.as_str(), "x-powered-by");
                assert_eq!(value, "Titanium");
            }
            _ => panic!("expected Replace action"),
        }
    }
}
