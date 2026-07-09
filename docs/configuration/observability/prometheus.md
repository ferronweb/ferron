---
title: "Configuration: Prometheus metrics"
description: "Prometheus metrics export configuration for monitoring Ferron server performance and health."
---

This page documents the Prometheus metrics export configuration for Ferron. The `observability-prometheus` module exports Ferron's internal metrics in Prometheus format, enabling integration with Prometheus servers, Grafana dashboards, and other monitoring systems.

## Directives

Prometheus metrics are configured via `observability` blocks with `provider prometheus`:

```ferron
observability {
    provider prometheus
    endpoint_listen "127.0.0.1:8889"
    endpoint_format text
    endpoint_auth_token "my-scrape-token"
}
```

### Configuration directives

| Directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `provider` | `"prometheus"` | Specifies the Prometheus observability provider. Required. | none |
| `endpoint_listen` | `<socket_address>` | Socket address to listen on for Prometheus metrics requests. Supports IPv4, IPv6, and port specifications. | `"127.0.0.1:8889"` |
| `endpoint_format` | `<format>` | Output format for metrics. Supported values: `"text"` (Prometheus text format), `"protobuf"` (Prometheus protobuf format). | `"text"` |
| `endpoint_native_histograms` | `<bool>` | Enable native exponential histograms in protobuf format. When enabled, histogram metrics include both classic buckets and native histogram data in protobuf output. Text format always shows classic buckets regardless of this setting. | `false` |
| `endpoint_auth_token` | `<token>` | Bearer token for authenticating Prometheus scrape requests. When set, scrapers must send `Authorization: Bearer <token>` header. | none (authentication disabled) |

### Socket address format

The `endpoint_listen` directive accepts standard Rust socket address syntax:

- IPv4: `"127.0.0.1:8889"`, `"0.0.0.0:8889"`
- IPv6: `"[::1]:8889"`, `"[::]:8889"`
- Port-only: `":8889"` (binds to all interfaces)

> [!warning]
> Binding to `0.0.0.0` or `[::]` exposes the metrics endpoint to all network interfaces. For production deployments, consider:
>
> - Binding to localhost only (`127.0.0.1` or `::1`)
> - Using firewall rules to restrict access
> - Placing Ferron behind a reverse proxy with authentication

### Format options

- **`"text"`** — standard Prometheus text exposition format (default)
- **`"protobuf"`** — Prometheus protobuf format for more efficient scraping

### Native histograms

When `endpoint_native_histograms true` is set and `endpoint_format` is `"protobuf"`, histogram metrics include native exponential histogram data alongside classic bucket histograms. Native histograms provide high-resolution percentile data across deep orders of magnitude (1ms to 100s) without requiring manual bucket allocation.

Text format always exposes classic bucket histograms regardless of this setting, since the OpenMetrics text format does not support native histograms.

> [!note]
> Native histograms require Prometheus 2.40+ or compatible clients that support the OpenMetrics native histogram protocol. Older Prometheus versions will silently ignore the native histogram data and use the classic buckets.

### Metric exemplars

When a request has an active trace context (trace ID and span ID), the Prometheus module automatically attaches **exemplars** to counter and histogram observations. Exemplars link a specific metric observation to a trace, enabling drill-down from a metric spike to the specific request that caused it.

Each exemplar contains:

- `trace_id` — the W3C trace ID of the request
- `span_id` — the W3C span ID of the request

Exemplars are enabled by default for all counter metrics. For histograms, exemplars are active when `endpoint_native_histograms` is `false` (the default), since native histograms and exemplars are mutually exclusive in Ferron's Prometheus module.

> [!note]
> Exemplars are displayed in the OpenMetrics text format as comments appended at the end of the metric line and are natively supported in the Prometheus protobuf format. Prometheus 2.26+ and Grafana can display exemplars for trace-to-metrics correlation.

### Baggage promotion

The `baggage` sub-directive promotes specific W3C Baggage keys into Prometheus metric labels. This is useful for adding request-scoped context (such as tenant IDs or user roles) to your metrics without custom instrumentation.

```ferron
observability {
    provider prometheus

    baggage {
        key "tenant.id" {
            attribute "tenant.id"
            max_distinct 100
        }
    }
}
```

Each `key` entry configures one baggage key to promote:

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `key` | `<string>` | The W3C Baggage key to extract. Required. | - |
| `attribute` | `<string>` | The Prometheus label name to use. | same as the baggage key |
| `max_distinct` | `<number> \| false` | Maximum distinct label values before hashing. Prevents high-cardinality label explosion. | 100 |

> [!warning]
> Prometheus metrics with high-cardinality labels can cause significant performance issues and memory consumption. Always set `max_distinct` on baggage keys with unbounded values (such as user IDs or request IDs). Values exceeding the distinct cap are automatically hashed to a deterministic `hash_<hex>` string.

## Metrics endpoint

When configured, the Prometheus module starts an HTTP server that exposes metrics at the `/metrics` endpoint:

