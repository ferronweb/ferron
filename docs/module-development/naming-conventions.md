---
title: "Naming conventions"
description: "Ferron 3 naming conventions for directives, crates, and modules."
---

Use consistent names so configuration, code, and docs stay aligned.

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

The module loader struct is `<Feature>ModuleLoader` (e.g.
`HttpRewriteModuleLoader`, `MemoryDnsModuleLoader`).

## Directive names

Directives are **lower_snake_case** and use the Ferron configuration style:

- No semicolons (newline terminated).
- Bare strings without quotes unless the value contains spaces or special chars.
- Boolean flags are bare (`directive`) for `true`, `directive false` for `false`.
- Raw string literals `r"..."` for regex patterns.

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

Scoped providers use a `provider` directive whose value selects the
implementation:

```ferron
{
    tls {
        provider selfsigned
        selfsigned {
            days 365
        }
    }

    observability {
        provider memory
        memory {
            max_events 1000
        }
    }
}
```

The provider name returned by `Provider::name()` must exactly match the value
users write in `provider <name>`. Keep it short, lower case, and without
underscores when possible.

## Configuration keys

- Use descriptive names that match the feature, e.g. `max_events`, `days`,
  `listen`.
- Do not prefix keys with the module name when inside a scoped block: the block
  already scopes them. Example: inside `selfsigned { ... }` use `days`, not
  `selfsigned_days`.

## Module names (`Module::name()` / `Stage::name()`)

- `Stage::name()` is used for `StageConstraint::Before` / `After`. It must be
  unique per context type (`HttpContext`, etc.). Use lower snake case that
  matches the feature, e.g. `example_header`, `hello`, `headers`.
- `Module::name()` is used for logs (`Starting module: <name>`). Use lower
  snake case.

## Versioning

Keep crate version in sync with the Ferron core version when possible (e.g.
`3.0.0-beta.11`). Example modules in
`https://github.com/ferronweb/ferron3-example-modules` track the upstream
`3.x` branch via `branch = "3.x"` in `Cargo.toml`. When upstream releases a
stable version, update the git dependency or publish a versioned crate.

## Documentation

- One-sentence `description` in `Cargo.toml` that states what the crate does.
- Module-level docs (`//!`) that show a minimal `ferron` config block.
- `README.md` with the same config snippet so it renders on crates.io / GitHub.
