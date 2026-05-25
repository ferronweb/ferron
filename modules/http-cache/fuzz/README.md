Fuzz targets for ferron-http-cache

## Why Fuzzing Makes Sense Here

The cache module processes **untrusted HTTP headers** (both request and response) and makes **critical correctness decisions** (cache hits/misses, purges, TTLs) based on complex parsing logic.

## Targets

| Target | What it fuzzes |
|--------|----------------|
| `lscache_parsers` | All four LSCache header parsing functions (`X-Litespeed-Cache-Control`, `X-Litespeed-Vary`, `X-Litespeed-Tag`, `X-Litespeed-Purge`) |
| `policy_evaluation` | Request and response cache policy logic (`parse_request_policy`, `evaluate_response_policy`) with full header combinations |
| `cache_key_roundtrip` | Cache key generation (`build_entry_key`) via insert+lookup round-trips on `CacheStore` |

## Running

From this directory:

```bash
cargo +nightly fuzz run lscache_parsers
cargo +nightly fuzz run policy_evaluation
cargo +nightly fuzz run cache_key_roundtrip
```

With a timeout per input:

```bash
cargo +nightly fuzz run lscache_parsers -- -max_len=4096 -timeout=5
```

## Corpus

Manually seed interesting inputs:

```bash
mkdir -p corpus/lscache_parsers
# Empty tokens between delimiters
printf 'X-Litespeed-Purge: url=/path,,tag=foo\n' > corpus/lscache_parsers/double-comma
printf 'X-Litespeed-Purge: public,private\n' > corpus/lscache_parsers/mixed-scope
```

## Notes

- Uses `arbitrary` for structured fuzzing where multiple input dimensions are needed.
- The parsers are already defensive (`to_str().ok()` for UTF-8, `parse::<u64>()` for integers) but edge cases in delimiter handling, token matching, and logic interactions are hard to test exhaustively with unit tests.
- All three targets use coverage-guided fuzzing to explore branches.
