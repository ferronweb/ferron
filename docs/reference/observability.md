---
title: Observability backends
description: "Configure Ferron observability backends (logfile, OTLP) and learn the exported OpenTelemetry-style metrics, logs, and traces."
---

Ferron 2.2.0 and newer support modular observability backends. This allows you to monitor your Ferron server and gain insights into its performance and behavior.

The following observability backend support is built into Ferron and is enabled by default:

- _logfile_ - this observability backend logs requests and errors into files.
- _otlp_ (Ferron 2.2.0 or newer) - this observability backend sends requests and errors into a service supporting OTLP (such as an OpenTelemetry collector).
- _stdlog_ (Ferron 2.5.0 or newer) - this observability backend logs requests and errors into standard I/O.

Ferron also supports additional observability backends that can be enabled at compile-time.

Additional observability backend support provided by Ferron are from these repositories:

- [ferron-observability-example](https://github.com/ferronweb/ferron-observability-example.git) - logs requests and errors into the console.

If you would like to use Ferron with additional observability backends, you can check the [compilation notes](https://github.com/ferronweb/ferron/blob/2.x/COMPILATION.md).

## Metrics notes

Metrics in Ferron are specified with OpenTelemetry-style names. Below are the metrics sent by Ferron:

- **`http.server.active_requests`** (unit: `{request}`)
  - Number of active HTTP server requests.
  - **Attributes**
    - `http.request.method` - HTTP request method.
    - `url.scheme` - URL scheme (either `"http"` or `"https"`).
    - `network.protocol.name` - Always `"http"`.
    - `network.protocol.version` - HTTP version.
    - `ferron.http.request.error_status_code` - HTTP error status code that occurred before a request handler with error configuration is executed.
- **`http.server.request.duration`** (unit: `s`)
  - Duration of HTTP server requests. This metric also includes requests, where an HTTP response error occurred earlier.
  - **Attributes**
    - `http.request.method` - HTTP request method.
    - `url.scheme` - URL scheme (either `"http"` or `"https"`).
    - `network.protocol.name` - Always `"http"`.
    - `network.protocol.version` - HTTP version.
    - `ferron.http.request.error_status_code` - HTTP error status code that occurred before a request handler with error configuration is executed.
- **`ferron.http.server.request_count`** (unit: `{request}`)
  - Number of HTTP server requests. This metric also includes requests, where an HTTP response error occurred earlier.
  - **Attributes**
    - `http.request.method` - HTTP request method.
    - `url.scheme` - URL scheme (either `"http"` or `"https"`).
    - `network.protocol.name` - Always `"http"`.
    - `network.protocol.version` - HTTP version.
    - `http.response.status_code` - HTTP response status code.
    - `error.type` - Error type (if status code indicates a client or a server error).
    - `ferron.http.request.error_status_code` - HTTP error status code that occurred before a request handler with error configuration is executed.
- **`ferron.proxy.backends.selected`** (unit: `{backend}`; _rproxy_ module)
  - Number of times a backend server was selected.
  - **Attributes**
    - `ferron.proxy.backend_url` - Backend server URL.
    - `ferron.proxy.backend_unix_path` - Backend server Unix socket path.
- **`ferron.proxy.backends.unhealthy`** (unit: `{backend}`; _rproxy_ module)
  - Number of health check failures for a backend server.
  - **Attributes**
    - `ferron.proxy.backend_url` - Backend server URL.
    - `ferron.proxy.backend_unix_path` - Backend server Unix socket path.
- **`ferron.proxy.requests`** (unit: `{request}`; _rproxy_ module; Ferron 2.3.0 or newer)
  - Number of reverse proxy requests.
  - **Attributes**
    - `ferron.proxy.connection_reused` - Whether an HTTP client connection was reused.
- **`ferron.cache.lookups`** (unit: `{lookup}`; _cache_ module)
  - Number of times a cache lookup was performed.
  - **Attributes**
    - `ferron.cache.result` - Cache lookup result (either `"hit"` or `"miss"`).
- **`ferron.cache.items`** (unit: `{item}`; _cache_ module)
  - Number of items in the cache.
- **`ferron.cache.evictions`** (unit: `{eviction}`; _cache_ module)
  - Number of cache evictions (removals of items).
  - **Attributes**
    - `ferron.cache.eviction_reason` - Cache eviction reason (either `"size"` or `"expired"`).
- **`process.cpu.time`** (unit: `s`; Linux)
  - Total CPU seconds broken down by different states.
  - **Attributes**
    - `cpu.mode` - The mode of the CPU (`"user"` or `"system"`)
- **`process.cpu.utilization`** (unit: `1`; Linux)
  - Difference in process.cpu.time since the last measurement, divided by the elapsed time and number of CPUs available to the process.
  - **Attributes**
    - `cpu.mode` - The mode of the CPU (`"user"` or `"system"`)
- **`process.memory.usage`** (unit: `By`; Linux)
  - The amount of physical memory in use.
- **`process.memory.virtual`** (unit: `By`; Linux)
  - The amount of committed virtual memory.

## Observability backend notes

### _otlp_ observability backend

This observability backend support OTLP (OpenTelemetry Protocol) logs, metrics and traces.

For OTLP logs, access logs have an `access` OTLP scope, while error logs have an `error` scope.

For OTLP metrics, they have a `ferron` scope.

For OTLP traces, they have a `ferron` scope.
