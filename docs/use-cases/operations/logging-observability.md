---
title: Logging & observability
description: "Practical Ferron setups for access logs, JSON formatting, container-friendly outputs, and OTLP export for centralized observability."
---

Ferron supports multiple observability outputs, so you can start with local log files and later move to centralized telemetry without changing your application stack.

This page focuses on common deployment patterns. For directive-level details, see [Configuration: observability and logging](/docs/v3/configuration/observability/logging).

For specific backend configurations:

- [Prometheus metrics](/docs/v3/configuration/observability/prometheus)
- [OTLP observability](/docs/v3/configuration/observability/otlp)

> [!tip]
> Start simple: text or JSON logs first, then add Prometheus metrics, then OTLP for full observability. All three signals (logs, metrics, traces) from the same HTTP request share the same `trace_id`, enabling correlated queries.

## Basic production logs to files

Use this when running Ferron directly on a VM or bare metal and collecting logs from disk:

```ferron
example.com {
    log "access.log"

    root /var/www/html
}
```

The text formatter uses the Combined Log Format (CLF) by default, the same format used by Apache and Nginx.

## JSON-format access logs

Use this when you need structured logs for easier parsing by log aggregation tools (for example, ELK Stack, Splunk, or cloud-native log processors):

```ferron
example.com {
    log "access.log" {
        format json
    }

    root /var/www/html
}
```

Example output:

```json
{"method":"GET","path":"/index.html","status":200,"duration_secs":0.012,"client_ip":"127.0.0.1","remote_ip":"127.0.0.1"}
```

You can also select specific fields:

```ferron
example.com {
    log "access.log" {
        format json
        fields "method" "path" "status" "duration_secs" "client_ip"
    }

    root /var/www/html
}
```

## Custom text log patterns

You can customize the text log format using the `access_pattern` directive:

```ferron
example.com {
    log "access.log" {
        format text
        access_pattern "%client_ip - %auth_user [%{%d/%b/%Y:%H:%M:%S %z}t] \"%method %path_and_query %version\" %status %content_length \"%{Referer}i\" \"%{User-Agent}i\""
    }

    root /var/www/html
}
```

## Log rotation

To prevent log files from growing too large, you can configure Ferron to rotate them automatically:

```ferron
example.com {
    log "access.log" {
        access_log_rotate_size 10485760
        access_log_rotate_keep 7
    }

    error_log "error.log" {
        error_log_rotate_size 10485760
        error_log_rotate_keep 7
    }
}
```

## Trace ID logging for debugging

Use this when you need to correlate log messages across requests without setting up a full OTLP backend. The `force_trace` directive enables trace context for every request, and console/file loggers automatically prefix messages with `[trace=<span_id>]`:

```ferron
{
    http {
        force_trace true
    }
}

example.com {
    # HTTP access logs
    log "access.log" {
        format text
        access_pattern ">>> %trace_id <<< %client_ip - %auth_user [%t] \"%method %path_and_query %version\" %status %content_length \"%{Referer}i\" \"%{User-Agent}i\""
    }

    # "Error" logs, also with trace IDs
    error_log "error.log"

    root /var/www/html
}
```

Example log output:

```text
[2026-04-05 14:32:01.123 INFO] [trace=abc123def456] Request processed successfully
[2026-04-05 14:32:01.124 DEBUG] [trace=abc123def456] Cache miss for key: user:123
```

You can then use `grep` to filter logs by a specific trace ID:

```bash
grep "trace=abc123def456" /var/log/ferron/access.log
grep "trace=abc123def456" /var/log/ferron/error.log
```

## Centralized observability with OTLP

Use this when shipping logs, metrics, and traces to an OpenTelemetry collector:

```ferron
example.com {
    observability {
        provider otlp

        logs http://otel-collector.internal:4318/v1/logs
        metrics http://otel-collector.internal:4318/v1/metrics
        traces http://otel-collector.internal:4317/v1/traces

        service_name "ferron-prod"
    }

    root /var/www/html
}
```

