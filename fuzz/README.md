# Ferron fuzz targets

Fuzzing setup for Ferron's HTTP pipeline and core parsers, powered by [`cargo-fuzz`](https://rust-fuzz.github.io/book/cargo-fuzz.html).

This crate (`ferron-fuzz`) is not part of the main workspace (the root `Cargo.toml` excludes `fuzz/`), so `cargo build/test --workspace` skips it. It is only used through the dedicated `cargo +nightly fuzz` commands below.

## Prerequisites

A nightly toolchain and `cargo-fuzz`:

```
cargo +nightly install cargo-fuzz
rustup toolchain install nightly
```

The fuzz targets build with the `fuzz` feature of the module crates (`ferron-http-server`, `ferron-http-proxy`, `ferron-http-cache`, `ferron-http-ratelimit`).

## Running a target

Run from inside the `fuzz/` directory:

```
cargo +nightly fuzz run fuzz_http_pipeline
```

Every target:

| Target                   | What it exercises                                                                                                                                                                                                                                                                                 |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `fuzz_http_pipeline`     | The full HTTP request pipeline end-to-end: request parsing (plain HTTP/1.1 and HTTP/2-style pseudo-header input), the request-handler stages (`ClientIpFromHeaderStage`, `HttpsRedirectStage`), and error paths. The handler must never panic; error responses (400, 404, 500, ...) are expected. |
| `fuzz_canonicalize_path` | URL path canonicalization with security invariants: no `..` traversal, no standalone `.` segments, no control characters, uppercase percent-encoding in the forwarded path, unreserved characters decoded in the routing path, and no double/nested encoding.                                     |
| `fuzz_load_balancers`    | Upstream load-balancer algorithms: consistent hashing (ring determinism and rebuild stability), weighted round robin, P2C/EWMA score and decay math (finiteness, non-negativity), and the backend selector over all algorithm variants.                                                           |
| `fuzz_cache`             | LSCache header parsing (`X-LiteSpeed-Cache-Control`, `Vary`, `Tag`, `Purge`) and cache-policy evaluation: age values are non-negative, vary cookies sorted, tags deduplicated, and purge operations always carry selectors.                                                                       |
| `fuzz_ratelimit`         | The token-bucket rate limiter under concurrent access: arbitrary thread/key/operation counts run against a shared `TokenBucketRegistry`, asserting no key ever exceeds its configured capacity.                                                                                                   |
| `fuzz_traceparent`       | W3C `traceparent` header parsing. Successful parses must yield a 32-char lowercase-hex trace ID and a 16-char lowercase-hex span ID.                                                                                                                                                              |
| `fuzz_qvalue`            | `Accept`/q-value header parsing (both the plain and grouped variants). Must never panic and must not produce empty groups.                                                                                                                                                                        |

Dictionaries live in `fuzz/dictionaries/` (e.g. `http_request.dict`, `traceparent.dict`, `canonicalize.dict`) and seed corpora in `fuzz/corpus/<target>/`. Crashes and other artifacts are written to `fuzz/artifacts/<target>/`.

## Adding a target

1. Add a `#![no_main]` harness under `fuzz_targets/`.
2. Declare it as a `[[bin]]` entry in `fuzz/Cargo.toml` with `test = false` and `doc = false`.
3. Seed a corpus directory at `fuzz/corpus/<target>/` if useful, then run it with `cargo +nightly fuzz run <target>`.
4. Update `fuzz/README.md` with the new target's description.
