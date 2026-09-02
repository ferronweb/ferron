---
title: "Module development: overview"
description: "Overview of Ferron 3 module development, what modules are, and where to start."
---

Ferron can be extended through custom external modules that provide custom functionality for your Ferron server.

Ferron 3 modules are Rust library crates. The server loads them at compile
time through the `ferron-entrypoint` crate. You add a module to a custom
Ferron binary and the server calls its `ModuleLoader` hooks during start-up.

This section explains how Ferron modules work, how to create your first
module, and how to package and test it.

## What a module can do

A module can:

- Add a pipeline stage that runs for each HTTP request (`Stage<HttpContext>`).
- Run a custom server that listens on its own socket (`Module` + `Runtime`).
- Provide an observability sink that stores or forwards logs, metrics, or traces (`Provider<ObservabilityContext>`).
- Provide a TLS certificate resolver (`Provider<TlsContext>` + `TlsResolver`).
- Provide a DNS client for ACME DNS-01 challenges, and anything that needs DNS (`Provider<DnsContext>` + `DnsClient`).
- Load configuration from a new source (`ConfigurationAdapter`).
- Format access or application logs (`Provider<LogFormatterContext>`).

Each capability corresponds to a trait in `ferron-core` or a shared type in
`types/*`. You implement only the traits your module needs.

## Prerequisites

- Familiarity with Rust programming.
- Rust toolchain from [rustup.rs](https://rustup.rs/).
- Clone of Ferron `3.x` for reference and for `cargo doc`:
  `git clone https://github.com/ferronweb/ferron -b 3.x`.
- Read [Architecture](/docs/v3/module-development/concepts/architecture) to learn how
  the server orders stages and manages runtimes.

## Where to go next

- [Architecture](/docs/v3/module-development/concepts/architecture): how Ferron starts, how the pipeline runs, and how the two runtimes work.
- [Creating a module](/docs/v3/module-development/guides/creating-a-module): step-by-step guide to write a `ModuleLoader` crate.
- [Module API](/docs/v3/module-development/concepts/module-api): introduction to the traits you implement.
- [Naming conventions](/docs/v3/module-development/guides/naming-conventions): how to name directives and crates.
- [Example modules](/docs/v3/module-development/guides/examples): runnable examples you can copy.
- [Obtaining API docs](/docs/v3/module-development/guides/obtaining-api-docs): how to generate local Rust docs for `ferron-core` and `types/*`.

> [!tip]
> Start with a minimal HTTP stage (see the `ferron-http-header-append` example). It is the shortest path to a working module and covers most concepts you need for other kinds.
