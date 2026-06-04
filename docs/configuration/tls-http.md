---
title: "Configuration: HTTP TLS provider"
description: "Obtain TLS certificates from a remote HTTP endpoint, with automatic refresh and observability."
---

This page documents the `http` TLS provider (`tls-http` module), which fetches TLS certificates from a **remote HTTP API** at a configurable interval. Unlike the ACME provider, this module does not perform certificate issuance or challenge validation — it simply polls an endpoint that returns a certificate chain and private key in JSON format.

This is useful when you have an **external certificate management service** (e.g., HashiCorp Vault, a custom PKI, or a cloud certificate manager) that exposes certificates via a REST API.

```ferron
example.com {
    tls {
        provider http
        url "https://cert-manager.internal.example.com/api/cert/example.com"
    }
}
```

## Directives

### Configuration parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `provider` | `http` | — | Must be set to `"http"` |
| `url` | `<string>` | — | URL to fetch the certificate from (required) |
| `refresh_interval` | `<duration>` | `1h` | How often to poll the certificate endpoint |
| `no_verification` | `<bool>` | `false` | Skip TLS verification for the certificate endpoint |

**Configuration example:**

```ferron
example.com {
    tls {
        provider http
        url "https://cert-manager.internal.example.com/api/cert/example.com"
        refresh_interval "30m"

        ocsp {
            enabled true
        }
    }
}
```

## How it works

The `tls-http` module runs a background task that:

1. **Polls** the configured URL at the specified `refresh_interval`
2. **Parses** the JSON response expecting `certificate` (PEM-encoded chain) and `private_key` (PEM-encoded key) fields
3. **Replaces** the current TLS certified key if the certificate has changed
4. **Continues** polling indefinitely until the server shuts down

The HTTP client supports both HTTP/1.1 and HTTP/2. TLS verification is enabled by default — use `no_verification` only for internal endpoints with self-signed certificates.

### Response format

The endpoint must return a JSON object with the following structure:

```json
{
    "private_key": "-----BEGIN PRIVATE KEY-----\n...",
    "certificate": "-----BEGIN CERTIFICATE-----\n...\n-----END CERTIFICATE-----\n-----BEGIN CERTIFICATE-----\n...\n-----END CERTIFICATE-----"
}
```

- `private_key`: PEM-encoded private key (any supported format)
- `certificate`: PEM-encoded certificate chain, with the leaf certificate first, followed by intermediates

## Configuration examples

### Basic usage

```ferron
example.com {
    tls {
        provider http
        url "https://cert-manager.internal.example.com/api/cert/example.com"
    }
}
```

### With custom refresh interval

```ferron
example.com {
    tls {
        provider http
        url "https://cert-manager.internal.example.com/api/cert/example.com"
        refresh_interval "15m"
    }
}
```

### With TLS verification disabled (internal endpoints)

```ferron
example.com {
    tls {
        provider http
        url "https://cert-manager.internal.example.com/api/cert/example.com"
        no_verification
    }
}
```

### With OCSP stapling

```ferron
example.com {
    tls {
        provider http
        url "https://cert-manager.internal.example.com/api/cert/example.com"

        ocsp {
            enabled true
        }
    }
}
```

## Certificate refresh behavior

### Change detection

The module compares the newly fetched certificate chain against the currently loaded one. If the certificates are identical, the TLS configuration is **not** updated, avoiding unnecessary client reconnections.

### Refresh interval

The `refresh_interval` directive controls how often the endpoint is polled. The default is **1 hour**. Shorter intervals mean faster certificate updates but more HTTP requests. Longer intervals reduce load on the certificate service but delay certificate rotation.

### Continuous operation

The polling loop runs indefinitely. If a request fails (network error, parse error, etc.), the module logs a warning and retries on the next interval. The currently loaded certificate remains in effect until a successful response is received.

## Observability

The `tls-http` module emits log events and metrics through the configured observability pipeline for troubleshooting and monitoring.

### Log events

| Level | Message | When |
|-------|---------|------|
| `INFO` | `TLS-HTTP certificate polling started for <url>` | Background task started |
| `INFO` | `TLS certificate refreshed successfully from HTTP endpoint` | Certificate updated |
| `WARN` | `Failed to build HTTP request for 'tls-http': <error>` | Request construction failed |
| `WARN` | `Failed to send HTTP request for 'tls-http': <error>` | HTTP request failed |
| `WARN` | `Failed to parse the HTTP response from TLS certificate endpoint: <error>` | JSON parse error |
| `WARN` | `Failed to parse the TLS certificate chain from TLS endpoint response: <error>` | PEM chain parse error |
| `WARN` | `Failed to parse the TLS private key from TLS endpoint response: <error>` | PEM key parse error |
| `WARN` | `Failed to load the TLS private key: <error>` | Key loading error |
| `WARN` | `Can't build TLS client configuration for 'tls-http'` | Invalid TLS config |

