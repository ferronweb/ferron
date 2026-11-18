---
title: "Configuration: control plane"
description: "Embed control plane metadata and span links in access logs, metrics, and traces."
---

The `control_plane` block embeds metadata and static OpenTelemetry span links from the server configuration into access logs and traces. A control plane (for example, a Kubernetes ingress controller) writes the configuration. The data plane serves requests.

## Directives

| Sub-block    | Arguments       | Adds                                                      |
| ------------ | --------------- | --------------------------------------------------------- |
| `metadata`   | `<key> <value>` | `ferron.control_plane.*` attributes on observability data |
| `span_links` | none            | Static links on every `ferron.request` span               |

### Metadata injection

Metadata keys appear on every signal:

| Signal             | Form                                                      |
| ------------------ | --------------------------------------------------------- |
| OTLP traces        | `ferron.control_plane.<key>` on the `ferron.request` span |
| OTLP logs          | `ferron.control_plane.<key>` on log records               |
| OTLP metrics       | `ferron.control_plane.<key>` on metric data points        |
| Access logs        | `ferron.control_plane.<key>` on access log records        |
| Console/file logs  | `[key=value]` prefix on log lines                         |
| Prometheus metrics | `ferron_control_plane_<key>` constant label               |

### Span links

Each `span_links` block defines one link:

| Directive    | Type    | Required | Description                                              |
| ------------ | ------- | -------- | -------------------------------------------------------- |
| `trace_id`   | string  | yes      | Trace ID of the linked span. Exactly 32 hex characters.  |
| `span_id`    | string  | yes      | Span ID of the linked span. Exactly 16 hex characters.   |
| `sampled`    | boolean | no       | Whether the linked span was sampled. Default `false`.     |
| `attributes` | block   | no       | Key-value pairs that describe the relationship.          |

For example, `attributes { relationship triggers }`.

## Precedence

`control_plane` can appear at three levels. The most specific level wins:

1. `location` block inside a host.
2. Host block, for example `example.com { ... }`.
3. Global block at the top level of the configuration.

The most specific block replaces the others. Metadata and span links are not merged across levels.

> [!tip]
> Most Ferron directives merge or inherit values across levels. `control_plane` does not.

## Variable interpolation

Metadata values support `{{variable}}` interpolation with request variables:

```ferron
{
    control_plane {
        metadata {
            request_url "{{request.scheme}}://{{request.host}}{{request.uri}}"
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

    proxy http://backend:3000
}
```

### Kubernetes ingress controller

A Kubernetes ingress controller writes the configuration from the `Ingress` resource:

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
        traces http://otlp-collector:4318/v1/traces {
            protocol http/protobuf
        }
    }
}
```

Use the metadata and links to:

- Find all requests for an ingress with `ferron.control_plane.ingress_name`.
- Join traces with control plane events by the span link trace ID.
- Compare clusters by filtering metrics on `ferron.control_plane.cluster`.

### Signal output

With metadata `{ org_id acme team platform }`:

| Signal                  | Appearance                                  |
| ----------------------- | ------------------------------------------- |
| OTLP trace attribute    | `ferron.control_plane.org_id: "acme"`       |
| Console log             | `[org_id=acme] [team=platform] request ...` |
| Prometheus metric label | `ferron_control_plane_org_id{...}`          |

## See also

- [OTLP observability](/docs/configuration/observability/otlp): OTLP export configuration
- [Tracing](/docs/configuration/observability/tracing): W3C Trace Context and sampling
- [Prometheus metrics](/docs/configuration/observability/prometheus): native Prometheus metrics export
