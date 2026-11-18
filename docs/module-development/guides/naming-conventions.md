---
title: "Naming conventions"
description: "Ferron 3 naming conventions for directives, crates, and modules."
---

Consistent names keep configuration, code, and docs aligned.

## Crate names

Crates use kebab-case with a Ferron prefix:

| Kind               | Pattern                          | Example                                            |
| ------------------ | -------------------------------- | -------------------------------------------------- |
| HTTP stage         | `ferron-http-<feature>`          | `ferron-http-rewrite`, `ferron-http-header-append` |
| Custom server      | `ferron-<name>-server`           | `ferron-echo-server`                               |
| Observability sink | `ferron-observability-<backend>` | `ferron-observability-otlp`                        |
| TLS provider       | `ferron-tls-<provider>`          | `ferron-tls-acme`                                  |
| DNS provider       | `ferron-dns-<provider>`          | `ferron-dns-cloudflare`                            |
| Config adapter     | `ferron-config-<format>`         | `ferron-config-toml`                               |
| Log formatter      | `ferron-logformat-<format>`      | `ferron-logformat-json`                            |

The module loader struct uses the pattern `<Feature>ModuleLoader`, for example `HttpRewriteModuleLoader` or `MemoryDnsModuleLoader`.

## Directive names

Directives use lower snake case and follow the Ferron configuration style:

- No semicolons, because a newline ends a directive.
- Bare strings without quotes, unless the value contains spaces or special characters.
- Bare boolean flags (`directive`) for `true`, and `directive false` for `false`.
- Raw string literals (`r"..."`) for regex patterns.

Examples:

```ferron
{
    example_header hello-world
    hello_path /greet
    echo_server {
        listen "127.0.0.1:9090"
    }
}
```

A scoped provider uses a `provider` directive whose value selects the implementation:

```ferron
{
    # tls {
    #     provider selfsigned
    #     selfsigned {
    #         days 365
    #     }
    # }

    # observability {
    #     provider memory
    #     memory {
    #         max_events 1000
    #     }
    # }

    console_log
}
```

The name returned by `Provider::name()` must match the value that users write in `provider <name>` exactly. Keep the name short and lower case, and avoid underscores when possible.

## Configuration keys

- Use descriptive names that match the feature, for example `max_events`, `days`, or `listen`.
- Do not prefix keys with the module name inside a scoped block, because the block already scopes them. For example, inside `selfsigned { ... }` use `days`, not `selfsigned_days`.

## Module names (`Module::name()` and `Stage::name()`)

- `Stage::name()` feeds `StageConstraint::Before` and `After`. It must be unique per context type (`HttpContext` and so on). Use lower snake case that matches the feature, for example `example_header`, `hello`, or `headers`.
- `Module::name()` appears in logs (`Starting module: <name>`). Use lower snake case.

## Versioning

Keep the crate version in sync with the Ferron core version when possible, for example `3.0.0-beta.11`. Example modules in `https://github.com/ferronweb/ferron3-example-modules` track the upstream `3.x` branch through `branch = "3.x"` in `Cargo.toml`. When upstream releases a stable version, update the git dependency or publish a versioned crate.

## Documentation

- One-sentence `description` in `Cargo.toml` that states what the crate does.
- Module-level docs (`//!`) that show a minimal `ferron` config block.
- `README.md` with the same config snippet, so it renders on crates.io and GitHub.
