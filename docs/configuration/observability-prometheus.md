---
title: "Configuration: Prometheus metrics"
description: "Prometheus metrics export configuration for monitoring Ferron server performance and health."
---

This page documents the Prometheus metrics export configuration for Ferron. The `observability-prometheus` module exports Ferron's internal metrics in Prometheus format, enabling integration with Prometheus servers, Grafana dashboards, and other monitoring systems.

## Directives

Prometheus metrics are configured via `observability` blocks with `provider "prometheus"`:

```ferron
example.com {
    observability {
        provider "prometheus"
        endpoint_listen "127.0.0.1:8889"
        endpoint_format "text"
    }
}
```

### Configuration directives

| Directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `provider` | `"prometheus"` | Specifies the Prometheus observability provider. Required. | none |
| `endpoint_listen` | `<socket_address>` | Socket address to listen on for Prometheus metrics requests. Supports IPv4, IPv6, and port specifications. | `"127.0.0.1:8889"` |
| `endpoint_format` | `<format>` | Output format for metrics. Supported values: `"text"` (Prometheus text format), `"protobuf"` (Prometheus protobuf format). | `"text"` |

### Socket address format

The `endpoint_listen` directive accepts standard Rust socket address syntax:

- IPv4: `"127.0.0.1:8889"`, `"0.0.0.0:8889"`
- IPv6: `"[::1]:8889"`, `"[::]:8889"`
- Port-only: `":8889"` (binds to all interfaces)

**Security note:** Binding to `0.0.0.0` or `[::]` exposes the metrics endpoint to all network interfaces. For production deployments, consider:

- Binding to localhost only (`127.0.0.1` or `::1`)
- Using firewall rules to restrict access
- Placing Ferron behind a reverse proxy with authentication

### Format options

- **`"text"`** — standard Prometheus text exposition format (default)
- **`"protobuf"`** — Prometheus protobuf format for more efficient scraping

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
        provider "prometheus"
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
        provider "prometheus"
        endpoint_listen "0.0.0.0:8889"
        endpoint_format "text"
    }
    root /var/www/html
}
```

### IPv6 monitoring

```ferron
# IPv6 monitoring
example.com {
    observability {
        provider "prometheus"
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
        provider "prometheus"
        endpoint_listen "127.0.0.1:9001"
    }
    root /var/www/example
}

api.example.com {
    observability {
        provider "prometheus"
        endpoint_listen "127.0.0.1:9002"
    }
    proxy http://backend:3000
}
```

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

## Notes and troubleshooting

- **Endpoint availability** - the Prometheus endpoint is started lazily when the first metric event is received for a given configuration. This means there may be a slight delay for the first request.
- **Metric cardinality** - be aware of high-cardinality labels that could cause performance issues in Prometheus. Ferron limits label values to reasonable cardinality.
- **Port conflicts** - if the metrics endpoint fails to start, check for port conflicts with `netstat -tuln | grep 8889` or similar.
- **Firewall rules** - ensure your firewall allows traffic to the metrics port if binding to non-localhost addresses.
- **Authentication** - the Prometheus endpoint does not currently support authentication. For secure deployments, place Ferron behind a reverse proxy with authentication or use network-level access controls.
- **Multiple configurations** - each unique combination of `endpoint_listen` and `endpoint_format` creates a separate metrics endpoint. This allows different hosts to export metrics to different ports or formats.

## See also

- [Observability and logging](/docs/v3/configuration/observability-logging) for general observability configuration
- [OTLP export](/docs/v3/configuration/observability-logging#otlp-export) for OpenTelemetry-based monitoring
