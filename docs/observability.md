---
title: Observability backends
---

Ferron UNRELEASED and newer support modular observability backends. This allows you to monitor your Ferron server and gain insights into its performance and behavior.

The following observability backend support is built into Ferron and is enabled by default:

- _logfile_ - this observability backend logs requests and errors into files.
- _otlp_ (Ferron UNRELEASED or newer) - this observability backend sends requests and errors into a service supporting OTLP (such as an OpenTelemetry collector).

Ferron also supports additional observability backends that can be enabled at compile-time.

Additional observability backend support provided by Ferron are from these repositories:

**TODO: add observability backends**

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
- **`http.server.request.duration`** (unit: `s`)
  - Duration of HTTP server requests.
  - **Attributes**
    - `http.request.method` - HTTP request method.
    - `url.scheme` - URL scheme (either `"http"` or `"https"`).
    - `network.protocol.name` - Always `"http"`.
    - `network.protocol.version` - HTTP version.
- **`ferron.http.server.request_count`** (unit: `{request}`)
  - Number of HTTP server requests.
  - **Attributes**
    - `http.request.method` - HTTP request method.
    - `url.scheme` - URL scheme (either `"http"` or `"https"`).
    - `network.protocol.name` - Always `"http"`.
    - `network.protocol.version` - HTTP version.
    - `http.response.status_code` - HTTP response status code.
    - `error.type` - Error type (if status code indicates a client or a server error).
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

## Observability backend notes

### _otlp_ observability backend

This observability backend support OTLP (OpenTelemetry Protocol) logs and metrics.

For OTLP logs, access logs have an `access` OTLP scope, while error logs have an `error` scope.

For OTLP metrics, they have a `ferron` scope.
