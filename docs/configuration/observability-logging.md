---
title: "Configuration: observability & logging"
description: "Access log, error log, and OTLP/std stream observability directives for Ferron configuration."
---

This page describes KDL directives for configuring Ferron logging outputs, formats, and OpenTelemetry export endpoints.

## Directives

### Logging

- `log_date_format <log_date_format: string>`
  - This directive specifies the date format (according to POSIX) for the access log file. Default: `"%d/%b/%Y:%H:%M:%S %z"`
- `log_format <log_format: string>`
  - This directive specifies the entry format for the access log file. The placeholders can be found in the reference below the section specifying. Default: `"{client_ip} - {auth_user} [{timestamp}] \"{method} {path_and_query} {version}\" {status_code} {content_length} \"{header:Referer}\" \"{header:User-Agent}\""` (Combined Log Format)
- `log_json [<property_name>=<property_template: string>]...` (Ferron 2.7.0 or newer)
  - This directive switches access logs to JSON output. The default JSON object contains `timestamp`, `client_ip`, `auth_user`, `method`, `path_and_query`, `version`, `status_code`, `content_length`, `referer`, and `user_agent`. Optional props add extra string properties whose values are rendered from access-log placeholders. If `log_json` is set, `log_format` is ignored, while `log_date_format` still controls the `timestamp` field. Default: disabled
- `log <log_file_path: string>` (_logfile_ observability backend)
  - This directive specifies the path to the access log file, which contains the HTTP response logs in either text (Combined Log Format by default) or JSON format (Ferron 2.7.0 or newer) depending on the logging directives in use. This directive was a global and virtual host directive before Ferron 2.2.0. Default: none
- `error_log <error_log_file_path: string>` (_logfile_ observability backend)
  - This directive specifies the path to the error log file. This directive was a global and virtual host directive before Ferron 2.2.0. Default: none
- `otlp_no_verification [otlp_no_verification: bool]` (_otlp_ observability backend; Ferron 2.2.0 or newer)
  - This directive specifies whether the server should not verify the TLS certificate of the OTLP (OpenTelemetry Protocol) endpoint. Default: `otlp_no_verification #false`
- `otlp_service_name <otlp_service_name: string>` (_otlp_ observability backend; Ferron 2.2.0 or newer)
  - This directive specifies the service name to be used in the OTLP (OpenTelemetry Protocol) endpoint. Default: `otlp_service_name "ferron"`
- `otlp_logs <otlp_logs_endpoint: string|null> [authorization=<otlp_logs_authorization: string>] [protocol=<otlp_logs_protocol: string>]` (_otlp_ observability backend; Ferron 2.2.0 or newer)
  - This directive specifies the endpoint URL to be used for logging logs into the OTLP (OpenTelemetry Protocol) endpoint. The `authorization` prop is a value for `Authorization` HTTP header, if HTTP protocol is used. The `protocol` prop specifies a protocol to use (`grpc` for gRPC, `http/protobuf` for HTTP with protobuf data, `http/json` for HTTP with JSON data). HTTP and HTTPS (only for HTTP-based protocols) URLs are supported. Default: `otlp_logs #null protocol="grpc"`
- `otlp_metrics <otlp_metrics_endpoint: string|null> [authorization=<otlp_metrics_authorization: string>] [protocol=<otlp_metrics_protocol: string>]` (_otlp_ observability backend; Ferron 2.2.0 or newer)
  - This directive specifies the endpoint URL to be used for logging metrics into the OTLP (OpenTelemetry Protocol) endpoint. The `authorization` prop is a value for `Authorization` HTTP header, if HTTP protocol is used. The `protocol` prop specifies a protocol to use (`grpc` for gRPC, `http/protobuf` for HTTP with protobuf data, `http/json` for HTTP with JSON data). HTTP and HTTPS (only for HTTP-based protocols) URLs are supported. Default: `otlp_metrics #null protocol="grpc"`
- `otlp_traces <otlp_traces_endpoint: string|null> [authorization=<otlp_traces_authorization: string>] [protocol=<otlp_traces_protocol: string>]` (_otlp_ observability backend; Ferron 2.2.0 or newer)
  - This directive specifies the endpoint URL to be used for logging traces into the OTLP (OpenTelemetry Protocol) endpoint. The `authorization` prop is a value for `Authorization` HTTP header, if HTTP protocol is used. The `protocol` prop specifies a protocol to use (`grpc` for gRPC, `http/protobuf` for HTTP with protobuf data, `http/json` for HTTP with JSON data). HTTP and HTTPS (only for HTTP-based protocols) URLs are supported. Default: `otlp_traces #null protocol="grpc"`
- `log_stdout [enable_log_stdout: bool]` (_stdlog_ observability backend; Ferron 2.5.0 or newer)
  - This directive specifies whether to enable logging HTTP response logs (access logs) to the standard output stream. Default: `log_stdout #false`
- `log_stderr [enable_log_stderr: bool]` (_stdlog_ observability backend; Ferron 2.5.0 or newer)
  - This directive specifies whether to enable logging HTTP response logs (access logs) to the standard error stream. Default: `log_stderr #false`
- `error_log_stdout [enable_error_log_stdout: bool]` (_stdlog_ observability backend; Ferron 2.5.0 or newer)
  - This directive specifies whether to enable logging error logs to the standard output stream. Default: `error_log_stdout #false`
- `error_log_stderr [enable_error_log_stderr: bool]` (_stdlog_ observability backend; Ferron 2.5.0 or newer)
  - This directive specifies whether to enable logging error logs to the standard error stream. Default: `error_log_stderr #false`
- `log_rotate_size <log_rotate_size: integer|null>` (_log_ observability backend; Ferron 2.6.0 or newer)
  - This directive specifies the maximum size (in bytes) of an access log file before it is rotated. After rotation, previous log files are renamed with a numeric suffix. If the value is `#null`, log rotation is disabled. Default: `log_rotate_size #null`
- `log_rotate_keep <log_rotate_keep: integer|null>` (_log_ observability backend; Ferron 2.6.0 or newer)
  - This directive specifies the maximum number of access log files to keep. If the value is `#null`, log rotation is disabled. Default: `log_rotate_count #null`
- `error_log_rotate_size <error_log_rotate_size: integer|null>` (_log_ observability backend; Ferron 2.6.0 or newer)
  - This directive specifies the maximum size (in bytes) of an error log file before it is rotated. After rotation, previous log files are renamed with a numeric suffix. If the value is `#null`, log rotation is disabled. Default: `error_log_rotate_size #null`
- `error_log_rotate_keep <error_log_rotate_keep: integer|null>` (_log_ observability backend; Ferron 2.6.0 or newer)
  - This directive specifies the maximum number of error log files to keep. If the value is `#null`, log rotation is disabled. Default: `error_log_rotate_count #null`

**Configuration example:**

```kdl
* {
    log_date_format "%d/%b/%Y:%H:%M:%S %z"
    log_json request_id="{header:X-Request-Id}" request_target="{method} {path_and_query}"
}

example.com {
    log "/var/log/ferron/example.com.access.log"
    error_log "/var/log/ferron/example.com.error.log"
}
```
