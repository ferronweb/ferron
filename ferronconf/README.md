# `ferronconf`

A Rust library for parsing `ferron.conf` configuration files — a domain-specific language for custom web server configurations.

## Overview

This crate provides a reference implementation of the `ferron.conf` format, including:

- **Lexer** — Tokenizes configuration files with support for comments, strings, numbers, booleans, and interpolation
- **Parser** — Builds an AST from tokens with full error reporting
- **AST** — Type-safe representation of configuration structures

For the complete format specification, see [SPECIFICATION.md](./SPECIFICATION.md).

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
ferronconf = "0.1.0"
```

## Quick start

```rust
use ferronconf::Config;
use std::str::FromStr;

let input = r#"
example.com {
    root /var/www/example
    ssl true
}

*.example.com:443 {
    proxy http://backend
}
"#;

let config = Config::from_str(input)?;
```

## Configuration format

The `ferron.conf` format supports five statement types:

### 1. Directives

Key-value pairs with optional nested blocks:

```ferron
server_name example.com
max_connections 1000
enabled true
root "{{app.root}}"
```

### 2. Host blocks

Configuration scoped to specific hosts (top-level only):

```ferron
# Simple hostname
example.com {
    root /var/www/example
}

# Wildcard subdomains
*.example.com {
    ssl enabled
}

# With protocol and port
http://api.example.com:8080 {
    proxy http://backend
}

# IPv6
[2001:db8::1]:8080 {
    root /ipv6-only
}

# Multiple hosts (comma-separated)
example.com, www.example.com {
    shared_config true
}
```

### 3. Global blocks

Global configuration applied to all hosts (top-level only):

```ferron
{
    default_timeout 30s
    log_format combined
}
```

### 4. Snippet blocks

Reusable configuration fragments:

```ferron
snippet ssl_config {
    ssl_certificate /etc/ssl/cert.pem
    ssl_key /etc/ssl/key.pem
}
```

### 5. Match blocks

Conditional logic based on request attributes:

```ferron
match api_rules {
    request.path ~ "^/api/"
    request.method in "GET,POST"
    request.header.user_agent != "curl"
}
```

Supported operators: `==`, `!=`, `~` (regex), `!~` (negated regex), `in`

### Comments

```ferron
# This is a comment
server_name example.com  # inline comment
```

**Note:** Comments are not allowed inside `match` blocks.

## Data types

| Type | Example | Description |
|------|---------|-------------|
| String (quoted) | `"hello world"` | Supports escape sequences (`\n`, `\t`, `\\`, `\"`) |
| String (bare) | `example.com` | Unquoted alphanumeric with `_-.:/+*` |
| Number | `42`, `3.14`, `-10` | Integer or decimal |
| Boolean | `true`, `false` | Case-sensitive literals |
| Interpolation | `{{app.root}}` | Variable reference with dotted path |

## Library API

### Parsing

```rust
use ferronconf::Config;
use std::str::FromStr;

let config = Config::from_str(input)?;
```

### AST navigation

```rust
use ferronconf::Statement;

// Find all directives with a given name
let roots = config.find_directives("root");

// Find all host blocks
let hosts = config.find_host_blocks();

// Find all match blocks
let matchers = config.find_match_blocks();

// Navigate statements
for stmt in &config.statements {
    match stmt {
        Statement::Directive(d) => {
            println!("Directive: {}", d.name);
            if let Some(root) = d.get_string_arg(0) {
                println!("  Root: {}", root);
            }
        }
        Statement::HostBlock(hb) => {
            for pattern in &hb.hosts {
                println!("Host: {}", pattern.as_str());
            }
        }
        Statement::MatchBlock(mb) => {
            println!("Matcher: {}", mb.matcher);
        }
        Statement::GlobalBlock(gb) => {
            // Access global configuration
        }
        Statement::SnippetBlock(sb) => {
            println!("Snippet: {}", sb.name);
        }
    }
}
```

