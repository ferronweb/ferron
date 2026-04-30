---
title: "Migrating from Ferron 2 to Ferron 3"
description: "Step-by-step guide for migrating a Ferron 2 KDL configuration to Ferron 3, including the migration tool, rollback steps, manual adjustments, and a verification checklist."

---

This guide shows how to migrate your Ferron 2 configuration (`.kdl`) to Ferron 3 (`.conf`).

Ferron 3 uses a new configuration format, updated observability, and a more explicit routing model. **Most Ferron 2 configs can be migrated with only a few manual changes.** The safest approach is to keep the original Ferron 2 config untouched, generate a new Ferron 3 config beside it, and validate before switching traffic.

## Quick summary

Upgrading from Ferron 2 to Ferron 3 takes five steps:

1. Back up the Ferron 2 configuration and note the current service/package state
2. Replace the Ferron 2 installation with Ferron 3
3. Run the migration tool into a new output file
4. Review and validate the generated config
5. Switch Ferron 3 into service only after validation succeeds

```bash
# Replace Ferron 2 with Ferron 3
apt remove ferron && apt install ferron3

# Migrate your config into a new file
ferron-kdl2ferron ferron.kdl ferron.conf.new

# Validate the result
ferron validate ferron.conf.new

# Start the server
ferron run -c ferron.conf.new
```

Most setups work with minimal changes. Read on for the details.

## Make the migration safer

Before you change anything, keep a copy of the working Ferron 2 config and, if possible, the current service/package state.

1. Copy `ferron.kdl` to a backup file such as `ferron.kdl.bak`.
2. Convert the config into a separate file such as `ferron.conf.new`.
3. Validate the new config before replacing any live config path.
4. Compare the generated file with the backup if you want to review the exact changes.
5. Keep the old Ferron 2 config until Ferron 3 has served traffic successfully.

## Replacing Ferron 2 with Ferron 3

Replace Ferron 2 before migrating the configuration. Use the path that matches how you installed it.

### Docker

If you installed Ferron 2 via an official Docker image, change `2` in the tag to `3`. For example, `ferronserver/ferron:2-alpine` becomes `ferronserver/ferron:3-alpine`.

### Windows installer

Back up `ferron.kdl` first, then run the Ferron 2 uninstaller as an administrator. After that, install Ferron 3 from the [downloads page](/download).

### Debian package

```bash
sudo apt remove ferron
sudo apt install ferron3
```

### RPM package

```bash
sudo yum remove ferron
sudo yum install ferron3
```

### Linux installer script

```bash
sudo systemctl disable ferron # If using systemd
#sudo update-rc.d remove ferron # If not using systemd

# Remove Ferron 2 files
sudo rm -rf /usr/sbin/ferron /usr/sbin/ferron-passwd /usr/sbin/ferron-yaml2kdl /usr/sbin/ferron-precompress /etc/.ferron-installer.prop /etc/systemd/system/ferron.service /etc/init.d/ferron

# Remove old Ferron user
sudo userdel ferron

# Install Ferron 3
sudo bash -c "$(curl -fsSL https://get.ferron.sh/v3)"
```

### Rolling back to Ferron 2

If you need to roll back, stop Ferron 3, restore the Ferron 2 config backup, and reinstall Ferron 2 using the original install method before starting the old service again.

- **Docker**: switch the image tag back from `:3` to `:2`.
- **Windows installer**: uninstall Ferron 3, reinstall Ferron 2, then restore `ferron.kdl`.
- **Debian/RPM**: remove `ferron3`, reinstall the Ferron 2 package you were using before, then copy back the old config.
- **Linux installer script**: remove the Ferron 3 files, reinstall Ferron 2 from the previous script or package, then restore the old config and service setup.

## Using the migration tool

Ferron 3 includes a migration tool that converts Ferron 2 `.kdl` configuration files to Ferron 3 `.conf` format.

### Basic usage

```bash
ferron-kdl2ferron input.kdl output.conf
```

This reads `input.kdl` and writes the converted Ferron 3 configuration to `output.conf`.

### What the migration tool does

The tool handles these conversions automatically:

- `*` global block → bare `{ }` global block
- `auto_tls` → `tls { provider acme }`
- `auto_tls_contact` → `tls { contact ... }`
- `tls cert key` → `tls { provider manual cert ... key ... }`
- `log` / `error_log` → `observability { provider file ... }`
- `otlp_logs` / `otlp_metrics` / `otlp_traces` → `observability { provider otlp ... }`
- `location` blocks → `location` blocks (without `remove_base`)
- `proxy` directives → `proxy { upstream ... }` blocks
- `proxy_request_header` → `request_header` with `+`/`-` prefix
- `user` directives → `basic_auth { users { ... } }`
- `limit` → `rate_limit`
- `block` / `allow` → preserved as-is
- `snippet` / `use` → `snippet` / `use` preserved
- `include` → `include` preserved

