# Fuzzing the URL Canonicalizer

This directory contains fuzz targets for testing the URL canonicalization functions in `ferron-http-server`.

## Prerequisites

Install `cargo-fuzz`:

```bash
cargo install cargo-fuzz
```

You'll also need a nightly Rust toolchain:

```bash
rustup default nightly
```

## Running Fuzzers

From this directory, run one of the fuzz targets:

```bash
# Fuzz canonicalize_path()
cargo +nightly fuzz run canonicalize_path

# Fuzz canonicalize_path_routing()
cargo +nightly fuzz run canonicalize_path_routing
```

Each fuzzer accepts arbitrary byte slices and attempts to find inputs that:
- Trigger panics or assertion failures
- Cause the function to reject dangerous URL patterns
- Reveal edge cases in percent-encoding, dot-segment resolution, or null-byte handling

## Corpus

Fuzzed inputs are automatically saved in `corpus/` for each target. You can manually add seed inputs there to accelerate coverage.

## Seed Corpus (Optional)

You can seed the fuzzer with known-evil URLs to improve coverage:

```bash
# Example seeds for canonicalize_path
echo -n '%2e%2e/' > seeds/%2e%2e
echo -n '%00' > seeds/%00
echo -n '///' > seeds/triple-slash
echo -n '%252e%252e' > seeds/double-encoded
```

Then run:

```bash
cargo +nightly fuzz run canonicalize_path -- -max_len=1024
```

## Debugging Crashes

When a fuzzer finds a crash, it's saved in `crashes/`. To reproduce it:

```bash
cargo +nightly fuzz debug canonicalize_path
```

Or manually run with `cargo run`:

```bash
cargo run --release --bin canonicalize_path -- <crash_input_file>
```

## Notes

- Inputs are expected to be UTF-8 strings; non-UTF-8 bytes are silently rejected.
- The fuzzer relies on libFuzzer's coverage-guided exploration; crashes and hangs are the primary failure modes.
- For end-to-end testing, consider adding integration fuzz targets that feed canonicalized paths into downstream consumers (e.g., file resolution).