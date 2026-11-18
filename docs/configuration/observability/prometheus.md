---
title: "Configuration: Prometheus metrics"
description: "Export Ferron metrics in Prometheus format over an HTTP endpoint."
---

The `observability-prometheus` module serves Ferron metrics in Prometheus format over HTTP. Prometheus, Grafana, and other OpenMetrics-compatible systems can scrape the endpoint.

## Directives

```ferron
example.com {
    observability {
        provider prometheus
        endpoint_listen 127.0.0.1:8889
        endpoint_format text
        endpoint_auth_token my-scrape-token
    }
}
```

| Directive                    | Arguments          | Description                                                        | Default                        |
| ---------------------------- | ------------------ | ------------------------------------------------------------------ | ------------------------------ |
| `provider`                   | `"prometheus"`     | Selects the Prometheus provider. Required.                         | none                           |
| `endpoint_listen`            | `<socket_address>` | Listen address for scrape requests. Supports IPv4, IPv6, and port-only forms. | `"127.0.0.1:8889"` |
| `endpoint_format`            | `<format>`         | Output format: `"text"` or `"protobuf"`.                           | `"text"`                       |
| `endpoint_native_histograms` | `<bool>`           | Add native exponential histograms to protobuf output.              | `false`                        |
| `endpoint_auth_token`        | `<token>`          | Require `Authorization: Bearer <token>` on scrape requests.        | none (authentication disabled) |

### Socket address format

`endpoint_listen` accepts standard Rust socket address syntax:

- IPv4: `"127.0.0.1:8889"`, `"0.0.0.0:8889"`
- IPv6: `"[::1]:8889"`, `"[::]:8889"`
- Port only: `":8889"`, which binds to all interfaces

> [!warning]
> Binding to `0.0.0.0` or `[::]` exposes the metrics endpoint to all network interfaces. Bind to localhost (`127.0.0.1` or `::1`), restrict access with firewall rules, or place Ferron behind an authenticated reverse proxy.

### Native histograms

Set `endpoint_native_histograms` and `endpoint_format "protobuf"` to include native exponential histogram data alongside classic bucket histograms. Native histograms give high-resolution percentiles from 1ms to 100s without manual bucket allocation.

Text format exposes classic bucket histograms only. The OpenMetrics text format does not support native histograms.

> [!note]
> Native histograms require Prometheus 2.40+ or a compatible client. Older Prometheus versions ignore the native histogram data and use the classic buckets.

### Metric exemplars

When a request has an active trace context, the Prometheus module attaches exemplars to counter and histogram observations. An exemplar links a metric observation to a specific trace, so you can move from a metric spike to the request that caused it.

Each exemplar contains:

- `trace_id`: the W3C trace ID of the request
- `span_id`: the W3C span ID of the request

Counters carry exemplars by default. Histograms carry exemplars when `endpoint_native_histograms` is `false` (the default). Native histograms and exemplars are mutually exclusive in the Ferron Prometheus module.

> [!note]
> In OpenMetrics text format, exemplars appear as comments at the end of the metric line. The protobuf format supports them natively. Prometheus 2.26+ and Grafana can display exemplars for trace-to-metric correlation.

### Baggage promotion

The `baggage` sub-directive promotes W3C Baggage keys into Prometheus metric labels. Use it to add request-scoped context, such as tenant IDs or user roles, without custom instrumentation.

```ferron
example.com {
    observability {
        provider prometheus

        baggage {
            key tenant.id {
                attribute tenant.id
                max_distinct 100
            }
        }
    }
}
```

Each `key` entry configures one baggage key:

| Directive      | Arguments           | Description                                                                               | Default                 |
| -------------- | ------------------- | ----------------------------------------------------------------------------------------- | ----------------------- |
| `key`          | `<string>`          | W3C Baggage key to extract. Required.                                                     | none                    |
| `attribute`    | `<string>`          | Prometheus label name.                                                                    | same as the baggage key |
| `max_distinct` | `<number> \| false` | Maximum distinct label values before dropping. Prevents high-cardinality label explosion.  | 100                     |

> [!warning]
> High-cardinality labels can reduce performance and increase memory use in Prometheus. Always set `max_distinct` on baggage keys with unbounded values, such as user IDs or request IDs. Values above the cap are dropped.

## Metrics endpoint

When configured, the module starts an HTTP server and exposes metrics at `/metrics`:

```bash
curl http://localhost:8889/metrics
```

Example output in text format:

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
> If the metrics endpoint does not start, check for a port conflict with `netstat -tuln | grep 8889`. If you bind to a non-loopback address, allow traffic to the metrics port in the firewall.

### Metric naming

Ferron converts OpenTelemetry metric names and types to Prometheus format:

- OpenTelemetry metric names become snake_case.
- Attributes become Prometheus labels.
- Counters, gauges, and histograms keep their type.

## Configuration examples

### Basic local monitoring

```ferron
example.com {
    observability {
        provider prometheus
        endpoint_listen 127.0.0.1:8889
    }
}
```

### Multiple hosts with different endpoints

```ferron
example.com {
    observability {
        provider prometheus
        endpoint_listen 127.0.0.1:9001
    }
}

api.example.com {
    observability {
        provider prometheus
        endpoint_listen 127.0.0.1:9002
    }
}
```

## Prometheus server configuration

Add this to `prometheus.yml` to scrape Ferron metrics:

```yaml
scrape_configs:
  - job_name: "ferron"
    static_configs:
      - targets: ["localhost:8889"]
    scrape_interval: 15s
    scrape_timeout: 10s
```

## Best practices

`ferron doctor` reports these checks for directives on this page:

### Endpoint authentication

- **`endpoint_listen` on a non-loopback address without `endpoint_auth_token`**: the metrics endpoint allows unauthenticated access from all network interfaces. Bind to a loopback address, set `endpoint_auth_token` to require a bearer token, or restrict access with network controls.

### High cardinality

- **`max_distinct false` in baggage**: do not set high-cardinality attributes in baggage. They can increase memory use and reduce performance.

## See also

- [Observability and logging](/docs/configuration/observability/logging): general observability configuration
- [OTLP export](/docs/configuration/observability/otlp): OpenTelemetry-based monitoring