If you use gRPC OTLP endpoints, set `protocol "grpc"` and optionally an auth header:

```ferron
example.com {
    observability {
        provider otlp

        logs https://otel.example.net/v1/logs {
            protocol "grpc"
            authorization "Bearer YOUR_TOKEN"
        }

        metrics https://otel.example.net/v1/metrics {
            protocol "grpc"
            authorization "Bearer YOUR_TOKEN"
        }

        traces https://otel.example.net/v1/traces {
            protocol "grpc"
            authorization "Bearer YOUR_TOKEN"
        }

        service_name "ferron-prod"
    }
}
```

### Structured (OTEL-style) logs over OTLP

If your collector or APM expects log records in the OpenTelemetry semantic-convention shape, set `log_style modern`. In this mode each log record's body is a short summary (e.g. `"Upstream circuit opened"`) and per-event attributes are published as typed OpenTelemetry attributes. Access logs use a body of `"Access log (http)"` and are remapped to OTEL semantic-convention names such as `url.path`, `http.request.method`, `http.response.status_code`, `client.address`, and `http.server.request.duration`. Local console and file log sinks are unaffected.

```ferron
example.com {
    observability {
        provider otlp
        log_style modern
        service_name "ferron-prod"

        logs https://otel.example.net/v1/logs {
            protocol "http/protobuf"
        }
    }

    root /var/www/html
}
```

### "Traditional" logs over OTLP

If you want to send traditional, human-readable log records via OTLP (e.g., for compatibility with existing logging systems), set `log_style` to `legacy`. This preserves the original log message body and disables per-event attribute mapping.

```ferron
example.com {
    observability {
        provider otlp
        log_style legacy
        service_name "ferron-prod"

        logs https://otel.example.net/v1/logs {
            protocol "http/protobuf"
        }
    }

    root /var/www/html
}
```

## Prometheus metrics monitoring

Use this when you want to expose metrics for Prometheus scraping:

```ferron
example.com {
    observability {
        provider prometheus
        endpoint_listen "127.0.0.1:8889"
        endpoint_format text
    }

    root /var/www/html
}
```

This starts a metrics endpoint at `http://localhost:8889/metrics` that Prometheus can scrape.

### Production Prometheus setup

```ferron
example.com {
    observability {
        provider prometheus
        endpoint_listen "0.0.0.0:8889"
        endpoint_format text
    }

    root /var/www/html
}
```

### Multiple hosts with different metrics ports

```ferron
# Main website
example.com {
    observability {
        provider prometheus
        endpoint_listen "127.0.0.1:9001"
    }
    root /var/www/example
}

# API service
api.example.com {
    observability {
        provider prometheus
        endpoint_listen "127.0.0.1:9002"
    }
    proxy http://backend:3000
}
```

## Hybrid setup: local fallback + OTLP

A practical migration strategy is to keep file logs for local troubleshooting while also exporting telemetry centrally:

```ferron
example.com {
    log "access.log" {
        format json
    }

    observability {
        provider otlp

        logs http://otel-collector.internal:4318/v1/logs
        metrics http://otel-collector.internal:4318/v1/metrics
        traces http://otel-collector.internal:4317/v1/traces

        service_name "ferron-prod"
    }

    root /var/www/html
}
```

### Hybrid setup: Prometheus + OTLP

You can combine both Prometheus and OTLP for maximum flexibility:

```ferron
example.com {
    # Local Prometheus metrics
    observability {
        provider prometheus
        endpoint_listen "127.0.0.1:8889"
    }

    # Centralized OTLP export
    observability {
        provider otlp
        service_name "ferron-prod"

        logs http://otel-collector.internal:4318/v1/logs
        metrics http://otel-collector.internal:4318/v1/metrics
        traces http://otel-collector.internal:4317/v1/traces
    }

    root /var/www/html
}
```