### Known limitations

The migration tool provides a **starting point**, not a perfect conversion. Keep these limitations in mind:

1. **`location` with `remove_base=#false`** - the tool may generate `location` blocks that need manual adjustment, since Ferron 3 always strips the base path.
2. **Match names** - generated `match` block names may be verbose. You should rename them for clarity.
3. **Placeholders in proxy paths** - `{client_ip}` and similar placeholders in proxy URLs need to be converted to `{{remote.ip}}` interpolated strings.
4. **Complex `log_format`** - custom log format strings may need manual review to ensure placeholder names are correct.
5. **`fcgi_php`** - the `fcgi_php` directive is preserved but may need adjustment depending on your FastCGI setup.
6. **Rego subconditions** - Rego-based conditions are not migrated. You need to rewrite them using standard match expressions.
7. **`trust_x_forwarded_for`** - this is converted to `client_ip_from_header "x-forwarded-for" { trusted_proxy "0.0.0.0/0" }`.

### After migration: manual review checklist

After running the migration tool, review the generated config for:

- [ ] All `condition` blocks converted to `match` blocks
- [ ] All `{placeholder}` references in `match` blocks converted to `request.*` variables
- [ ] `location` blocks with `remove_base=#false` adjusted for automatic base removal
- [ ] Proxy paths with placeholders converted to `{{interpolated}}` strings
- [ ] `observability` blocks reviewed for correct provider and format
- [ ] TLS configuration verified (provider, challenge type, contact)
- [ ] Include paths updated if needed
- [ ] Duration strings use suffix syntax (`30s`, `1h`) where appropriate

## What's changed

### Configuration format

| Ferron 2                            | Ferron 3                             |
| ----------------------------------- | ------------------------------------ |
| `.kdl` files                        | `.conf` files                        |
| `#true`, `#false`, `#null` booleans | `true`, `false`                      |
| `globals { }` for global config     | `{ }` (bare block) for global config |
| `duration 30000` for durations      | `30s`, `1h`, `90s` (suffix syntax)   |

### Global block

In Ferron 2 you used `globals` for global settings. In Ferron 3, use a bare block:

```kdl
// Ferron 2
globals {
    timeout 300000
    io_uring
}
```

```ferron
# Ferron 3
{
    timeout "5m"

    runtime {
        io_uring true
    }
}
```

### `location` behavior

In Ferron 2, `location` blocks used a `remove_base` property to control whether the matched prefix was stripped from the URL. In Ferron 3, the base path is **always automatically removed** — there is no `remove_base` property.

```kdl
// Ferron 2
example.com {
    location "/api" remove_base=#true {
        proxy "http://localhost:3000"
    }

    location "/" {
        root "/var/www/html"
    }
}
```

```ferron
# Ferron 3 — `remove_base` is no longer needed
example.com {
    location /api {
        proxy http://localhost:3000
    }

    location / {
        root /var/www/html
    }
}
```

If you had `remove_base=#false` in Ferron 2 (keeping the base path), you need to handle this differently in Ferron 3. The matched prefix is always stripped. To preserve the path, you would need to use URL rewriting or adjust your backend accordingly.

### Conditionals: `condition` → `match`

Ferron 2 used `condition` to define named checks and `if`/`if_not` to apply them. Ferron 3 uses `match` for the same purpose, but with a different syntax for subconditions:

```kdl
// Ferron 2
example.com {
  condition "IS_API" {
    is_regex "{path}" "^/api(/|$)"
  }

  if "IS_API" {
    proxy "http://127.0.0.1:3000"
  }

  if_not "IS_API" {
    root "/var/www/html"
  }
}
```

```ferron
# Ferron 3 — use `match` with expression syntax
match api_request {
    request.uri.path ~ "/api"
}

example.com {
    if api_request {
        proxy http://localhost:3000
    }

    if_not api_request {
        root /var/www/html
    }
}
```

Key differences:

- `condition` is replaced by `match`
- Subconditions become expressions (e.g., `request.uri.path ~ "/api"`)
- Placeholders like `{path}` are replaced by variables like `request.uri.path`
- `is_language` is replaced by `in` operator on `request.header.accept_language`
- `is_equal` / `is_not_equal` / `is_regex` / `is_not_regex` become `==`, `!=`, `~`, `!~`
- `is_remote_ip` / `is_forwarded_for` become `remote.ip ==` comparisons
- Rego subconditions are deprecated — use standard match expressions instead

### Placeholders in match blocks

