//! `/config` endpoint — returns sanitized server configuration as JSON.

use ferron_core::config::ServerConfiguration;
use serde_json::{Map, Value};

/// Known sensitive directive names (or substrings thereof) that should be redacted.
const SENSITIVE_DIRECTIVES: &[&str] = &[
    "key",
    "cert",
    "private_key",
    "password",
    "secret",
    "token",
    "ticket_keys",
    "bearer",
    "passwd",
    "htpasswd",
];

/// Check if a directive name is considered sensitive and should be redacted.
///
/// A directive is considered sensitive if its lowercase name contains any of
/// the configured sensitive keywords (e.g. `private_key`, `tls_cert`,
/// `auth_token`).
fn is_sensitive(name: &str) -> bool {
    let name_lower = name.to_lowercase();
    SENSITIVE_DIRECTIVES
        .iter()
        .any(|sensitive| name_lower.contains(sensitive))
}

/// Serialize a single configuration value to JSON.
fn value_to_json(value: &ferron_core::config::ServerConfigurationValue) -> Value {
    match value {
        ferron_core::config::ServerConfigurationValue::String(s, _) => Value::String(s.clone()),
        ferron_core::config::ServerConfigurationValue::Number(n, _) => Value::Number((*n).into()),
        ferron_core::config::ServerConfigurationValue::Float(f, _) => {
            serde_json::Number::from_f64(*f).map_or(Value::Null, Value::Number)
        }
        ferron_core::config::ServerConfigurationValue::Boolean(b, _) => Value::Bool(*b),
        ferron_core::config::ServerConfigurationValue::InterpolatedString(parts, _) => {
            let mut s = String::new();
            for part in parts {
                match part {
                    ferron_core::config::ServerConfigurationInterpolatedStringPart::String(t) => {
                        s.push_str(t)
                    }
                    ferron_core::config::ServerConfigurationInterpolatedStringPart::Variable(v) => {
                        s.push_str(&format!("{{{{{}}}}}", v))
                    }
                }
            }
            Value::String(s)
        }
    }
}

/// Serialize a configuration block to JSON, recursively redacting sensitive directives.
fn block_to_json(block: &ferron_core::config::ServerConfigurationBlock) -> Value {
    let mut map = Map::new();

    for (name, entries) in block.directives.iter() {
        if is_sensitive(name) {
            map.insert(name.clone(), Value::String("[redacted]".to_string()));
            continue;
        }

        let entries_json: Vec<Value> = entries
            .iter()
            .map(|entry| {
                let mut entry_map = Map::new();

                // Serialize args
                let args_json: Vec<Value> = entry.args.iter().map(value_to_json).collect();
                entry_map.insert("args".to_string(), Value::Array(args_json));

                // Serialize children if present
                if let Some(children) = &entry.children {
                    entry_map.insert("children".to_string(), block_to_json(children));
                }

                Value::Object(entry_map)
            })
            .collect();

        map.insert(name.clone(), Value::Array(entries_json));
    }

    Value::Object(map)
}

/// Sanitize the full server configuration for safe public exposure.
pub fn sanitize_config(config: &ServerConfiguration) -> Value {
    let mut result = Map::new();

    // Global config
    result.insert(
        "global_config".to_string(),
        block_to_json(&config.global_config),
    );

    // Ports
    let ports_map: Map<String, Value> = config
        .ports
        .iter()
        .map(|(protocol, port_configs)| {
            let ports_json: Vec<Value> = port_configs
                .iter()
                .map(|pc| {
                    let mut pc_map = Map::new();
                    if let Some(port) = pc.port {
                        pc_map.insert("port".to_string(), Value::Number(port.into()));
                    }
                    let hosts_json: Vec<Value> = pc
                        .hosts
                        .iter()
                        .map(|(filters, block)| {
                            let mut host_map = Map::new();

                            // Filters
                            let mut filters_map = Map::new();
                            if let Some(ip) = filters.ip {
                                filters_map.insert("ip".to_string(), Value::String(ip.to_string()));
                            }
                            if let Some(host) = &filters.host {
                                filters_map.insert("host".to_string(), Value::String(host.clone()));
                            }
                            host_map.insert("filters".to_string(), Value::Object(filters_map));

                            // Block (sanitized)
                            host_map.insert("config".to_string(), block_to_json(block));

                            Value::Object(host_map)
                        })
                        .collect();
                    pc_map.insert("hosts".to_string(), Value::Array(hosts_json));

                    Value::Object(pc_map)
                })
                .collect();

            (protocol.clone(), Value::Array(ports_json))
        })
        .collect();

    result.insert("ports".to_string(), Value::Object(ports_map));
    Value::Object(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_sensitive_matches_known_substrings() {
        // Exact matches
        assert!(is_sensitive("password"));
        assert!(is_sensitive("private_key"));
        assert!(is_sensitive("htpasswd"));
        assert!(is_sensitive("secret"));

        // Compound names that should be redacted
        assert!(is_sensitive("tls_cert"));
        assert!(is_sensitive("ssl_private_key"));
        assert!(is_sensitive("auth_token"));
        assert!(is_sensitive("api_bearer"));
        assert!(is_sensitive("session_ticket_keys"));

        // Case insensitive
        assert!(is_sensitive("PASSWORD"));
        assert!(is_sensitive("Tls_Cert"));
    }

    #[test]
    fn is_sensitive_does_not_match_unrelated_directives() {
        // Unrelated directives that share no sensitive substring
        assert!(!is_sensitive("listen"));
        assert!(!is_sensitive("root"));
        assert!(!is_sensitive("server_name"));
        assert!(!is_sensitive("log_level"));
        assert!(!is_sensitive("max_connections"));

        // Single characters that previously caused false positives
        // (e.g. "m" in "max_memory" was matching "m" from "password"/"secret")
        assert!(!is_sensitive("m"));
        assert!(!is_sensitive("k"));
        assert!(!is_sensitive("s"));
        assert!(!is_sensitive("h"));
    }
}
