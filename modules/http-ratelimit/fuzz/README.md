Fuzz targets for http-ratelimit

Run (from this directory):

cargo +nightly fuzz run rate_limit_concurrent

Notes:
- The harness uses deterministic zero refill rate to detect concurrency duplication bugs where multiple buckets for the same key allow more successful consumes than capacity.
- Seed corpus can include heavy concurrency patterns (many threads, same key) and simple single-key drains.