Ferron 2 used `{placeholder}` syntax throughout. Ferron 3 uses `request.*` variables in `match` blocks and `{{env.VAR}}` for environment variables:

| Ferron 2 placeholder | Ferron 3 variable     |
| -------------------- | --------------------- |
| `{path}`             | `request.uri.path`    |
| `{path_and_query}`   | `request.uri`         |
| `{method}`           | `request.method`      |
| `{version}`          | `request.version`     |
| `{header:name}`      | `request.header.name` |
| `{scheme}`           | `request.scheme`      |
| `{client_ip}`        | `remote.ip`           |
| `{client_port}`      | `remote.port`         |
| `{server_ip}`        | `server.ip`           |
| `{server_port}`      | `server.port`         |

### TLS / ACME

The TLS configuration has been restructured. In Ferron 2, `auto_tls` and `auto_tls_contact` were separate directives. In Ferron 3, everything lives inside a `tls` block:

```kdl
// Ferron 2
example.com {
    auto_tls
    auto_tls_contact "admin@example.com"
    auto_tls_letsencrypt_production #true
}
```

```ferron
# Ferron 3
example.com {
    tls {
        provider acme
        challenge http-01
        contact "admin@example.com"
    }
}
```

For manual TLS, Ferron 2 used `tls cert key` while Ferron 3 uses:

```ferron
example.com {
    tls {
        provider manual
        cert /path/to/cert.pem
        key /path/to/key.pem
    }
}
```

### Observability / logging

Ferron 2 had separate `log`, `error_log`, `otlp_logs`, and `log_format` directives. Ferron 3 consolidates these under the `observability` block:

```kdl
// Ferron 2
example.com {
    log /var/log/ferron/access.log
    error_log /var/log/ferron/error.log
    log_json timestamp="{timestamp}" status="{status_code}"
    otlp_logs "http://localhost:4317" protocol="grpc"
    otlp_metrics "http://localhost:4318"
}
```

```ferron
# Ferron 3
example.com {
    observability {
        provider file

        access_log /var/log/ferron/access.log
        error_log /var/log/ferron/error.log
        format json 
        fields "timestamp" "status"
    }
}
```

For console logging:

```ferron
example.com {
    console_log {
        format json
    }
}
```

For OTLP (OpenTelemetry) export:

```ferron
example.com {
    observability {
        provider otlp {
            logs "http://localhost:4317" {
                protocol grpc
            }
            metrics "http://localhost:4317" {
                protocol grpc
            }
            traces "http://localhost:4317" {
                protocol grpc
            }
        }
    }
}
```

### Reverse proxying

The `proxy` directive syntax changed slightly. In Ferron 2, backends were specified as positional arguments. In Ferron 3, upstreams use the `upstream` directive inside a `proxy` block:

```kdl
// Ferron 2
example.com {
    proxy "http://localhost:3000"
    proxy "http://localhost:3001"
    lb_algorithm round_robin
    proxy_keepalive
}
```

```ferron
# Ferron 3
example.com {
    proxy {
        upstream http://localhost:3000
        upstream http://localhost:3001

        lb_algorithm round_robin
        keepalive
    }
}
```

### Header manipulation

```kdl
// Ferron 2
example.com {
    header "X-Frame-Options" "DENY"
    header_remove "X-Powered-By"
    proxy "http://localhost:3000"
    proxy_request_header "X-Real-IP" "{client_ip}"
    proxy_request_header_remove "Host"
}
```

```ferron
# Ferron 3
example.com {
    header "X-Frame-Options" "DENY"
    header_remove "X-Powered-By"

    proxy {
        upstream http://localhost:3000

        request_header "+X-Real-IP" "{{remote.ip}}"
        request_header "-Host"
    }
}
```

Note: In Ferron 3, `+` prefix adds a header, `-` prefix removes a header, and bare names replace.

### Include syntax

Ferron 2 used `include "/path/to/*.kdl"`. Ferron 3 uses `include "/path/to/*.conf"`:

```kdl
// Ferron 2
//include "/etc/ferron.d/**/*.kdl"
```

```ferron
# Ferron 3
#include "/etc/ferron/conf.d/**/*.conf"
```

## Before → After examples

### Simple static site

```kdl
// Ferron 2
example.com {
    root "/var/www/html"
}
```

```ferron
# Ferron 3
example.com {
    root /var/www/html
}
```

### Reverse proxy with static files

```kdl
// Ferron 2
example.com {
    location "/api" remove_base=#true {
        proxy "http://localhost:3000/api"
    }

    location "/" {
        root "/var/www/html"
    }
}
```

```ferron
# Ferron 3
example.com {
    location /api {
        proxy http://localhost:3000
    }

    location / {
        root /var/www/html
    }
}
```

