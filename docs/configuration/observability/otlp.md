---
title: "Configuration: OTLP observability"
description: "Export logs, metrics, and traces to OpenTelemetry collectors over OTLP."
---

The `observability-otlp` module exports logs, metrics, and traces to OpenTelemetry collectors. You can send the data to Jaeger, Loki, Prometheus, or a commercial APM.

## Directives

Configure OTLP export in an `observability` block with `provider otlp`:

```ferron
example.com {
    observability {
        provider otlp

        logs https://collector:4318/v1/logs
        metrics https://collector:4318/v1/metrics
        traces https://collector:4317/v1/traces

        service_name my-service
    }
}
```

### Signals

Each signal has its own endpoint. When you omit a signal, it is disabled for that host.

| Directive | Arguments    | Purpose              | Default  |
| --------- | ------------ | -------------------- | -------- |
| `logs`    | `<endpoint>` | OTLP logs endpoint.  | disabled |
| `metrics` | `<endpoint>` | OTLP metrics endpoint. | disabled |
| `traces`  | `<endpoint>` | OTLP traces endpoint. | disabled |

Each signal sub-block accepts these directives:

| Directive       | Arguments  | Description                                                      | Default                                      |
| --------------- | ---------- | ---------------------------------------------------------------- | -------------------------------------------- |
| `protocol`      | `<string>` | Transport protocol: `grpc`, `http/protobuf`, or `http/json`.     | `grpc` (port 4317), `http/protobuf` (others) |
| `authorization` | `<string>` | HTTP `Authorization` header (HTTP) or gRPC metadata (gRPC).      | none                                         |
| `gzip`          | `[bool]`   | Compress export requests with gzip. Works for HTTP and gRPC.     | `false`                                      |

### Batching

The `logs` and `traces` sub-blocks support batching tuning:

| Directive           | Arguments    | Description                                       | Default |
| ------------------- | ------------ | ------------------------------------------------- | ------- |
| `export_interval`   | `<duration>` | Flush interval for a partially full export batch. | `5s`    |
| `export_batch_size` | `<number>`   | Finished items that trigger an export.            | `512`   |

### Metrics collection tuning

The `metrics` sub-block supports collection tuning:

| Directive           | Arguments    | Description                                                                                      | Default |
| ------------------- | ------------ | ------------------------------------------------------------------------------------------------ | ------- |
| `read_interval`     | `<duration>` | Interval at which the metric reader collects and exports all series.                             | `30s`   |
| `exemplars`         | `[bool]`     | Attach the last sampled measurement per series as an exemplar.                                   | `true`  |
| `native_histograms` | `[bool]`     | Aggregate histograms with the exponential layout. Set to `false` for explicit bucket boundaries. | `true`  |

Durations accept a number in seconds, a float in seconds, or a quoted string such as `"10s"`, `"5m"`, or `"1h"`.

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
> If the collector connection fails, check reachability with `curl -v https://collector:4317` and check the firewall rules.

> [!note]
> Exemplar export follows the OpenTelemetry convention. Ferron keeps only the last sample per series, and only when the sample carries a trace ID and span ID.

### Global options

| Directive         | Arguments  | Description                                                                                                                           | Default    |
| ----------------- | ---------- | ------------------------------------------------------------------------------------------------------------------------------------- | ---------- |
| `service_name`    | `<string>` | OTLP resource service name.                                                                                                           | `"ferron"` |
| `no_verification` | `[bool]`   | Disable TLS certificate verification. Use with caution.                                                                               | `false`    |
| `log_style`       | `<string>` | Log record style. `legacy` keeps the `message` body. `modern` publishes a `summary` with typed attributes and remaps fields.           | `"modern"` |
| `authorization`   | `<string>` | Fallback HTTP `Authorization` header (HTTP) or gRPC metadata (gRPC), when a signal has no value of its own.                           | none       |

> [!tip]
> OTLP resources include `process.pid` and `process.start_time` attributes. Backends use them to separate process lifetimes, so counters from different lifetimes do not mix.

### Baggage promotion

The `baggage` sub-directive promotes W3C Baggage keys into telemetry attributes for logs, metrics, and traces. Use it to add request-scoped context, such as tenant IDs or user roles, without custom instrumentation.

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

Each `key` entry configures one baggage key:

| Directive      | Arguments           | Description                                                                                     | Default                 |
| -------------- | ------------------- | ----------------------------------------------------------------------------------------------- | ----------------------- |
| `key`          | `<string>`          | W3C Baggage key to extract. Required.                                                           | none                    |
| `attribute`    | `<string>`          | OpenTelemetry attribute name.                                                                   | same as the baggage key |
| `signals`      | `<string>...`       | Signals that receive the attribute: `traces`, `logs`, `metrics`.                                | all signals             |
| `max_distinct` | `<number> \| false` | Maximum distinct values for metrics before dropping. Prevents high-cardinality label explosion.  | 100                     |

> [!tip]
> Ferron parses the `baggage` header and attaches it to spans. Use `baggage` to promote specific keys into telemetry attributes.

