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

1. **`location` with `remove_base=#false`** - the tool generates `match` + `if` blocks that may need manual adjustment.
2. **Match names** - generated `match` block names may be verbose. You should rename them for clarity.
3. **Complex `log_format`** - custom log format strings may need manual review to ensure placeholder names are correct.
4. **`fcgi_php`** - the `fcgi_php` directive is preserved but may need adjustment depending on your FastCGI setup.
5. **Rego subconditions** - Rego-based conditions are not migrated. You need to rewrite them using standard match expressions.

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
    http {
        timeout "5m"
    }

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
    tls /path/to/cert.pem /path/to/key.pem
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
        provider otlp

        logs http://localhost:4317
        metrics http://localhost:4317
        traces http://localhost:4317
    }
}
```

Ferron 3 also introduces a `log_style modern` directive in the OTLP observability block, enabled by default (unlike the previous `log_style legacy` behavior). See [OTLP observability](/docs/v3/configuration/observability/otlp#log-style) for the field mapping.

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

        algorithm round_robin
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
    header +X-Frame-Options "DENY"
    header -X-Powered-By

    proxy {
        upstream http://localhost:3000

        request_header +X-Real-IP "{{remote.ip}}"
        request_header -Host
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

Ferron 2 defaulted to TLS-ALPN-01. Ferron 3 defaults to **HTTP-01**. If you rely on TLS-ALPN-01, specify it explicitly:

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
    http {
        timeout 30           # Plain number = seconds
        #timeout "30s"       # Duration with suffix
    }
}
```

### Mixing `condition` blocks with `match` blocks

The most common pitfall is mixing the old `condition`/`if`/`if_not` syntax with the new `match`/`if`/`if_not` syntax. **These are not interchangeable** — they are two entirely different systems.

In Ferron 2, `condition` blocks used subconditions like `is_equal`, `is_not_equal`, `is_regex`, `is_not_regex`, `is_remote_ip`, `is_forwarded_for`, and `is_language`. In Ferron 3, these are replaced by `match` blocks with expression operators (`==`, `!=`, `~`, `!~`, `in`).

If you accidentally use a Ferron 2 `condition` block in a Ferron 3 configuration, the server will fail to parse it. Similarly, if you use a Ferron 2 `if`/`if_not` referencing an old `condition` name while also defining a `match` block with a similar name, the two systems will not connect — the `if`/`if_not` will reference the old condition name, not the new `match` block.

**Example of the pitfall** — this will **not** work:

```ferron
# INVALID: mixing condition (Ferron 2) with if (Ferron 3)
condition "IS_API" {
    is_regex "{path}" "^/api(/|$)"   # Ferron 2 syntax — ignored
}

example.com {
    if "IS_API" {                     # References the old condition, not match
        proxy http://localhost:3000
    }
}
```

**Correct approach** — use `match` throughout:

```ferron
# VALID: all Ferron 3
match api_request {
    request.uri.path ~ "/api"
}

example.com {
    if api_request {
        proxy http://localhost:3000
    }
}
```

### Placeholders in conditionals

Even if you migrate `condition` → `match`, you must also migrate the placeholder syntax used inside subconditions. Ferron 2 used `{path}`, `{client_ip}`, `{header:name}` etc. inside `condition` blocks. Ferron 3 uses `request.uri.path`, `remote.ip`, `request.header.name` etc. inside `match` blocks.

**Example of the pitfall** — this will **not** work:

```ferron
# INVALID: match block using Ferron 2 placeholders
match api_request {
    request.uri.path ~ "{path}"   # "{path}" is a Ferron 2 placeholder — ignored
}
```

**Correct approach** — use Ferron 3 variables:

```ferron
# VALID: match block using Ferron 3 variables
match api_request {
    request.uri.path ~ "/api"
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

> [!important]
> If `ferron validate` reports errors, address them before deploying to production.