```bash
curl http://localhost:8889/metrics
```

Example output (text format):

```text
# HELP http_server_active_requests Number of active HTTP requests
# TYPE http_server_active_requests gauge
http_server_active_requests 5

# HELP http_server_request_duration_seconds Duration of HTTP requests in seconds
# TYPE http_server_request_duration_seconds histogram
http_server_request_duration_seconds_bucket{http_request_method="GET",le="0.005"} 100
http_server_request_duration_seconds_bucket{http_request_method="GET",le="0.01"} 150
http_server_request_duration_seconds_bucket{http_request_method="GET",le="0.025"} 175
http_server_request_duration_seconds_bucket{http_request_method="GET",le="0.05"} 180
http_server_request_duration_seconds_bucket{http_request_method="GET",le="0.1"} 185
http_server_request_duration_seconds_bucket{http_request_method="GET",le="0.25"} 190
http_server_request_duration_seconds_bucket{http_request_method="GET",le="0.5"} 192
http_server_request_duration_seconds_bucket{http_request_method="GET",le="1.0"} 193
http_server_request_duration_seconds_bucket{http_request_method="GET",le="2.5"} 194
http_server_request_duration_seconds_bucket{http_request_method="GET",le="5.0"} 195
http_server_request_duration_seconds_bucket{http_request_method="GET",le="10.0"} 195
http_server_request_duration_seconds_bucket{http_request_method="GET",le="+Inf"} 195
http_server_request_duration_seconds_sum{http_request_method="GET"} 12.345
http_server_request_duration_seconds_count{http_request_method="GET"} 195
```

> [!tip]
>
> - If the metrics endpoint fails to start, check for port conflicts with `netstat -tuln | grep 8889` or similar.
> - Ensure your firewall allows traffic to the metrics port if binding to non-localhost addresses.

### Metric naming

Ferron metrics follow OpenTelemetry semantic conventions and are automatically converted to Prometheus format:

- OpenTelemetry metric names are converted to snake_case
- Attributes become Prometheus labels
- Counter metrics become Prometheus counters
- Gauge metrics become Prometheus gauges
- Histogram metrics become Prometheus histograms

## Configuration examples

### Basic local monitoring

```ferron
# Global configuration
example.com {
    observability {
        provider prometheus
        endpoint_listen "127.0.0.1:8889"
    }
    root /var/www/html
}
```

### Production monitoring with all interfaces

```ferron
# Production setup with all interfaces (use with firewall)
example.com {
    observability {
        provider prometheus
        endpoint_listen "0.0.0.0:8889"
        endpoint_format text
    }
    root /var/www/html
}
```

### IPv6 monitoring

```ferron
# IPv6 monitoring
example.com {
    observability {
        provider prometheus
        endpoint_listen "[::]:8889"
    }
    root /var/www/html
}
```

### Multiple hosts with different endpoints

```ferron
# Different metrics endpoints for different hosts
example.com {
    observability {
        provider prometheus
        endpoint_listen "127.0.0.1:9001"
    }
    root /var/www/example
}

api.example.com {
    observability {
        provider prometheus
        endpoint_listen "127.0.0.1:9002"
    }
    proxy http://backend:3000
}
```

### Baggage promotion with cardinality control

```ferron
example.com {
    observability {
        provider prometheus
        endpoint_listen "127.0.0.1:8889"

        baggage {
            # Promote tenant ID as a metric label (bounded cardinality)
            key "tenant.id" {
                attribute "tenant.id"
                max_distinct 50
            }

            # Promote user role with strict cardinality cap
            key "user.role" {
                attribute "ferron.user_role"
                max_distinct 10
            }
        }
    }
}
```

> [!tip]
> Be aware of high-cardinality labels that could cause performance issues in Prometheus. Use `max_distinct` on `baggage` keys to cap distinct label values and prevent label explosion.

## Prometheus server configuration

Add the following to your `prometheus.yml` to scrape Ferron metrics:

```yaml
scrape_configs:
  - job_name: 'ferron'
    static_configs:
      - targets: ['localhost:8889']
    scrape_interval: 15s
    scrape_timeout: 10s
```

## Best practices

The following best-practice checks are reported by `ferron doctor` for directives on this page.

### Endpoint authentication

- **`endpoint_listen` on non-loopback address without `endpoint_auth_token`** — The metrics endpoint is unauthenticated and exposed to all network interfaces. Bind to a loopback address, use `endpoint_auth_token` to require a bearer token, or restrict access via network controls.

### `max_distinct` high cardinality prevention

- **`max_distinct false` inside Baggage configuration** - high-cardinality attributes should not be set in baggage, as they can lead to excessive memory usage and performance issues.

## See also

- [Observability and logging](/docs/v3/configuration/observability/logging) for general observability configuration
- [OTLP export](/docs/v3/configuration/observability/logging#otlp-export) for OpenTelemetry-based monitoring
