---
title: "Grafana reference dashboard"
description: "Reference Grafana dashboard for Ferron 3: RED and USE panels, edge cache, retry budgets, and connection pools."
---

Ferron ships a reference Grafana dashboard: [`dashboards/ferron-3-reference.json`](https://github.com/ferronweb/ferron/blob/3.x/dashboards/ferron-3-reference.json). One layout covers a CDN edge, an API gateway, and a shared-hosting box through template variables and collapsible rows.

> [!note]
> The dashboard expects a Prometheus-compatible datasource, such as [Prometheus metrics](/docs/configuration/observability/prometheus) exported by Ferron or scraped through Mimir or Cortex. It expects OpenTelemetry-style metric names converted to Prometheus snake_case. Counters gain a `_total` suffix.

## Layout

The top half shows user-facing symptoms. The bottom half shows internal causes.

| Section                 | Method                                    | Reports                                                              | Use for                       |
| ----------------------- | ----------------------------------------- | -------------------------------------------------------------------- | ----------------------------- |
| Top half (expanded)     | RED: rate, errors, duration               | p99 latency spikes, 5xx rate                                         | Triage and SLA verification   |
| Bottom half (collapsed) | USE: utilization, saturation, errors      | Pool exhaustion, drained retry budget, circuit trips                 | Root cause after an alert     |

## Template variables

| Variable           | Selects                         | Source label               |
| ------------------ | ------------------------------- | -------------------------- |
| `Job`              | Scrape job. Default `ferron`.   | `job`                      |
| `Instance`         | Scrape target                   | `instance`                 |
| `Upstream backend` | Reverse-proxy backend           | `ferron_proxy_backend_url` |
| `Host (tenant)`    | SNI or tenant. TLS metrics only. | `ferron.host`             |
| `Cache zone`       | Named cache zone                | `ferron.cache.zone`        |
| `Rate-limit zone`  | Named rate-limit zone           | `ferron.ratelimit.zone`    |

Latency panels query native and classic histograms side by side, for example `p95` and `p95 classic`. Only the matching series carries data. There is no histogram-mode switch.

> [!warning]
> Ferron does not expose a per-route (request-path) metric label. The `$route` variable from generic dashboard designs has no matching signal. Filter by upstream backend or by cache or rate-limit zone. For per-route granularity, promote a bounded route attribute with [baggage promotion](/docs/configuration/observability/prometheus#baggage-promotion). Add it as a variable, but cap `max_distinct` to avoid label explosion.

## Rows

### Core L7 matrix (RED)

- **Request rate by status class**: `sum by (http_response_status_code) (rate(ferron_http_server_request_count_total{...}))`
- **Active requests**: `sum(http_server_active_requests)`
- **5xx and 4xx error ratio**: error-class request rate divided by total request rate
- **Request latency (p50/p95/p99)**: `histogram_quantile` over `http_server_request_duration_seconds`, queried for [native exponential histograms](/docs/configuration/observability/metrics#exponential-histograms) and for classic `_bucket` series

### Resiliency primitives

- **Retry budget exhausted /s**: `rate(ferron_proxy_retry_budget_exhausted_total)`
- **Retry budget tokens available**: `ferron_proxy_retry_budget_tokens_available`
- **Retry final failures**: `ferron_proxy_retry_final == 0`
- **Rate-limit rejected /s by zone**: `rate(ferron_ratelimit_rejected_total)`
- **Rate-limit allowed and throttled /s**: `rate(ferron_ratelimit_allowed_total)` and `rate(ferron_ratelimit_throttled_total)`

### Infrastructure saturation

- **Pool saturation**: `ferron_proxy_pool_outstanding` against `ferron_proxy_pool_local_limit` and `ferron_proxy_pool_global_limit`
- **Pool wait time p95**: `histogram_quantile` over `ferron_proxy_pool_wait_time_seconds`, native and classic queries side by side
- **Pool waits /s**: `rate(ferron_proxy_pool_waits_total)`
- **Pool hit ratio**: hits divided by hits plus misses
- **Top backends by outstanding connections**: table of backends with the most outstanding connections
- **Circuit breaker state and unhealthy backends**: `ferron_proxy_circuit_state`, `rate(ferron_proxy_backends_unhealthy_total)`, `ferron_proxy_lb_active_connections`
- **Host saturation**: `process_cpu_utilization_ratio`, `process_unix_file_descriptor_count`, `process_memory_usage_bytes`

### Edge cache

- **Cache hit ratio**: `rate(ferron_cache_requests_total{...,result="hit"})` divided by the total
- **Cache entries**: `ferron_cache_entries` by zone
- **Cache evictions /s by reason**: `rate(ferron_cache_evictions_total)` split by `ferron.cache.reason`
- **Cache request outcomes /s by reason**: `rate(ferron_cache_requests_total)` split by zone, result, and `ferron.cache.reason`. The result explains a low hit ratio (`response-no-store`, `private-no-identity`, `zero-ttl`, bypasses).
- **DNS cache TTL remaining**: `ferron_proxy_dns_cache_ttl_remaining_seconds` with min, average, and max from the `aggregation` label. Also shows the DNS hit ratio.

### Edge policy (4xx attribution)

Separates intentional edge-policy decisions from unexpected application errors:

- **Policy decisions /s**: `rate(ferron_response_status_rule_matched_total)` by `ferron.rule_id`, `rate(ferron_abuseban_rejected_total)` by `ferron.abuseban.reason`, `rate(ferron_ratelimit_rejected_total)` by zone, and `rate(ferron_response_ip_blocked_total)`

When the 4xx ratio spikes, read this row first. If the spike is fully explained here, scanners and policy enforcement caused it, not a backend failure.

## Deploy the dashboard

### Import the JSON

1. In Grafana, select Dashboards > New > Import, then upload `dashboards/ferron-3-reference.json`.
2. Select your Prometheus or Mimir datasource.

### Provision the dashboard

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

Enable the features for the rows you use:

| Row                                    | Required configuration                                                                                                                    |
| -------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| Retry budget and rate limiting         | [`rate_limit`](/docs/configuration/content/rate-limit) and a proxy with [`retry`](/docs/configuration/proxy/reverse-proxy) settings.      |
| Pools and circuit breaker              | A [`proxy`](/docs/configuration/proxy/reverse-proxy) with upstreams.                                                                      |
| Cache and DNS cache                    | [`cache`](/docs/configuration/content/cache) and strict or SRV [`dns_servers`](/docs/configuration/proxy/reverse-proxy).                  |
| Policy attribution                     | [`status`](/docs/configuration/routing/response) rules, [`abuse_protection`](/docs/configuration/content/abuse-ban), or [`rate_limit`](/docs/configuration/content/rate-limit). |

Set `name` on status rules to get readable `ferron.rule_id` values.

## Known gaps

- Per-route filtering requires baggage promotion.
- TLS metrics (`ferron_tls_*`) appear only when you configure HTTPS. The panels render empty on HTTP-only instances.
- Rows for inactive modules show empty panels. The dashboard does not error.

## See also

- [Configuration: metrics](/docs/configuration/observability/metrics): full metric catalog and alerting patterns
- [Prometheus metrics](/docs/configuration/observability/prometheus): exporter setup and native histograms
- [OTLP observability](/docs/configuration/observability/otlp): OpenTelemetry export alternative
