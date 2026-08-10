---
title: "Configuration: StatsD metrics"
description: "StatsD metrics export configuration for sending Ferron metrics to a StatsD server."
---

This page documents the StatsD metrics export configuration for Ferron. The `observability-statsd` module sends the internal metrics of Ferron to a StatsD server over UDP. This lets you integrate with StatsD-compatible monitoring stacks, such as Graphite, Telegraf, and Datadog Agent.

The module supports the [StatsD protocol](https://github.com/statsd/statsd) and optional [DogStatsD extensions](https://docs.datadoghq.com/developers/dogstatsd/). You can enable the DogStatsD extensions with the `datadog` directive.

## Directives

You configure StatsD metrics in `observability` blocks with `provider statsd`:

```ferron
observability {
    provider statsd
    host "127.0.0.1"
    port 8125
    prefix "myapp"
    datadog true
}
```

### Configuration directives

| Directive | Arguments          | Description                                                                                      | Default        |
| --------- | ------------------ | ------------------------------------------------------------------------------------------------ | -------------- |
| `provider` | `"statsd"`        | Specifies the StatsD observability provider. Required.                                          | none           |
| `host`    | `<hostname>`       | Hostname or IP address of the StatsD server.                                                     | `"127.0.0.1"`  |
| `port`    | `<number>`         | UDP port of the StatsD server. Must be between 1 and 65535.                                      | `8125`         |
| `prefix`  | `<string>`         | Prefix prepended to every metric name with a `.` separator.                                      | none           |
| `datadog` | `<bool>`           | Enable DogStatsD extensions: metric tags and the histogram metric type.                          | `false`        |

The server sends each metric as a separate UDP datagram. The module does not wait for acknowledgments, so a missing or slow StatsD server does not slow down Ferron.

> [!note]
> UDP is a best-effort protocol. A StatsD server that is unreachable drops metric datagrams silently. Monitor the StatsD server health separately.

## Metric name prefixes

The `prefix` directive prepends a namespace to every metric name. This is useful when several services share one StatsD server. With `prefix "myapp"`, the metric `ferron.http.server.request_count` becomes `myapp.ferron.http.server.request_count`.

Without a `prefix` directive, the module sends metric names as-is. Ferron metric names already start with `ferron.`, so you usually do not need a prefix.

> [!note]
> The prefix must not contain the StatsD reserved characters `:`, `|`, `#`, or `@`.

## Metric type mapping

Ferron emits OpenTelemetry-style metric events. The module maps them to StatsD types:

| Ferron metric type | StatsD type | Value semantics                                |
| ------------------ | ----------- | ---------------------------------------------- |
| `Counter`          | `c`         | Increment delta                                |
| `Gauge`            | `g`         | Absolute value                                 |
| `UpDownCounter`    | `g`         | Signed delta (`+3\|g` or `-3\|g`)              |
| `Histogram`        | `ms` or `h` | Single observation (see below)                 |

Histogram metrics become timers with the `ms` type. When the metric unit is seconds, the module converts the value to milliseconds. This matches the StatsD timer convention.

In Datadog mode (`datadog true`), histogram metrics use the DogStatsD histogram type `h` instead of `ms`. The module does not convert the unit in Datadog mode, because DogStatsD applies its own histogram aggregations.

## Datadog extensions

Set `datadog true` to enable DogStatsD extensions:

- **Metric tags**. The module renders metric attributes as DogStatsD tags, for example `|#ferron.host:localhost,http.response.status_code:200`. Control plane metadata becomes tags with the `ferron_control_plane_` key prefix.
- **Histogram metric type**. Histogram metrics use the DogStatsD histogram type `h`.

Tag values are sanitized for the DogStatsD tag syntax. The reserved characters `,`, `#`, and `:` become `?`. Values longer than 128 characters are replaced with a deterministic hash. This prevents tag injection and high-cardinality tag explosion.

Without `datadog`, the module does not add tags and sends all histogram metrics as `ms` timers.

## Example

```ferron
observability {
    provider statsd
    host "statsd.internal.example.com"
    port 8125
    prefix "web"
}
```

This configuration sends metrics such as `web.ferron.http.server.request_count:1|c` to `statsd.internal.example.com:8125`.

For more information about the metrics Ferron emits, see [Metrics](/docs/v3/configuration/observability/metrics).
