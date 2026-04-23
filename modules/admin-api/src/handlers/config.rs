//! `/config` endpoint — returns sanitized server configuration as JSON.

use ferron_core::config::ServerConfiguration;
use serde_json::{Map, Value};

/// Known sensitive directive names that should be redacted.
const SENSITIVE_DIRECTIVES: &[&str] = &[
    "key",
    "cert",
    "private_key",
    "password",
    "secret",
    "token",
    "ticket_keys",
];

/// Check if a directive name is considered sensitive and should be redacted.
fn is_sensitive(name: &str) -> bool {
    SENSITIVE_DIRECTIVES.contains(&name)
        || name.contains(|c| SENSITIVE_DIRECTIVES.iter().any(|s| s.contains(c)))
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
    fn sanitize_config_produces_valid_json() {
        let config = ServerConfiguration::default();
        let json = sanitize_config(&config);
        assert!(json.is_object());
        assert!(json.get("global_config").is_some());
        assert!(json.get("ports").is_some());
    }
}
