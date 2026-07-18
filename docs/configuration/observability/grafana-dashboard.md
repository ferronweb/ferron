---
title: "Grafana reference dashboard"
description: "A dynamic, modular Grafana dashboard for Ferron 3 covering the RED and USE methods across edge cache, retry budgets, and connection pools."
---

Ferron ships a reference Grafana dashboard ([`dashboards/ferron-3-reference.json` in the repository](https://github.com/ferronweb/ferron/blob/3.x/dashboards/ferron-3-reference.json)) that adapts to different operational roles from a single layout. Instead of maintaining separate dashboards for a CDN edge, an API gateway, and a shared-hosting box, the reference template uses template variables and collapsible rows so one structure serves all three.

> [!note]
> The dashboard targets a Prometheus-compatible datasource (such as [Prometheus metrics](/docs/v3/configuration/observability/prometheus) exported by Ferron or scraped through Mimir/Cortex). It expects Ferron's OpenTelemetry-style metric names after conversion to Prometheus snake_case (counters gain a `_total` suffix).

## Design: symptoms on top, causes on the bottom

The dashboard is split along the classic RED vs. USE boundary so an engineer's eye moves from symptom to root cause during an incident:

| Dashboard section | Methodology | Reports | Best used for |
| --- | --- | --- | --- |
| Top half (always expanded) | **RED** (Rate, Errors, Duration) | User-facing symptoms: p99 latency spikes, exploding 5xx rates | High-level triage and SLA verification |
| Bottom half (collapsible rows) | **USE** (Utilization, Saturation, Errors) | Internal proxy causes: pool exhaustion, drained retry budget, circuit trips | Root-cause analysis after an alert fires |

## Template variables

Top-level variables let a single dashboard adapt to any deployment without editing panels:

| Variable | Selects | Source label |
| --- | --- | --- |
| `Job` | Scrape job (default `ferron`) | `job` |
| `Instance` | Scrape target | `instance` |
| `Upstream backend` | Reverse-proxy backend | `ferron_proxy_backend_url` |
| `Host (tenant)` | SNI / tenant (TLS metrics only) | `ferron.host` |
| `Cache zone` | Named cache zone | `ferron.cache.zone` |
| `Rate-limit zone` | Named rate-limit zone | `ferron.ratelimit.zone` |

> [!warning]
> Ferron does **not** expose a per-route (request-path) metric label. The sketch variable `$route` from generic dashboard designs has no matching signal. Filter by upstream backend or by cache/rate-limit zone instead. If you need per-route granularity, promote a bounded route attribute with [baggage promotion](/docs/v3/configuration/observability/prometheus#baggage-promotion) and add it as a variable — but cap `max_distinct` to avoid label explosion.

## Rows

### Row 1 — Core L7 Matrix (RED, always expanded)

The row every deployment reads first:

- **Traffic rate by status class** — `sum by (http_response_status_code) (rate(ferron_http_server_request_count_total{...}))`
- **Active requests** — `sum(http_server_active_requests)`
- **5xx / 4xx error ratio** — error-class request rate divided by total request rate
- **Request latency (p50/p95/p99)** — `histogram_quantile` over `http_server_request_duration_seconds_bucket`. With [native exponential histograms](/docs/v3/configuration/observability/metrics#exponential-histograms) enabled, tail latencies (p99/p99.9) stay sharp without manual bucket tuning.

### Row 2 — Resiliency primitives (collapsible)

Maps the token-bucket retry budget and rate limiting:

- **Retry budget exhausted /s** — `rate(ferron_proxy_retry_budget_exhausted_total)`
- **Retry budget tokens available** — `ferron_proxy_retry_budget_tokens_available`
- **Retry final failures** — `ferron_proxy_retry_final == 0`
- **Rate-limit rejected /s (by zone)** — `rate(ferron_ratelimit_rejected_total)`
- **Rate-limit allowed vs throttled /s** — `rate(ferron_ratelimit_allowed_total)` / `rate(ferron_ratelimit_throttled_total)`

An API gateway engineer keeps this row open to watch JWT-adjacent throttling and endpoint rejection spikes; a CDN operator can leave it collapsed.

### Row 3 — Infrastructure saturation (USE, collapsible)

Connection-pool and host-pressure panels, crucial for multi-tenant edges and service meshes:

- **Pool saturation** — `ferron_proxy_pool_outstanding` against `ferron_proxy_pool_local_limit` and `ferron_proxy_pool_global_limit`
- **Pool wait time p95** — `histogram_quantile` over `ferron_proxy_pool_wait_time_seconds_bucket`
- **Pool waits /s** — `rate(ferron_proxy_pool_waits_total)` (exhaustion events)
- **Pool hit ratio** — hits divided by hits + misses
- **Top backends by outstanding connections** — a table answering "who is the noisy neighbor" directly
- **Circuit breaker state & unhealthy backends** — `ferron_proxy_circuit_state`, `rate(ferron_proxy_backends_unhealthy_total)`, `ferron_proxy_lb_active_connections`
- **Host saturation** — `process_cpu_utilization_ratio`, `process_unix_file_descriptor_count`, `process_memory_usage_bytes`

### Row 4 — Edge cache (collapsible)

- **Cache hit ratio** — `rate(ferron_cache_requests_total{...,result="hit"})` / total
- **Cache entries** — `ferron_cache_entries` by zone
- **Cache evictions /s (by reason)** — `rate(ferron_cache_evictions_total)` split by `ferron.cache.reason`
- **Egress bandwidth** — `rate(ferron_static_bytes_sent_sum)` (static-file and PHP-accelerator egress; see the gap below)
- **DNS cache TTL remaining** — `ferron_proxy_dns_cache_ttl_remaining_seconds` (min/avg/max via the `aggregation` label) and DNS hit ratio

A CDN or PHP-accelerator operator keeps this row pinned; an API gateway user can ignore it.

## Deploying the dashboard

### Import the JSON

1. In Grafana, **Dashboards → New → Import**, then upload `dashboards/ferron-3-reference.json`.
2. Pick your Prometheus/Mimir datasource.

### Or provision it

Mount the JSON under a Grafana [file provisioner](https://grafana.com/docs/grafana/latest/administration/provisioning/#dashboards) `dashboards` path, or POST it to the HTTP API:

```bash
curl -u admin:admin -X POST \
  http://localhost:3000/api/dashboards/db \
  -H "Content-Type: application/json" \
  -d @dashboards/ferron-3-reference.json
```

### Required Ferron configuration

The dashboard reads metrics Ferron only emits when observability is configured:

```ferron
example.com {
    observability {
        provider prometheus
        endpoint_listen "127.0.0.1:8889"
    }
}
```

Enable the features whose rows you care about:

- **Row 2 (retry budget, rate limiting)** — configure [`rate_limit`](/docs/v3/configuration/content/rate-limit) and a proxy with [`retry`](/docs/v3/configuration/proxy/reverse-proxy) settings.
- **Row 3 (pools, circuit)** — configure a [`proxy`](/docs/v3/configuration/proxy/reverse-proxy) with upstreams.
- **Row 4 (cache, DNS cache)** — configure [`cache`](/docs/v3/configuration/content/cache) and strict/SRV [`dns_servers`](/docs/v3/configuration/proxy/reverse-proxy).

## Known gaps

- **Per-route filtering** is unavailable without baggage promotion, as noted above.
- **Egress bandwidth** exists only as `ferron.static.bytes_sent` (static files, PHP accelerator). Reverse-proxy upstream egress has no bytes metric today, so proxy-dominated CDN egress must be measured at the load balancer or added to Ferron later.
- **TLS / `ferron.host` metrics** (`ferron_tls_*`) appear only when HTTPS is configured; those panels render empty on HTTP-only instances.
- Rows whose Ferron modules are disabled show empty panels by design. The dashboard degrades gracefully rather than erroring.

## See also

- [Configuration: metrics](/docs/v3/configuration/observability/metrics) — full metric catalog and alerting patterns
- [Prometheus metrics](/docs/v3/configuration/observability/prometheus) — exporter setup and native histograms
- [OTLP observability](/docs/v3/configuration/observability/otlp) — OpenTelemetry export alternative