> [!info]
> Configure trace sampling in the `http` block. See [Tracing](/docs/configuration/observability/tracing#trace-sampling) for sampling modes, ratio-based sampling, and attribute-based sampling.

### Log style

The `log_style` directive selects the format of OTLP log records:

- `legacy`: the record body is the human-readable `message` text. The `format` directive continues to apply.
- `modern` (default): the record body is a short OpenTelemetry-friendly `summary`, for example `"Upstream circuit opened"`. Per-event attributes use OpenTelemetry types (string, boolean, integer, float). The `format` directive does not apply.

In modern mode, access logs use a body of `"Access log (<protocol>)"`. The record timestamp comes from the access event. Access-log fields are remapped to OpenTelemetry semantic-convention attribute names.

The most common remappings:

| Legacy field                               | OpenTelemetry attribute                                          |
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
| `res_header_<name>`                        | `http.response.header.<name>`                                      |
| `timestamp`, `trace_id`, `span_id`, `*_ip` | dropped. Use the record timestamp and standard attributes instead. |
| fields with `.`                            | `<field_name>`                                                     |
| other fields                               | `ferron.custom.<field_name>`                                       |

Example:

```ferron
example.com {
    observability {
        provider otlp
        log_style modern
        service_name my-service

        logs https://collector:4318/v1/logs {
            protocol http/protobuf
        }
    }
}
```

> [!warning]
> The validator rejects `log_style modern` together with a `format` directive.

## Configuration examples

### Basic OTLP configuration

```ferron
example.com {
    observability {
        provider otlp
        service_name my-ferron-instance

        traces https://otlp-collector:4317/v1/traces
    }
}
```

### Complete observability setup

```ferron
example.com {
    observability {
        provider otlp
        service_name ferron-production

        logs https://logs-collector:4318/v1/logs {
            protocol http/protobuf
            authorization "Bearer my-secret-token"
        }

        metrics https://metrics-collector:4318/v1/metrics {
            protocol http/json
        }

        traces https://traces-collector:4317/v1/traces
    }
}
```

### Disable TLS verification (development only)

```ferron
example.com {
    observability {
        provider otlp
        service_name ferron-dev
        no_verification

        traces https://localhost:4317/v1/traces
    }
}
```

> [!warning]
> Use `no_verification` only for development or testing with self-signed certificates.

### Production trace sampling

```ferron
example.com {
    http {
        trace_sampling parentbased_traceidratio {
            ratio 0.1
        }
    }

    observability {
        provider otlp
        service_name ferron-production

        traces https://collector:4317/v1/traces
    }
}
```

## Protocol options

| Protocol        | Use                                                        |
| --------------- | ---------------------------------------------------------- |
| `grpc`          | Binary gRPC. Recommended for production.                   |
| `http/protobuf` | HTTP with Protocol Buffers. For HTTP-based collectors.     |
| `http/json`     | HTTP with JSON. For debugging and development.             |

> [!note]
> Not all collectors support all protocols. Check your collector documentation.

## Signal correlation

Traces, request-scoped logs, and access logs from the same request share one span context when Ferron has a request trace. Ferron attaches values from the incoming `baggage` header to the span context. Backends can then query by baggage key, for example `userId`.

### Trace context propagation

Ferron does the following for each request:

1. Generates a trace ID when the request has no trace context.
2. Propagates trace context in the W3C headers `traceparent` and `tracestate`.
3. Propagates baggage in the W3C `baggage` header.
4. Creates one local request span and nests pipeline, stage, and error-pipeline spans under it.
5. Adds the request span context to OTLP logs and access logs.

When a request carries a `baggage` header, Ferron parses it and attaches the key-value pairs to the OpenTelemetry span context. Downstream spans and the backend can then read the baggage.

> [!note]
> All signals from the same request share the same trace context.

OTLP metrics do not carry per-request trace or span IDs. Correlate metrics by semantic attributes, resource attributes, and timestamps. A metric data point does not join directly to a single trace.

## Export error logs

Each provider logs a warning when an export error occurs:

| Summary                     | Level | Attributes                              |
| --------------------------- | ----- | --------------------------------------- |
| Error with logs provider    | WARN  | `error.message` (string): error details |
| Error with metrics provider | WARN  | `error.message` (string): error details |
| Error with traces provider  | WARN  | `error.message` (string): error details |

## Best practices

`ferron doctor` reports these checks for directives on this page:

- **`max_distinct false` in baggage**: do not set high-cardinality attributes in baggage. They can increase memory use and reduce performance.
- **Service name not set**: without `service_name`, Ferron uses `"ferron"`. This can attribute data incorrectly.
- **`log_style legacy`**: OpenTelemetry log reports are harder to filter or aggregate in legacy mode.
- **`no_verification` enabled**: disable TLS verification only for testing.

## See also

- [Observability and logging](/docs/configuration/observability/logging): general observability configuration
- [Tracing](/docs/configuration/observability/tracing): W3C Trace Context and Baggage propagation
- [Prometheus metrics](/docs/configuration/observability/prometheus): native Prometheus metrics export
- [Core directives](/docs/configuration/server/core-directives#observability): global observability settings
