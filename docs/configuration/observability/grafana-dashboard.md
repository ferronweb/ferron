---
title: "Grafana reference dashboard"
description: "Modular Grafana dashboard for Ferron 3 covering RED and USE methods, edge cache, retry budgets, and connection pools."
---

Ferron ships a reference Grafana dashboard ([`dashboards/ferron-3-reference.json` in the repository](https://github.com/ferronweb/ferron/blob/3.x/dashboards/ferron-3-reference.json)). It adapts to different operational roles from a single layout. Maintaining separate dashboards for a CDN edge, an API gateway, and a shared-hosting box is a lot of work. The reference template uses template variables and collapsible rows. One structure serves all three.

> [!note]
> The dashboard targets a Prometheus-compatible datasource (such as [Prometheus metrics](/docs/configuration/observability/prometheus) exported by Ferron or scraped through Mimir/Cortex). It expects OpenTelemetry-style metric names from Ferron converted to Prometheus snake_case. Counters gain a `_total` suffix.

## Design: symptoms on top, causes on the bottom

The dashboard splits along the classic RED versus USE boundary. This layout lets a user move from symptom to root cause during an incident:

| Dashboard section              | Methodology                               | Reports                                                                     | Best used for                            |
| ------------------------------ | ----------------------------------------- | --------------------------------------------------------------------------- | ---------------------------------------- |
| Top half (always expanded)     | **RED** (Rate, Errors, Duration)          | User-facing symptoms: p99 latency spikes, exploding 5xx rates               | High-level triage and SLA verification   |
| Bottom half (collapsible rows) | **USE** (Utilization, Saturation, Errors) | Internal proxy causes: pool exhaustion, drained retry budget, circuit trips | Root-cause analysis after an alert fires |

## Template variables

Top-level variables let a single dashboard adapt to any deployment without editing panels:

| Variable           | Selects                         | Source label               |
| ------------------ | ------------------------------- | -------------------------- |
| `Job`              | Scrape job (default `ferron`)   | `job`                      |
| `Instance`         | Scrape target                   | `instance`                 |
| `Upstream backend` | Reverse-proxy backend           | `ferron_proxy_backend_url` |
| `Host (tenant)`    | SNI / tenant (TLS metrics only) | `ferron.host`              |
| `Cache zone`       | Named cache zone                | `ferron.cache.zone`        |
| `Rate-limit zone`  | Named rate-limit zone           | `ferron.ratelimit.zone`    |

Latency and egress panels query both native and classic histograms side by side (for example `p95` and `p95 classic`), so panels render regardless of the exporter's histogram mode. Only the matching series carries data; the other stays empty. There is no histogram-mode switch to set.

> [!warning]
> Ferron does not expose a per-route (request-path) metric label. The sketch `$route` variable from generic dashboard designs has no matching signal. Filter by upstream backend or by cache/rate-limit zone instead. If you need per-route granularity, promote a bounded route attribute with [baggage promotion](/docs/configuration/observability/prometheus#baggage-promotion). Add it as a variable, but cap `max_distinct` to avoid label explosion.

## Rows

### Row 1: core L7 matrix (RED, always expanded)

The row every deployment reads first:

- **Traffic rate by status class**: `sum by (http_response_status_code) (rate(ferron_http_server_request_count_total{...}))`
- **Active requests**: `sum(http_server_active_requests)`
- **5xx / 4xx error ratio**: error-class request rate divided by total request rate
- **Request latency (p50/p95/p99)**: `histogram_quantile` over `http_server_request_duration_seconds`, queried twice — once for [native exponential histograms](/docs/configuration/observability/metrics#exponential-histograms) and once for classic `_bucket` series — so tail latencies stay sharp without manual bucket tuning on either export mode.

### Row 2: resiliency primitives (collapsible)

Maps the token-bucket retry budget and rate limiting:

- **Retry budget exhausted /s**: `rate(ferron_proxy_retry_budget_exhausted_total)`
- **Retry budget tokens available**: `ferron_proxy_retry_budget_tokens_available`
- **Retry final failures**: `ferron_proxy_retry_final == 0`
- **Rate-limit rejected /s (by zone)**: `rate(ferron_ratelimit_rejected_total)`
- **Rate-limit allowed vs throttled /s**: `rate(ferron_ratelimit_allowed_total)` / `rate(ferron_ratelimit_throttled_total)`

An API gateway engineer keeps this row open to watch JWT-adjacent throttling and endpoint rejection spikes. A CDN operator can leave it collapsed.

### Row 3: infrastructure saturation (USE, collapsible)

Connection-pool and host-pressure panels, crucial for multi-tenant edges and service meshes:

- **Pool saturation**: `ferron_proxy_pool_outstanding` against `ferron_proxy_pool_local_limit` and `ferron_proxy_pool_global_limit`
- **Pool wait time p95**: `histogram_quantile` over `ferron_proxy_pool_wait_time_seconds`, native and classic queries side by side
- **Pool waits /s**: `rate(ferron_proxy_pool_waits_total)` (exhaustion events)
- **Pool hit ratio**: hits divided by hits + misses
- **Top backends by outstanding connections**: a table answering "who is the noisy neighbor" directly
- **Circuit breaker state & unhealthy backends**: `ferron_proxy_circuit_state`, `rate(ferron_proxy_backends_unhealthy_total)`, `ferron_proxy_lb_active_connections`
- **Host saturation**: `process_cpu_utilization_ratio`, `process_unix_file_descriptor_count`, `process_memory_usage_bytes`

### Row 4: edge cache (collapsible)

- **Cache hit ratio**: `rate(ferron_cache_requests_total{...,result="hit"})` / total
- **Cache entries**: `ferron_cache_entries` by zone
- **Cache evictions /s (by reason)**: `rate(ferron_cache_evictions_total)` split by `ferron.cache.reason`
- **Cache request outcomes /s (by reason)**: `rate(ferron_cache_requests_total)` split by zone, result, and `ferron.cache.reason` — answers why the hit ratio is low (`response-no-store`, `private-no-identity`, `zero-ttl`, bypasses)
- **Egress bandwidth: static file bytes/s**: native `histogram_sum` and classic `_sum` queries side by side (static-file and PHP-accelerator egress. See the gap below)
- **DNS cache TTL remaining**: `ferron_proxy_dns_cache_ttl_remaining_seconds` (min/avg/max via the `aggregation` label) and DNS hit ratio

A CDN or PHP-accelerator operator keeps this row pinned. An API gateway user can ignore it.

### Row 5: edge policy (4xx attribution, collapsible)

Splits 4xx traffic into intentional edge-policy decisions versus unexpected application errors:

- **Policy decisions /s**: `rate(ferron_response_status_rule_matched_total)` by `ferron.rule_id`, `rate(ferron_abuseban_rejected_total)` by `ferron.abuseban.reason`, `rate(ferron_ratelimit_rejected_total)` by zone, and `rate(ferron_response_ip_blocked_total)`

When the 4xx ratio spikes, check this row first: if the spike is fully explained here, it is scanners and policy enforcement, not a backend failure.

## Deploying the dashboard

### Import the JSON

1. In Grafana, Dashboards → New → Import, then upload `dashboards/ferron-3-reference.json`.
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

The dashboard reads metrics that Ferron emits only when you configure observability:

```ferron
example.com {
    observability {
        provider prometheus
        endpoint_listen "127.0.0.1:8889"
    }
}
```

Enable the features whose rows you care about:

- **Row 2 (retry budget, rate limiting)**: configure [`rate_limit`](/docs/configuration/content/rate-limit) and a proxy with [`retry`](/docs/configuration/proxy/reverse-proxy) settings.
- **Row 3 (pools, circuit)**: configure a [`proxy`](/docs/configuration/proxy/reverse-proxy) with upstreams.
- **Row 4 (cache, DNS cache)**: configure [`cache`](/docs/configuration/content/cache) and strict/SRV [`dns_servers`](/docs/configuration/proxy/reverse-proxy).
- **Row 5 (policy attribution)**: configure [`status`](/docs/configuration/routing/response) rules, [`abuse_protection`](/docs/configuration/content/abuse-ban), or [`rate_limit`](/docs/configuration/content/rate-limit). Readable `ferron.rule_id` values require giving `status` rules `name` values.

## Known gaps

- **Per-route filtering** is unavailable without baggage promotion.
- **Egress bandwidth** exists only as `ferron.static.bytes_sent` (static files, PHP accelerator). Reverse-proxy upstream egress has no bytes metric today. You must measure proxy-dominated CDN egress at the load balancer or add it to Ferron later.
- **TLS / `ferron.host` metrics** (`ferron_tls_*`) appear only when you configure HTTPS. Those panels render empty on HTTP-only instances.
- Rows whose Ferron modules are not active show empty panels by design. The dashboard degrades gracefully rather than erroring.

## See also

- [Configuration: metrics](/docs/configuration/observability/metrics): full metric catalog and alerting patterns
- [Prometheus metrics](/docs/configuration/observability/prometheus): exporter setup and native histograms
- [OTLP observability](/docs/configuration/observability/otlp): OpenTelemetry export alternative