### Metrics

| Metric | Type | Attributes | Description |
|--------|------|--------|-------------|
| `ferron.tls_http.requests_total` | Counter | `status` (`success`, `error`) | Total HTTP requests to the certificate endpoint |
| `ferron.tls_http.request_duration_seconds` | Histogram | `status` (`success`, `error`) | HTTP request duration in seconds |
| `ferron.tls_http.certificates_refreshed_total` | Counter | `status` (`success`, `error`) | Certificate refresh outcomes |
| `ferron.tls.certificate_not_after` | Gauge | `ferron.host`, `ferron.tls.provider` (`http`), `crypto.certificate.serial_number` | Certificate `notAfter` as Unix epoch seconds |
| `ferron.tls_http.next_refresh_seconds` | Gauge | — | Seconds until next certificate refresh |

The certificate expiration gauge is shared across all TLS providers (manual, ACME, HTTP, local) and is emitted every time a certificate is mounted into the in-memory context.

## Security considerations

- **Private keys are never logged** or exposed in error messages.
- The certificate endpoint URL should be protected with authentication (e.g., API keys, mTLS) in production.
- Use `no_verification` only for internal endpoints with self-signed certificates — never for public endpoints.
- The private key is loaded into memory and used only for TLS — it is never written to disk by this module.
- If the certificate endpoint returns a valid but untrusted certificate chain, Ferron will still use it. Ensure your endpoint only returns certificates from trusted CAs.

## Notes and troubleshooting

### "Failed to parse the HTTP response from TLS certificate endpoint: ..."

The endpoint returned a response that couldn't be parsed as JSON. Check that:

- The endpoint returns valid JSON with `private_key` and `certificate` fields
- The response content type is `application/json`
- There are no encoding issues (e.g., BOM characters)

### "Failed to parse the TLS certificate chain from TLS endpoint response: ..."

The `certificate` field couldn't be parsed as PEM. Check that:

- The certificate is in PEM format (starts with `-----BEGIN CERTIFICATE-----`)
- The chain includes the leaf certificate first, followed by intermediates
- There are no extra whitespace or encoding issues

### "Failed to parse the TLS private key from TLS endpoint response: ..."

The `private_key` field couldn't be parsed as PEM. Check that:

- The key is in PEM format (starts with `-----BEGIN PRIVATE KEY-----` or similar)
- The key format is supported (RSA, EC, Ed25519)
- There are no extra whitespace or encoding issues

### Certificate not updating

If the certificate isn't updating despite changes on the server side:

1. Check the logs for `TLS certificate refreshed successfully` — if missing, the fetch may be failing
2. Verify the endpoint URL is correct and reachable
3. Check `ferron.tls.certificate_not_after` (with `ferron.tls.provider="http"`) to see when the loaded certificate actually expires
4. Ensure the `refresh_interval` isn't too long for your use case

### Observability data missing

If metrics or logs aren't appearing:

1. Verify that an observability backend (Prometheus, OTLP, etc.) is configured
2. Check that the `observability` block is present in the global configuration
3. Verify that the `metrics-prometheus` or `observability-otlp` module is loaded

## See also

- [Security and TLS](/docs/v3/configuration/security-tls) — cipher suites, ECDH curves, mTLS
- [ACME automatic TLS](/docs/v3/configuration/tls-acme) — production TLS certificates via ACME
- [TLS session ticket keys](/docs/v3/configuration/tls-session-tickets) — session resumption
- [OCSP stapling](/docs/v3/configuration/ocsp-stapling) — OCSP response stapling
- [HTTP host directives](/docs/v3/configuration/http-host) — per-host TLS configuration

## Best practices

The following best-practice checks are reported by `ferron doctor` for directives on this page.

- **`url` with plain HTTP** — Certificate endpoints returning private keys should use HTTPS with authentication.
- **`no_verification` for certificate endpoint** — Disabling TLS verification for the certificate endpoint should only be used for strictly internal and otherwise authenticated endpoints.
