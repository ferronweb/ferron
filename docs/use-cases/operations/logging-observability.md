---
title: Logging & observability
description: "Set up access logs, JSON formatting, trace IDs, Prometheus metrics, and OTLP export for Ferron."
---

Ferron can write local log files and export logs, metrics, and traces to an OpenTelemetry collector. Start with local files. Add Prometheus or OTLP when you need central storage.

The patterns below cover common deployments. For directive details, see [Configuration: logging](/docs/configuration/observability/logging).

- [Prometheus metrics](/docs/configuration/observability/prometheus)
- [OTLP observability](/docs/configuration/observability/otlp)

> [!tip]
> Start with text or JSON logs, then add Prometheus metrics, then OTLP. Logs, metrics, and traces from one HTTP request share the same `trace_id`.
>
> Configure logging and observability in global scope, outside host blocks. Many control plane events, such as automatic TLS and configuration drift, do not belong to a host. Host-scoped logging does not record them.

## Write production logs to files

```ferron
{
    log "access.log"
}

example.com {
    root /var/www/html
}
```

The text formatter uses the Combined Log Format (CLF) by default. Apache and NGINX use the same format.

## Write JSON access logs

JSON output is easier for log processors such as ELK Stack, Splunk, or cloud-native tools:

```ferron
{
    log "access.log" {
        format json
    }
}

example.com {
    root /var/www/html
}
```

Example output:

```json
{
  "method": "GET",
  "path": "/index.html",
  "status": 200,
  "duration_secs": 0.012,
  "client_ip": "127.0.0.1",
  "remote_ip": "127.0.0.1"
}
```

To keep only selected fields:

```ferron
{
    log "access.log" {
        format json
        fields method path status duration_secs client_ip
    }
}

example.com {
    root /var/www/html
}
```

## Custom text log patterns

Set the text layout with `access_pattern`:

```ferron
{
    log "access.log" {
        format text
        access_pattern "%client_ip - %auth_user [%{%d/%b/%Y:%H:%M:%S %z}t] \"%method %path_and_query %version\" %status %content_length \"%{Referer}i\" \"%{User-Agent}i\""
    }
}

example.com {
    root /var/www/html
}
```

## Rotate log files

```ferron
{
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

## Log trace IDs

Trace context is always on. Console and file loggers prefix messages with `[trace=<trace_id>]`. Access logs use the Enhanced Combined Log Format by default. It adds the hostname and trace ID fields.

```text
[2026-04-05 14:32:01.123 INFO] [trace=abc123def456] Request processed successfully
[2026-04-05 14:32:01.124 DEBUG] [trace=abc123def456] Cache miss for key: user:123
```

Filter both logs by trace ID:

```bash
grep "trace=abc123def456" /var/log/ferron/access.log /var/log/ferron/error.log
```

## Per-host logs

Global logs apply to all hosts. A host can override the paths:

```ferron
{
    log "access.log"
    error_log "error.log"
}

example.com {
    log "example-access.log"
    error_log "example-error.log"

    root /var/www/html
}
```

## Export to an OpenTelemetry collector

```ferron
{
    observability {
        provider otlp

        logs http://otel-collector.internal:4318/v1/logs
        metrics http://otel-collector.internal:4318/v1/metrics
        traces http://otel-collector.internal:4317/v1/traces

        service_name ferron-prod
    }
}

example.com {
    root /var/www/html
}
```

For gRPC endpoints, set `protocol grpc` and an auth header:

```ferron
{
    observability {
        provider otlp

        logs https://otel.example.net/v1/logs {
            protocol grpc
            authorization "Bearer YOUR_TOKEN"
        }

        metrics https://otel.example.net/v1/metrics {
            protocol grpc
            authorization "Bearer YOUR_TOKEN"
        }

        traces https://otel.example.net/v1/traces {
            protocol grpc
            authorization "Bearer YOUR_TOKEN"
        }

        service_name ferron-prod
    }
}
```

### OTEL-style log records

Set `log_style modern` when the collector expects OpenTelemetry semantic conventions. Each log record body is a short summary, such as `"Upstream circuit opened"`. Ferron publishes per-event attributes as typed OpenTelemetry attributes.

Access logs use the body `"Access log (http)"`. Ferron maps the fields to names such as `url.path`, `http.request.method`, `http.response.status_code`, `client.address`, and `http.server.request.duration`. Console and file sinks do not change.

```ferron
{
    observability {
        provider otlp
        log_style modern
        service_name ferron-prod

        logs https://otel.example.net/v1/logs {
            protocol "http/protobuf"
        }
    }
}

example.com {
    root /var/www/html
}
```

### Traditional log records

Set `log_style legacy` to keep the original message body and disable per-event attribute mapping:

```ferron
{
    observability {
        provider otlp
        log_style legacy
        service_name ferron-prod

        logs https://otel.example.net/v1/logs {
            protocol "http/protobuf"
        }
    }
}

example.com {
    root /var/www/html
}
```

## Export Prometheus metrics

```ferron
{
    observability {
        provider prometheus
        endpoint_listen "127.0.0.1:8889"
        endpoint_format text
    }
}

example.com {
    root /var/www/html
}
```

Prometheus can scrape `http://localhost:8889/metrics`.

For a protected endpoint:

```ferron
{
    observability {
        provider prometheus
        endpoint_listen "0.0.0.0:8889"
        endpoint_auth_token "your-secret-token"
        endpoint_format text
    }
}

example.com {
    root /var/www/html
}
```

### Separate metrics ports per host

```ferron
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

## Keep local logs and export OTLP

Keep file logs for local troubleshooting and export telemetry at the same time:

```ferron
{
    log "access.log" {
        format json
    }

    observability {
        provider otlp

        logs http://otel-collector.internal:4318/v1/logs
        metrics http://otel-collector.internal:4318/v1/metrics
        traces http://otel-collector.internal:4317/v1/traces

        service_name ferron-prod
    }
}

example.com {
    root /var/www/html
}
```

### Combine Prometheus and OTLP

```ferron
{
    observability {
        provider prometheus
        endpoint_listen "127.0.0.1:8889"
    }

    observability {
        provider otlp
        service_name ferron-prod

        logs http://otel-collector.internal:4318/v1/logs
        metrics http://otel-collector.internal:4318/v1/metrics
        traces http://otel-collector.internal:4317/v1/traces
    }
}

example.com {
    root /var/www/html
}
```

> [!info]
> For OTLP options, see [OTLP observability](/docs/configuration/observability/otlp). For metric names and attributes, see [Metrics](/docs/configuration/observability/metrics).
