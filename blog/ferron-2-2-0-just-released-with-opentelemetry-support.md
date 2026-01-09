---
title: "Ferron 2.2.0 just released with OpenTelemetry support"
description: We are excited to announce the release of Ferron 2.2.0. This release brings new features (including OpenTelemetry support) and fixes.
date: 2025-12-03 19:07:00
cover: ./covers/ferron-2-2-0-just-released-with-opentelemetry-support.png
---

We are excited to introduce Ferron 2.2.0, which brings new features and fixes. This release in particular adds support for more advanced observability (like logs, metrics or traces) with OpenTelemetry.

## Key improvements and fixes

### OpenTelemetry support

One of the most important additions in this release is support for more advanced observability with the OpenTelemetry Protocol (OTLP; [relevant feature request](https://github.com/ferronweb/ferron/issues/154)).

Ferron now supports sending logs, metrics and traces to OpenTelemetry-compatible observability backends. This allows for more advanced observability and monitoring of Ferron web server instances.

Below is an example Ferron configuration with OpenTelemetry-based observability:

```kdl
globals {
  // Replace "localhost" with the hostname of your OpenTelemetry backend, such as an OpenTelemetry Collector
  otlp_logs "http://localhost:4317/v1/logs" protocol="grpc"
  otlp_metrics "http://localhost:4317/v1/metrics" protocol="grpc"
  otlp_traces "http://localhost:4317/v1/traces" protocol="grpc"
}

// Serve static files for example.com
example.com {
  otlp_service_name "ferron-example"
  root "/var/www/example.com"
}
```

You can then configure an OpenTelemetry Collector to receive logs, metrics and traces from Ferron, export them to observability backends (such as Prometheus for metrics, Grafana Loki for logs, Jaeger for traces), and visualize them (such as by creating Grafana dashboards).

You can read the [Ferron documentation](/docs/observability) for more information about observability backends.

### Host logging fix

Before implementing OpenTelemetry support, [an issue related to host logging was reported on GitHub](https://github.com/ferronweb/ferron/issues/315), so we fixed it. The web server now logs requests into host-specific access log files properly.

### Default cache item count limit enforcement fix

In the meantime when we added support for OTLP metrics, we also fixed an issue where the default HTTP cache item count limit (1024 items) was not being enforced. This fix ensures that the cache size is properly limited by default, so to prevent out-of-memory (OOM) issues related to the HTTP cache.

### Support for modular observability backend support

Ferron now also supports modular observability backend support, that is, it also allows adding custom observability backend support. If you would like to develop custom observability backend support, you can check out the [example of developing custom observability backend support](https://github.com/ferronweb/ferron-observability-example).

## Thank you!

We appreciate all the feedback and contributions from our community. Your support helps us improve Ferron with each release. Thank you for being a part of this journey!

_The Ferron Team_
