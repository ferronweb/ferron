---
title: "Configuration: control plane"
description: "Embed contextual metadata and span links from the control plane into access logs and traces."
---

This page documents the `control_plane` directive. It embeds contextual metadata and static OpenTelemetry span links from the server configuration into access logs and traces. A control plane (for example, a Kubernetes ingress controller) writes the configuration. The data plane serves requests. The `control_plane` directive bridges the gap between them.

> [!info]
>
> - For OTLP export configuration, see [OTLP observability](/docs/v3/configuration/observability/otlp).
> - For tracing configuration, see [Tracing](/docs/v3/configuration/observability/tracing).

## Directives

The `control_plane` block accepts two sub-blocks:

| Sub-block    | Arguments       | Description                                                                                                                   |
| ------------ | --------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| `metadata`   | `<key> <value>` | Arbitrary key-value pairs injected as `ferron.control_plane.*` attributes on all observability signals.                       |
| `span_links` | —               | Static OpenTelemetry span links attached to every `ferron.request` span, creating causal connections to control plane traces. |

### Metadata injection

Metadata values are automatically included as:

- **OTLP traces** — `ferron.control_plane.<key>` attributes on the `ferron.request` span
- **OTLP logs** — `ferron.control_plane.<key>` attributes on log records
- **OTLP metrics** — `ferron.control_plane.<key>` attributes on metric data points
- **Access logs** — `ferron.control_plane.<key>` attributes on access log records
- **Console/file logs** — `[key=value]` prefix prepended to log lines
- **Prometheus metrics** — `ferron_control_plane_<key>` constant labels

### Span links

Each `span_links` block defines one link with:

| Directive    | Type    | Required | Description                                                                |
| ------------ | ------- | -------- | -------------------------------------------------------------------------- |
| `trace_id`   | string  | yes      | 32 hex characters (the trace ID of the linked span)                        |
| `span_id`    | string  | yes      | 16 hex characters (the span ID of the linked span)                         |
| `sampled`    | boolean | no       | Whether the linked span was sampled (default: `false`)                     |
| `attributes` | block   | no       | Key-value pairs describing the relationship (e.g. `relationship triggers`) |

## Precedence

The `control_plane` directive can appear at three levels. When present at multiple levels, the most specific one wins:

1. **Location** (most specific) — inside a `location` block within a host
2. **Host** — inside a host block (for example, `example.com { ... }`)
3. **Global** (least specific) — at the top level of the configuration

Metadata and span links from more specific levels **fully replace** those from less specific levels — they are not merged.

> [!tip]
> Unlike most Ferron directives, metadata and span links are not merged across levels. A more specific `control_plane` block replaces the one from a less specific level.

## Variable interpolation

Metadata values support variable interpolation, allowing you to reference request variables:

```ferron
{
    control_plane {
        metadata {
            request_url "${scheme}://${host}${request_uri}"
        }
    }
}
```

## Examples

### Global metadata

```ferron
{
    control_plane {
        metadata {
            cluster production-us-east-1
            controller ferron-ingress
        }
    }
}

*:80 {
    root /var/www/ferron
}
```

### Per-host metadata with span links

```ferron
{
    control_plane {
        metadata {
            cluster production
        }
    }
}

api.example.com:80 {
    control_plane {
        metadata {
            service api-gateway
            version v2
        }
        span_links {
            trace_id "0af7651916cd43dd8448eb211c80319c"
            span_id "00f067aa0ba902b7"
            sampled
            attributes {
                relationship deploys
            }
        }
    }

    proxy "http://backend:3000"
}
```

### Full Kubernetes ingress controller example

A Kubernetes ingress controller would write the server configuration with metadata derived from the `Ingress` resource:

```ferron
{
    control_plane {
        metadata {
            cluster prod-us-east-1
        }
        span_links {
            trace_id "0af7651916cd43dd8448eb211c80319c"
            span_id "00f067aa0ba902b7"
            sampled
            attributes {
                relationship manages
                resource_type ingress
                resource_name my-app
                resource_namespace default
            }
        }
    }
    root /var/www/my-app
    observability {
        provider otlp
        service_name my-app
        traces "http://otlp-collector:4318/v1/traces" {
            protocol http/protobuf
        }
    }
}
```

This enables operators to:

- Query traces by `ferron.control_plane.ingress_name` to find all requests for a specific ingress
- Join traces with control plane events using the span link's trace ID
- Filter metrics by `ferron.control_plane.cluster` to compare performance across clusters

### Observability signal examples

When you configure metadata `{ org_id acme team platform }`:

| Signal                  | Appearance                                  |
| ----------------------- | ------------------------------------------- |
| OTLP trace attribute    | `ferron.control_plane.org_id: "acme"`       |
| Console log             | `[org_id=acme] [team=platform] request ...` |
| Prometheus metric label | `ferron_control_plane_org_id{...}`          |

## See also

- [OTLP observability](/docs/v3/configuration/observability/otlp) — OTLP export configuration
- [Tracing](/docs/v3/configuration/observability/tracing) — W3C Trace Context and sampling
- [Prometheus metrics](/docs/v3/configuration/observability/prometheus) — native Prometheus metrics export
