---
title: Logging and observability
description: "Practical Ferron setups for access/error logs, container-friendly std streams, and OTLP export for centralized observability."
---

Ferron supports multiple observability outputs, so you can start with local log files and later move to centralized telemetry without changing your application stack.

This page focuses on common deployment patterns. For directive-level details, see [Configuration: observability & logging](/docs/configuration/observability-logging) and [Observability backends reference](/docs/reference/observability).

## 1. Basic production logs to files

Use this when running Ferron directly on a VM or bare metal and collecting logs from disk.

```kdl
globals {
    log_date_format "%d/%b/%Y:%H:%M:%S %z"
    log_format "{client_ip} - {auth_user} [{timestamp}] \"{method} {path_and_query} {version}\" {status_code} {content_length} \"{header:Referer}\" \"{header:User-Agent}\""
}

example.com {
    log "/var/log/ferron/example.com.access.log"
    error_log "/var/log/ferron/example.com.error.log"
}
```

## 2. Container-friendly logging to stdout/stderr

Use this when running Ferron in containers (Docker, Kubernetes), where platform log collectors read process streams.

`stdlog` directives are available in Ferron 2.5.0 and newer.

```kdl
example.com {
    // Access logs to stdout
    log_stdout

    // Error logs to stderr
    error_log_stderr
}
```

If your platform expects everything on one stream, you can use `log_stderr` and `error_log_stderr`, or `log_stdout` and `error_log_stdout`.

## 3. Centralized observability with OTLP

Use this when shipping logs, metrics, and traces to an OpenTelemetry collector.

OTLP directives are available in Ferron 2.2.0 and newer.

```kdl
globals {
    otlp_service_name "ferron-prod"

    otlp_logs "http://otel-collector.internal:4317" protocol="grpc"
    otlp_metrics "http://otel-collector.internal:4317" protocol="grpc"
    otlp_traces "http://otel-collector.internal:4317" protocol="grpc"
}
```

If you use HTTP OTLP endpoints, set `protocol="http/protobuf"` (or `"http/json"`) and optionally an auth header:

```kdl
globals {
    otlp_logs "https://otel.example.net/v1/logs" protocol="http/protobuf" authorization="Bearer YOUR_TOKEN"
    otlp_metrics "https://otel.example.net/v1/metrics" protocol="http/protobuf" authorization="Bearer YOUR_TOKEN"
    otlp_traces "https://otel.example.net/v1/traces" protocol="http/protobuf" authorization="Bearer YOUR_TOKEN"
}
```

## 4. Hybrid setup: local fallback + OTLP

A practical migration strategy is to keep file logs for local troubleshooting while also exporting telemetry centrally.

```kdl
globals {
    otlp_service_name "ferron-prod"
    otlp_logs "http://otel-collector.internal:4317" protocol="grpc"
    otlp_metrics "http://otel-collector.internal:4317" protocol="grpc"
    otlp_traces "http://otel-collector.internal:4317" protocol="grpc"
}

example.com {
    log "/var/log/ferron/example.com.access.log"
    error_log "/var/log/ferron/example.com.error.log"
}
```

## Notes and troubleshooting

- Start simple: file logs or std streams first, OTLP second.
- Keep `otlp_no_verification #false` unless you are in a controlled test environment.
- If logs are missing, verify backend support in your Ferron build and check endpoint/protocol pairing.
- Use [placeholders reference](/docs/configuration/placeholders) when customizing `log_format`.
