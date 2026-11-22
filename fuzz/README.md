# Ferron fuzz targets

Fuzzing setup for Ferron's HTTP pipeline and core parsers, powered by [`cargo-fuzz`](https://rust-fuzz.github.io/book/cargo-fuzz.html).

## Prerequisites

A nightly toolchain and `cargo-fuzz`:

```bash
cargo +nightly install cargo-fuzz
rustup toolchain install nightly
```

## Running a target

Run from inside the `fuzz/` directory:

```bash
cargo +nightly fuzz run fuzz_http_pipeline
```

Targets are listed as Rust source files in the `fuzz/fuzz_targets` directory.

Dictionaries live in `fuzz/dictionaries/` (such as `http_request.dict`, `traceparent.dict` or `canonicalize.dict`) and seed corpora in `fuzz/corpus/<target>/`. Crashes and other artifacts are written to `fuzz/artifacts/<target>/`.
