---
title: "Configuration: OTLP observability"
description: "OpenTelemetry Protocol (OTLP) export configuration for logs, metrics, and traces."
---

This page documents the OTLP (OpenTelemetry Protocol) observability configuration for Ferron. The `observability-otlp` module exports logs, metrics, and traces to OpenTelemetry collectors. This allows integration with observability platforms such as Jaeger, Loki, Prometheus, and commercial APM solutions.

## Directives

You configure OTLP export via `observability` blocks with `provider otlp`:

```ferron
example.com {
    observability {
        provider otlp

        logs https://collector:4318/v1/logs
        metrics https://collector:4318/v1/metrics
        traces https://collector:4317/v1/traces

        service_name "my-service"
    }
}
```

### Signal sub-blocks

You configure each signal type (`logs`, `metrics`, `traces`) independently. Omitting a signal disables it for that host.

| Directive | Arguments    | Description            | Default  |
| --------- | ------------ | ---------------------- | -------- |
| `logs`    | `<endpoint>` | OTLP logs endpoint.    | disabled |
| `metrics` | `<endpoint>` | OTLP metrics endpoint. | disabled |
| `traces`  | `<endpoint>` | OTLP traces endpoint.  | disabled |

Each signal sub-block supports these nested directives:

| Directive       | Arguments  | Description                                                      | Default                                      |
| --------------- | ---------- | ---------------------------------------------------------------- | -------------------------------------------- |
| `protocol`      | `<string>` | Transport protocol. One of `grpc`, `http/protobuf`, `http/json`. | `grpc` (port 4317), `http/protobuf` (others) |
| `authorization` | `<string>` | HTTP `Authorization` header (HTTP) or gRPC metadata (gRPC).      | none                                         |
| `gzip`          | `[bool]`   | Compress export requests with gzip (HTTP and gRPC).              | `false`                                      |

The `logs` and `traces` sub-blocks also support batching tuning:

| Directive           | Arguments    | Description                                       | Default |
| ------------------- | ------------ | ------------------------------------------------- | ------- |
| `export_interval`   | `<duration>` | Flush interval for a partially full export batch. | `5s`    |
| `export_batch_size` | `<number>`   | Number of finished items that trigger an export.  | `512`   |

The `metrics` sub-block supports collection tuning:

| Directive           | Arguments    | Description                                                                                      | Default |
| ------------------- | ------------ | ------------------------------------------------------------------------------------------------ | ------- |
| `read_interval`     | `<duration>` | Interval at which the metric reader collects and exports all series.                             | `30s`   |
| `exemplars`         | `[bool]`     | Attach the last sampled measurement per series as an exemplar.                                   | `true`  |
| `native_histograms` | `[bool]`     | Aggregate histograms with the exponential layout. Set to `false` for explicit bucket boundaries. | `true`  |

Durations accept a number (seconds), a float (seconds), or a quoted string such as `"10s"`, `"5m"`, or `"1h"`.

Example with per-signal tuning:

```ferron
example.com {
    observability {
        provider otlp

        logs https://collector:4318/v1/logs {
            export_interval "10s"
            export_batch_size 256
        }
        metrics https://collector:4318/v1/metrics {
            read_interval "60s"
        }
        traces https://collector:4317/v1/traces {
            export_interval "5s"
        }
    }
}
```

> [!tip]
> If you have connection issues, verify collector endpoints are reachable with `curl -v https://collector:4317` and check your firewall rules.

> [!note]
> Exemplar export follows the OpenTelemetry convention: only the last sample per series is kept, and only when the sample carries a trace and span ID.

### Global options

| Directive         | Arguments  | Description                                                                                                                           | Default    |
| ----------------- | ---------- | ------------------------------------------------------------------------------------------------------------------------------------- | ---------- |
| `service_name`    | `<string>` | OTLP resource service name.                                                                                                           | `"ferron"` |
| `no_verification` | `[bool]`   | Disable TLS certificate verification. Use with caution.                                                                               | `false`    |
| `log_style`       | `<string>` | Log style for log records. `legacy` keeps the `message` body. `modern` publishes a `summary` with typed attributes and remaps fields. | `"modern"` |
| `authorization`   | `<string>` | Fallback HTTP `Authorization` header (HTTP) or gRPC metadata (gRPC), in case per-signal one is not configured.                        | none       |

> [!tip]
> The OTLP resource automatically includes `process.pid` and `process.start_time` attributes. These attributes let backends distinguish concurrent processes (same PID range after restart) from sequential lifetimes (different start times). This prevents cumulative counters from adjacent process lifetimes from mixing in dashboards.

### Baggage promotion

