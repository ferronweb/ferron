---
title: "Obtaining the module API docs"
description: "How to generate and read Ferron 3 module API docs from the 3.x branch."
---

Ferron does not publish its internal API docs on docs.rs for every commit.
Generate them locally from the `3.x` branch.

## Clone Ferron

```bash
git clone https://github.com/ferronweb/ferron -b 3.x
cd ferron
```

## Generate Rust docs

For `ferron-core` and all domain types:

```bash
cargo doc --no-deps -p ferron-core -p ferron-http -p ferron-observability -p ferron-tls -p ferron-dns -p ferron-ocsp
```

Open the docs:

```bash
xdg-open target/doc/ferron_core/index.html  # Linux
open target/doc/ferron_core/index.html      # macOS
```

Key entry points:

- `ferron_core::loader::ModuleLoader`
- `ferron_core::pipeline::Stage` and `Pipeline`
- `ferron_core::registry::Registry` and `StageConstraint`
- `ferron_core::providers::Provider`
- `ferron_core::config::adapter::ConfigurationAdapter`
- `ferron_core::runtime::Runtime`

Domain crates:

- `ferron-http` (`types/http`): `HttpContext`, `HttpResponse`
- `ferron-observability` (`types/observability`): `Event`, `EventSink`, `ObservabilityContext`
- `ferron-tls` (`types/tls`): `TlsContext`, `TlsResolver`
- `ferron-dns` (`types/dns`): `DnsContext`, `DnsClient`

> [!tip]
> Run `cargo doc --document-private-items --no-deps` to see internal helpers
> that may not be public but are useful examples.

## User-facing docs

User docs live in `docs/` in the Ferron repository. The sidebar is
`docs/links.json`. Read them on GitHub or locally:

- `https://github.com/ferronweb/ferron/blob/3.x/docs/module-development/` (this section)
- `https://github.com/ferronweb/ferron/blob/3.x/docs/configuration/` (directive reference)

## Keeping docs in sync

The example modules repository (`https://github.com/ferronweb/ferron3-example-modules`)
contains its own `AGENTS.md` that tells agents to fetch docs from the upstream
`3.x` branch (user docs and `cargo doc`). When Ferron API changes, regenerate
docs and update your `Cargo.toml` branch reference (`branch = "3.x"` always
tracks the latest development API).

## Short guide in `AGENTS.md`

The example repo `AGENTS.md` summarizes:

```markdown
git clone https://github.com/ferronweb/ferron -b 3.x /tmp/ferron
cd /tmp/ferron
cargo doc --no-deps
```

and lists the GitHub URLs for direct reading without cloning.