### Block helpers

```rust
// Find directive inside a block
if let Some(block) = directive.block {
    if let Some(nested) = block.find_directive("ssl") {
        // ...
    }
}
```

### Value extraction

```rust
use ferronconf::Value;

// From directive arguments
if let Some(root) = directive.get_string_arg(0) {
    // ...
}

if let Some(port) = directive.get_integer_arg(1) {
    // ...
}

if let Some(enabled) = directive.get_boolean_arg(2) {
    // ...
}

// From Value directly
match &value {
    Value::String(s, _) => { /* ... */ }
    Value::Integer(i, _) => { /* ... */ }
    Value::Float(f, _) => { /* ... */ }
    Value::Boolean(b, _) => { /* ... */ }
    Value::InterpolatedString(parts, _) => {
        for part in parts {
            match part {
                StringPart::Literal(s) => { /* ... */ }
                StringPart::Expression(path) => { /* ... */ }
            }
        }
    }
}
```

### Host pattern matching

```rust
use ferronconf::ast::HostLabels;

for host_block in config.find_host_blocks() {
    for pattern in &host_block.hosts {
        match &pattern.labels {
            HostLabels::Wildcard => { /* matches any host */ }
            HostLabels::Hostname(labels) => { /* e.g., ["example", "com"] */ }
            HostLabels::IpAddr(ip) => { /* IPv4 or IPv6 */ }
        }
        
        if let Some(port) = pattern.port {
            // ...
        }
        
        if let Some(protocol) = &pattern.protocol {
            // e.g., "http", "tcp"
        }
    }
    
    // Check if block matches a specific host
    if host_block.matches_host("example.com") {
        // ...
    }
}
```

### Match block inspection

```rust
for match_block in config.find_match_blocks() {
    println!("Matcher: {}", match_block.matcher);
    
    for expr in &match_block.expr {
        match &expr.left {
            Operand::Identifier(path, _) => {
                println!("  Path: {}", path.join("."));
            }
            Operand::String(s, _) => {
                println!("  String: {}", s);
            }
            _ => {}
        }
        
        println!("  Operator: {}", expr.op.as_str());
        
        // Check operator type
        if expr.is_equality() { /* ... */ }
        if expr.is_regex() { /* ... */ }
    }
}
```

## Error handling

Parse errors include line and column information:

```rust
use ferronconf::{Config, ParseError};
use std::str::FromStr;

match Config::from_str(input) {
    Ok(config) => { /* ... */ }
    Err(ParseError { message, span }) => {
        eprintln!("Error at line {}, column {}: {}", 
                  span.line, span.column, message);
    }
}
```

## Syntax highlighting

A TextMate grammar is provided in `ferron.tmLanguage.json` for editor syntax highlighting. Copy it to your editor's grammar directory or use it with tools like [`bat`](https://github.com/sharkdp/bat) or [`syntect`](https://github.com/trishume/syntect).

## Example configuration

```ferron
# Global defaults
{
    default_timeout 30s
    log_format combined
}

# Reusable SSL configuration
snippet ssl_config {
    ssl_certificate /etc/ssl/cert.pem
    ssl_key /etc/ssl/key.pem
    ssl_protocols TLSv1.2 TLSv1.3
}

# Main site
example.com:443 {
    include ssl_config
    root /var/www/example
    
    if curl_client {
        use set_curl
    }
}

# Wildcard subdomains
*.example.com {
    reverse_proxy localhost:9000
}

# Conditional routing
match api_rules {
    request.path ~ "^/api/"
    request.method in "GET,POST,PUT,DELETE"
}

# Protocol-specific configuration
http * {
    header X-Powered-By MyServer
}

# TCP service
tcp *:5432 {
    proxy localhost:5432
}
```

## Limitations

- Bare strings after identifiers may be ambiguous with host blocks at the top level
- Comments are not supported inside `match` blocks
- IPv4 octets are validated to be in range 0–255

## License

[Your license here]