The `baggage` sub-directive promotes specific W3C Baggage keys into telemetry attributes for logs, metrics, and traces. Use it to add request-scoped context (such as tenant IDs or user roles) to your telemetry signals without custom instrumentation.

```ferron
{
    observability {
        provider otlp

        traces https://collector:4317/v1/traces

        baggage {
            key "tenant.id" {
                attribute "tenant.id"
                signals traces logs
                max_distinct 1000
            }
        }
    }
}
```

Each `key` entry configures one baggage key to promote:

| Nested directive | Arguments           | Description                                                                                    | Default                 |
| ---------------- | ------------------- | ---------------------------------------------------------------------------------------------- | ----------------------- |
| `key`            | `<string>`          | The W3C Baggage key to extract. Required.                                                      | none                    |
| `attribute`      | `<string>`          | The OpenTelemetry attribute name to use.                                                       | same as the baggage key |
| `signals`        | `<string>...`       | Which signals to emit the attribute on. Values: `traces`, `logs`, `metrics`.                   | all signals             |
| `max_distinct`   | `<number> \| false` | Maximum distinct values for metrics before hashing. Prevents high-cardinality label explosion. | 100                     |

> [!tip]
> Ferron parses the `baggage` header and attaches it to spans automatically. Use the `baggage` sub-directive to promote specific keys into telemetry attributes.

