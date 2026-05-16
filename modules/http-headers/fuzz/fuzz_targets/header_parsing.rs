#![no_main]

use arbitrary::Unstructured;
use libfuzzer_sys::fuzz_target;
use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue};

use ferron_http_headers::config::parse_headers_config;

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);

    // Random number of entries
    let entries_count = match u.arbitrary::<u8>() { Ok(n) => (n as usize) % 8, Err(_) => 0 };

    let mut map: HashMap<String, Vec<ServerConfigurationDirectiveEntry>> = HashMap::new();
    let mut header_entries: Vec<ServerConfigurationDirectiveEntry> = Vec::new();

    for _ in 0..entries_count {
        // First argument: header name (possibly prefixed with + or -)
        let first = match u.arbitrary::<String>() {
            Ok(s) => {
                let mut s = s;
                if s.is_empty() {
                    s = "X-Fuzz-Header".to_string();
                }
                // Randomly prefix
                if let Ok(pfx) = u.arbitrary::<u8>() {
                    match pfx % 3 {
                        0 => format!("+{}", s),
                        1 => format!("-{}", s),
                        _ => s,
                    }
                } else {
                    s
                }
            }
            Err(_) => "X-Fuzz-Header".to_string(),
        };

        // Maybe a second argument (value)
        let second_opt = u.arbitrary::<String>().ok();

        let mut args = Vec::new();
        args.push(ServerConfigurationValue::String(first, None));
        if let Some(sv) = second_opt {
            args.push(ServerConfigurationValue::String(sv, None));
        }

        let entry = ServerConfigurationDirectiveEntry {
            args,
            children: None,
            span: None,
        };
        header_entries.push(entry);
    }

    if !header_entries.is_empty() {
        map.insert("header".to_string(), header_entries);
    }

    let block = ServerConfigurationBlock {
        directives: Arc::new(map),
        matchers: Default::default(),
        span: None,
    };

    let mut layered = LayeredConfiguration::new();
    layered.add_layer(Arc::new(block));

    // Build a minimal HttpContext (we only need configuration for parsing)
    let mut ctx = ferron_http::HttpContext {
        req: None,
        res: None,
        events: ferron_observability::CompositeEventSink::new(Vec::new()),
        configuration: layered,
        hostname: None,
        variables: rustc_hash::FxHashMap::default(),
        previous_error: None,
        original_uri: None,
        routing_uri: None,
        encrypted: false,
        local_address: "127.0.0.1:8080".parse().unwrap(),
        remote_address: "127.0.0.1:12345".parse().unwrap(),
        auth_user: None,
        https_port: None,
        extensions: typemap_rev::TypeMap::new(),
    };

    // Call the parser — errors are expected for malformed inputs, but should not panic
    let _ = parse_headers_config(&ctx);
});
