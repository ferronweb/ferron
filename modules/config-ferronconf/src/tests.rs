use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("invalid clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "ferron-config-ferronconf-{unique}-{}",
            std::process::id()
        ));

        fs::create_dir_all(&path).expect("failed to create test directory");

        Self { path }
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.path.join(name);
        fs::write(&path, contents).expect("failed to write configuration file");
        path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn adapt_file(path: &Path) -> ServerConfiguration {
    let mut params = HashMap::new();
    params.insert("file".to_string(), path.display().to_string());

    FerronConfConfigurationAdapter::new()
        .adapt(&params)
        .expect("configuration should adapt successfully")
        .0
}

#[test]
fn adapt_loads_includes_and_merges_global_and_host_blocks() {
    let dir = TestDir::new();
    let shared = dir.write(
        "shared.conf",
        r#"
{
    runtime {
        workers 4
    }
}

match is_cli {
    request.header.user_agent ~ "curl"
}

example.com {
    header X-Shared 1
}
"#,
    );

    let main = dir.write(
        "main.conf",
        r#"
include "shared.conf"

{
    runtime {
        io_uring true
    }
}

match is_api {
    request.path ~ "^/api"
}

example.com {
    root /srv/www
    if is_api {
        header X-Api 1
    }
}

http example.com {
    header X-Explicit 1
}

example.com {
    header X-Test 2
}

http example.com:8080 {
    header X-Port 8080
}
"#,
    );

    let config = adapt_file(&main);
    let shared_path = shared.display().to_string();

    let runtime = config
        .global_config
        .directives
        .get("runtime")
        .expect("runtime directives should exist");
    assert_eq!(runtime.len(), 2);
    assert_eq!(
        runtime[0]
            .span
            .as_ref()
            .and_then(|span| span.file.as_deref()),
        Some(shared_path.as_str())
    );

    let http_ports = config.ports.get("http").expect("http ports should exist");
    let default_port = http_ports
        .iter()
        .find(|port| port.port.is_none())
        .expect("default http port should exist");
    assert_eq!(default_port.hosts.len(), 1);

    let (filters, block) = &default_port.hosts[0];
    assert_eq!(filters.host.as_deref(), Some("example.com"));
    assert_eq!(block.directives.get("root").map(Vec::len), Some(1));
    assert_eq!(block.directives.get("header").map(Vec::len), Some(3));
    assert!(block.matchers.contains_key("is_api"));
    assert!(block.matchers.contains_key("is_cli"));

    let if_entry = block
        .directives
        .get("if")
        .and_then(|entries| entries.first())
        .expect("if directive should exist");
    let if_block = if_entry
        .children
        .as_ref()
        .expect("if directive should have a child block");
    assert!(if_block.matchers.contains_key("is_api"));
    assert!(if_block.matchers.contains_key("is_cli"));

    let port_8080 = http_ports
        .iter()
        .find(|port| port.port == Some(8080))
        .expect("http:8080 config should exist");
    assert_eq!(port_8080.hosts.len(), 1);
    assert_eq!(port_8080.hosts[0].0.host.as_deref(), Some("example.com"));
}

#[test]
fn adapt_expands_snippets_inside_blocks() {
    let dir = TestDir::new();
    dir.write(
        "shared.conf",
        r#"
snippet shared_defaults {
    header X-Shared 1
}
"#,
    );

    let main = dir.write(
        "main.conf",
        r#"
include "shared.conf"

snippet local_defaults {
    header X-Local 1
}

example.com {
    include shared_defaults
    use local_defaults
    header X-Direct 1
}
"#,
    );

    let config = adapt_file(&main);

    let http_ports = config.ports.get("http").expect("http ports should exist");
    let host = &http_ports
        .iter()
        .find(|port| port.port.is_none())
        .expect("default http port should exist")
        .hosts[0]
        .1;

    assert_eq!(host.directives.get("header").map(Vec::len), Some(3));
}

#[test]
fn adapt_rejects_include_cycles() {
    let dir = TestDir::new();
    let main = dir.write("main.conf", "include \"other.conf\"\n");
    dir.write("other.conf", "include \"main.conf\"\n");

    let mut params = HashMap::new();
    params.insert("file".to_string(), main.display().to_string());

    let result = FerronConfConfigurationAdapter::new().adapt(&params);
    assert!(result.is_err(), "cyclic includes should fail");
    let error = result.err().expect("result should contain an error");

    assert!(error.to_string().contains("Include cycle detected"));
}