### Conditional routing

```kdl
// Ferron 2
example.com {
  condition "IS_API" {
    is_regex "{path}" "^/api(/|$)"
  }

  if "IS_API" {
    proxy "http://127.0.0.1:3000"
  }

  if_not "IS_API" {
    root "/var/www/html"
  }
}
```

```ferron
# Ferron 3
match api_request {
    request.uri.path ~ "/api"
}

example.com {
    if api_request {
        proxy http://localhost:3000
    }

    if_not api_request {
        root /var/www/html
    }
}
```

### Automatic TLS

```kdl
// Ferron 2
example.com {
    auto_tls
    auto_tls_contact "admin@example.com"
    root "/var/www/html"
}
```

```ferron
# Ferron 3
example.com {
    tls {
        provider acme
        challenge http-01
        contact "admin@example.com"
    }

    root /var/www/html
}
```

### Manual TLS

```kdl
// Ferron 2
secure.example.com {
    tls "/etc/ssl/cert.pem" "/etc/ssl/key.pem"
    root "/var/www/html"
}
```

```ferron
# Ferron 3
secure.example.com {
    tls /etc/ssl/cert.pem /etc/ssl/key.pem
    root /var/www/html
}
```

### Logging with OTLP

```kdl
// Ferron 2
example.com {
    log /var/log/ferron/access.log
    error_log /var/log/ferron/error.log
    otlp_logs "http://localhost:4317" protocol="grpc"
```

```ferron
# Ferron 3
example.com {
    log /var/log/ferron/access.log
    error_log /var/log/ferron/error.log
    observability {
        provider otlp {
            logs "http://localhost:4317" {
                protocol grpc
            }
        }
    }
}
```

## Known pitfalls

### `location` always removes the base path

In Ferron 2, `location "/api" remove_base=#false` kept `/api` in the forwarded URL. In Ferron 3, the base path is **always** stripped. If your backend expects the full path, adjust the backend URL or use a rewrite rule.

**Example**: If you had `location "/api" { proxy "http://backend" }` with `remove_base=#false`, the Ferron 3 equivalent is simply:

```ferron
example.com {
    location /api {
        proxy http://backend/api
    }
}
```

The `/api` prefix is stripped from the request URL before proxying, so the backend still receives `/api` from the proxy URL.

### Handler execution order

Ferron 3 processes directives in a more defined order:

1. Global block configuration
2. Host block selection (by hostname/IP)
3. `location` blocks (longest prefix match wins)
4. `if` / `if_not` blocks

This is similar to Ferron 2, but the exact ordering of inherited directives may differ in complex configurations. Test thoroughly.

### ACME challenge type

Ferron 2 defaulted to TLS-ALPN-01 in some versions. Ferron 3 defaults to **HTTP-01**. If you rely on TLS-ALPN-01, specify it explicitly:

```ferron
example.com {
    tls {
        provider acme
        challenge tls-alpn-01
        contact "admin@example.com"
    }
}
```

### Header name normalization

In Ferron 3 `match` blocks, header names are normalized: lowercased with `_` converted to `-`. So `request.header.x_forwarded_for` reads the `x-forwarded-for` header.

### Duration strings

Ferron 2 used `duration 30000` syntax. Ferron 3 accepts bare duration strings:

```ferron
{
    timeout 1       # Plain number = hours (backward compatible)
    keepalive "30m"      # Duration with suffix
}
```

## Final verification checklist

Before switching to production:

- [ ] Run `ferron validate ferron.conf` — no errors
- [ ] Test routes behave as expected (proxy, static files, rewrites)
- [ ] TLS works (if enabled) — check certificate issuance and renewal
- [ ] Logs show no errors or warnings on startup
- [ ] Conditionals (`match`/`if`) evaluate correctly for your traffic patterns
- [ ] Proxy backends receive expected paths and headers
- [ ] DNS-01 challenge works (if using wildcard certificates)
- [ ] Observability (logging, OTLP) is sending data correctly

## Notes and troubleshooting

- The migration tool is a starting point. Manual review and adjustments are expected, especially for conditionals and complex proxy configurations.
- If `ferron validate` reports errors, address them before deploying to production.
- For `match` block expressions, see [Conditionals and variables](/docs/v3/configuration/conditionals).
- For `location` behavior, see [Routing and URL processing](/docs/v3/configuration/routing-url-processing).
- For TLS configuration, see [ACME automatic TLS](/docs/v3/configuration/tls-acme).
- For observability configuration, see [Observability and logging](/docs/v3/configuration/observability-logging).
- For the full Ferron 3 configuration reference, see [Syntax and file structure](/docs/v3/configuration/syntax).
