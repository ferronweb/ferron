---
title: "Configuration: StatsD metrics"
description: "Send Ferron metrics to a StatsD server over UDP, with optional DogStatsD extensions."
---

The `observability-statsd` module sends Ferron metrics to a StatsD server over UDP. It supports the [StatsD protocol](https://github.com/statsd/statsd) and optional [DogStatsD extensions](https://docs.datadoghq.com/developers/dogstatsd/), which you enable with `datadog`.

## Directives

```ferron
example.com {
    observability {
        provider statsd
        host 127.0.0.1
        port 8125
        prefix myapp
        datadog
    }
}
```

| Directive  | Arguments    | Description                                                             | Default       |
| ---------- | ------------ | ----------------------------------------------------------------------- | ------------- |
| `provider` | `"statsd"`   | Selects the StatsD provider. Required.                                  | none          |
| `host`     | `<hostname>` | Hostname or IP address of the StatsD server.                            | `"127.0.0.1"` |
| `port`     | `<number>`   | UDP port of the StatsD server, from 1 to 65535.                         | `8125`        |
| `prefix`   | `<string>`   | Prefix for every metric name, separated by a `.`.                       | none          |
| `datadog`  | `<bool>`     | Enable DogStatsD extensions: metric tags and the histogram type.        | `false`       |
| `baggage`  | `{ ... }`    | Promote W3C Baggage keys into DogStatsD tags.                           | none          |

Ferron sends each metric as a separate UDP datagram and does not wait for acknowledgments. A missing or slow StatsD server does not slow down Ferron.

> [!note]
> UDP is a best-effort protocol. An unreachable StatsD server drops metric datagrams silently. Monitor the StatsD server health separately.

## Metric name prefixes

`prefix` prepends a namespace and a `.` to every metric name. This separates services that share one StatsD server. With `prefix myapp`, the metric `ferron.http.server.request_count` becomes `myapp.ferron.http.server.request_count`.

Without `prefix`, the module sends metric names unchanged. Ferron metric names already start with `ferron.`, so you usually do not need a prefix.

> [!note]
> The prefix must not contain the StatsD reserved characters `:`, `|`, `#`, or `@`.

## Metric type mapping

Ferron maps its OpenTelemetry-style metric events to StatsD types:

| Ferron metric type | StatsD type | Value semantics                   |
| ------------------ | ----------- | --------------------------------- |
| `Counter`          | `c`         | Increment delta                   |
| `Gauge`            | `g`         | Absolute value                    |
| `UpDownCounter`    | `g`         | Signed delta (`+3\|g` or `-3\|g`) |
| `Histogram`        | `ms` or `h` | Single observation                |

Histogram metrics become timers with the `ms` type. When the metric unit is seconds, the module converts the value to milliseconds, which matches the StatsD timer convention.

With `datadog true`, histograms use the DogStatsD histogram type `h` instead of `ms`. The module does not convert the unit in Datadog mode because DogStatsD applies its own histogram aggregation.

## Datadog extensions

`datadog true` enables two extensions:

- Metric tags. The module renders metric attributes as DogStatsD tags, for example `|#ferron.host:localhost,http.response.status_code:200`. Control plane metadata keys use the `ferron_control_plane_` prefix.
- Histogram metric type. Histograms use the DogStatsD type `h`.

Tag values are sanitized for DogStatsD syntax. The reserved characters `,`, `#`, and `:` become `?`, which prevents tag injection.

Without `datadog`, the module adds no tags and sends all histograms as `ms` timers.

## Baggage promotion

The `baggage` sub-directive promotes W3C Baggage keys into DogStatsD tags. Use it to add request-scoped context, such as tenant IDs or user roles, without custom instrumentation. Promoted keys are emitted only when `datadog` is enabled, because vanilla StatsD has no tag syntax.

```ferron
example.com {
    observability {
        provider statsd
        datadog

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

| Directive      | Arguments           | Description                                                                           | Default                 |
| -------------- | ------------------- | ------------------------------------------------------------------------------------- | ----------------------- |
| `key`          | `<string>`          | W3C Baggage key to extract. Required.                                                 | none                    |
| `attribute`    | `<string>`          | DogStatsD tag name.                                                                   | same as the baggage key |
| `max_distinct` | `<number> \| false` | Maximum distinct tag values before dropping. Prevents high-cardinality tag explosion.  | 100                     |

> [!warning]
> Tags with high-cardinality values can reduce performance in the receiving backend. Always set `max_distinct` on baggage keys with unbounded values, such as user IDs or request IDs. Values above the cap are dropped.

## Example

```ferron
example.com {
    observability {
        provider statsd
        host statsd.internal.example.com
        port 8125
        prefix web
    }
}
```

This configuration sends metrics such as `web.ferron.http.server.request_count:1|c` to `statsd.internal.example.com:8125`.

## See also

- [Metrics](/docs/configuration/observability/metrics): metrics that Ferron emits
- [DogStatsD extensions](https://docs.datadoghq.com/developers/dogstatsd/): tag and histogram syntax
