---
title: Logging & observability
description: "Practical Ferron setups for access logs, JSON formatting, container-friendly outputs, and OTLP export for centralized observability."
---

Ferron supports multiple observability outputs, so you can start with local log files and later move to centralized telemetry without changing your application stack.

This page focuses on common deployment patterns. For directive-level details, see [Configuration: observability and logging](/docs/v3/configuration/observability-logging).

For specific backend configurations:

- [Prometheus metrics](/docs/v3/configuration/observability-prometheus)
- [OTLP observability](/docs/v3/configuration/observability-otlp)

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
        format "json"
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
        format "json"
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

## Centralized observability with OTLP

Use this when shipping logs, metrics, and traces to an OpenTelemetry collector:

```ferron
example.com {
    observability {
        provider "otlp"

        logs "http://otel-collector.internal:4318/v1/Logs" {
            protocol "http/protobuf"
        }

        metrics "http://otel-collector.internal:4318/v1/Metrics" {
            protocol "http/protobuf"
        }

        traces "http://otel-collector.internal:4317" {
            protocol "grpc"
        }

        service_name "ferron-prod"
    }

    root /var/www/html
}
```

If you use gRPC OTLP endpoints, set `protocol "grpc"` and optionally an auth header:

```ferron
example.com {
    observability {
        provider "otlp"

        logs "https://otel.example.net/v1/logs" {
            protocol "grpc"
            authorization "Bearer YOUR_TOKEN"
        }

        metrics "https://otel.example.net/v1/metrics" {
            protocol "grpc"
            authorization "Bearer YOUR_TOKEN"
        }

        traces "https://otel.example.net/v1/traces" {
            protocol "grpc"
            authorization "Bearer YOUR_TOKEN"
        }

        service_name "ferron-prod"
    }
}
```

## Prometheus metrics monitoring

Use this when you want to expose metrics for Prometheus scraping:

```ferron
example.com {
    observability {
        provider "prometheus"
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
        provider "prometheus"
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
        provider "prometheus"
        endpoint_listen "127.0.0.1:9001"
    }
    root /var/www/example
}

# API service
api.example.com {
    observability {
        provider "prometheus"
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
        format "json"
    }

    observability {
        provider "otlp"

        logs "http://otel-collector.internal:4318/v1/Logs" {
            protocol "http/protobuf"
        }

        metrics "http://otel-collector.internal:4318/v1/Metrics" {
            protocol "http/protobuf"
        }

        traces "http://otel-collector.internal:4317" {
            protocol "grpc"
        }

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
        provider "prometheus"
        endpoint_listen "127.0.0.1:8889"
    }

    # Centralized OTLP export
    observability {
        provider "otlp"
        service_name "ferron-prod"

        logs "http://otel-collector.internal:4318/v1/Logs" {
            protocol "http/protobuf"
        }

        metrics "http://otel-collector.internal:4318/v1/Metrics" {
            protocol "http/protobuf"
        }

        traces "http://otel-collector.internal:4317" {
            protocol "grpc"
        }
    }

    root /var/www/html
}
```

## Notes and troubleshooting

- Start simple: text or JSON logs first, then add Prometheus metrics, then OTLP for full observability.
- Keep `no_verify false` unless you are in a controlled test environment.
- If logs are missing, verify the formatter modules are loaded in your Ferron build and check endpoint/protocol pairing.
- All three signals (logs, metrics, traces) from the same HTTP request share the same `trace_id`, enabling correlated queries.
- If Ferron is behind a reverse proxy, configure `client_ip_from_header` so Ferron can see real client IPs. See [HTTP host directives](/docs/v3/configuration/http-host).
- For available access log fields, see [Configuration: observability and logging](/docs/v3/configuration/observability-logging#access-log-fields).
- For Prometheus metrics configuration, see [Prometheus metrics](/docs/v3/configuration/observability-prometheus).
- For OTLP configuration details, see [OTLP observability](/docs/v3/configuration/observability-otlp).