> [!info]
> You configure trace sampling in the `http` block. See [Tracing](/docs/v3/configuration/observability/tracing#trace-sampling) for details on configuring sampling modes, ratio-based sampling, and attribute-based sampling.

### Log style

The `log_style` directive selects how log records go over OTLP:

- `legacy` - each log record's body is the human-readable `message` text. The `format` directive (when set) continues to apply to log records. This is the existing behavior.
- `modern` (default) - each log record's body is a short OTEL-friendly `summary` (for example, `"Upstream circuit opened"`). Per-event attributes use OpenTelemetry types (string, boolean, integer, float). The `format` directive does not apply to log records in this mode. Access logs in modern mode use a body of `"Access log (<protocol>)"`. They set the record timestamp from the access event and remap access-log fields onto OTEL semantic-convention attribute names.

The most common access-log field remappings in modern mode are:

| Legacy field                               | OTEL semantic-convention attribute                                 |
| ------------------------------------------ | ------------------------------------------------------------------ |
| `path`                                     | `url.path`                                                         |
| `path_and_query`                           | `url.full`                                                         |
| `method`                                   | `http.request.method`                                              |
| `version`                                  | `network.protocol.version`                                         |
| `scheme`                                   | `url.scheme`                                                       |
| `client_ip_canonical`                      | `client.address`                                                   |
| `client_port`                              | `client.port`                                                      |
| `server_ip_canonical`                      | `server.address`                                                   |
| `server_port`                              | `server.port`                                                      |
| `auth_user`                                | `user.name`                                                        |
| `status`                                   | `http.response.status_code`                                        |
| `content_length`                           | `http.response.body.size`                                          |
| `duration_secs`                            | `http.server.request.duration`                                     |
| `header_<name>`                            | `http.request.header.<name>`                                       |
| `timestamp`, `trace_id`, `span_id`, `*_ip` | dropped (use the record timestamp and standard attributes instead) |
| fields with `.`                            | `<field_name>`                                                     |
| other fields                               | `ferron.custom.<field_name>`                                       |

Example:

```ferron
example.com {
    observability {
        provider otlp
        log_style modern
        service_name "my-service"

        logs https://collector:4318/v1/logs {
            protocol "http/protobuf"
        }
    }
}
```

Setting `log_style modern` together with a `format` directive is not allowed (the validator errors out when you set both).

## Configuration examples

### Basic OTLP configuration

```ferron
example.com {
    observability {
        provider otlp
        service_name "my-ferron-instance"

        traces https://otlp-collector:4317/v1/traces
    }
    root /var/www/html
}
```

### Complete observability setup

```ferron
example.com {
    observability {
        provider otlp
        service_name "ferron-production"

        logs "https://logs-collector:4318/v1/logs" {
            protocol "http/protobuf"
            authorization "Bearer my-secret-token"
        }

        metrics https://metrics-collector:4318/v1/metrics {
            protocol "http/json"
        }

        traces "https://traces-collector:4317/v1/traces"
    }
    root /var/www/html
}
```

### Multiple protocols

```ferron
# Different protocols for different signals
example.com {
    observability {
        provider otlp
        service_name "ferron-mixed"

        logs http://localhost:4318/v1/logs {
            protocol "http/json"
        }

        metrics http://localhost:4318/v1/metrics {
            protocol "http/protobuf"
        }

        traces http://localhost:4317/v1/traces {
            protocol "grpc"
        }
    }
}
```

### Disabling TLS verification (development only)

```ferron
# Only for development/testing
example.com {
    observability {
        provider otlp
        service_name "ferron-dev"
        no_verification

        traces https://localhost:4317/v1/traces
    }
}
```

> [!warning]
> Only disable TLS certificate verification with `no_verification` for development or testing with self-signed certificates.

### Production trace sampling

```ferron
example.com {
    http {
        trace_sampling "parentbased_traceidratio" {
            ratio 0.1
        }
    }

    observability {
        provider otlp
        service_name "ferron-production"

        traces https://collector:4317/v1/traces
    }
}
```

### Attribute-based trace sampling

```ferron
example.com {
    http {
        trace_sampling "attribute_based" {
            default_action "sample"
            rules {
                rule "exact" "http.request.method" "POST"
                rule "prefix" "url.path" "/api/"
            }
        }
    }

    observability {
        provider otlp
        service_name "ferron-production"

        traces https://collector:4317/v1/traces
    }
}
```

### Baggage promotion with cardinality control

```ferron
example.com {
    observability {
        provider otlp
        service_name "my-service"

        traces https://collector:4317/v1/traces

        baggage {
            # Promote tenant ID to traces and logs
            key "tenant.id" {
                attribute "tenant.id"
                signals traces logs
            }

            # Promote user role to all signals with cardinality cap
            key "user.role" {
                attribute "ferron.user_role"
                max_distinct 100
            }
        }
    }
}
```

## Protocol options

Ferron supports three OTLP protocols for exporting signals:

- `grpc` - gRPC protocol for efficient binary communication, recommended for production environments
- `http/protobuf` - HTTP with Protocol Buffers encoding, recommended for compatibility with HTTP-based collectors
- `http/json` - HTTP with JSON encoding, recommended for debugging and development

> [!note]
> Not all collectors support all protocols. Check your collector's documentation.

## Signal correlation

Request traces, request-scoped logs, and access logs from the same HTTP request share the same request span context. This happens when Ferron has a request trace. Ferron attaches baggage from the incoming `baggage` header to the span context. This makes baggage available for correlated queries in your observability backend. For example, "show me all logs for trace `abc123`" or "filter by baggage key `userId`".

> [!note]
> All signals from the same request share the same trace context.

### Trace context propagation

Ferron automatically:

1. **Generates trace IDs** for incoming requests without trace context
2. **Propagates trace context** via W3C Trace Context headers (`traceparent`, `tracestate`)
3. **Propagates baggage** via the W3C Baggage header (`baggage`)
4. **Creates one local request span** per request and nests pipeline, stage, and error-pipeline spans under it
5. **Adds request span context** to OTLP logs and access logs for correlation

When a request carries a `baggage` header, Ferron parses it and attaches the key-value pairs to the OpenTelemetry span context. This makes baggage available to downstream spans and visible in your observability backend as span baggage attributes.

Metrics exported through OTLP do not carry per-request trace or span IDs. Correlate metrics using their semantic attributes, resource attributes, and timestamps. Do not expect a metric data point to join directly to a single trace.

## Observability

### Logs

- `WARN`. Logged when an error occurs with the logs provider.
- `WARN`. Logged when an error occurs with the metrics provider.
- `WARN`. Logged when an error occurs with the traces provider.

### Structured logs

| Description (summary)       | Level | Attributes                              |
| --------------------------- | ----- | --------------------------------------- |
| Error with logs provider    | WARN  | `error.message` (string): error details |
| Error with metrics provider | WARN  | `error.message` (string): error details |
| Error with traces provider  | WARN  | `error.message` (string): error details |

## Best practices

`ferron doctor` reports the following best-practice checks for directives on this page.

- **`max_distinct false` inside Baggage configuration** - do not set high-cardinality attributes in baggage. They can lead to excessive memory usage and performance issues.
- **Service name not explicitly set** - when you do not set an explicit `service_name`, Ferron uses the default value `"ferron"`. This might attribute data incorrectly.
- **"Legacy" log style** - when using `log_style legacy`, OpenTelemetry log reports may be harder to filter or aggregate.
- **`no_verification` enabled**. Only disable TLS verification for OTLP endpoints when testing.

## See also

- [Observability and logging](/docs/v3/configuration/observability/logging) for general observability configuration
- [Tracing](/docs/v3/configuration/observability/tracing) for W3C Trace Context and Baggage propagation details
- [Prometheus metrics](/docs/v3/configuration/observability/prometheus) for native Prometheus metrics export
- [Core directives](/docs/v3/configuration/server/core-directives#observability) for global observability settings
