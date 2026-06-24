---
title: "Configuration: HTTP TLS provider"
description: "Obtain TLS certificates from a remote HTTP endpoint, with automatic refresh and observability."
---

This page documents the `http` TLS provider (`tls-http` module), which fetches TLS certificates from a **remote HTTP API**. It supports two modes:

- **Polling mode** (default): Polls a single endpoint at a configurable interval, suitable for known domains.
- **On-demand mode** (`on_demand true`): Fetches certificates lazily on first TLS handshake for each SNI hostname, with optional approval endpoint, suitable for wildcard domains.

Unlike the ACME provider, this module does not perform certificate issuance or challenge validation — it simply fetches a certificate chain and private key in JSON format from a configured endpoint.

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
| `refresh_interval` | `<duration>` | `1h` | How often to poll or refresh certificates |
| `no_verification` | `<bool>` | `false` | Skip TLS verification for the certificate endpoint |
| `on_demand` | `<bool>` | `false` | Enable on-demand (lazy) certificate fetching |
| `on_demand_ask` | `<string>` | — | Approval endpoint URL for on-demand requests |
| `on_demand_ask_auth` | `<string>` | — | Authorization header for the approval endpoint |
| `on_demand_ask_no_verification` | `<bool>` | `false` | Skip TLS verification for the approval endpoint |

**Polling mode configuration example:**

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

## Polling mode

Polling mode is the default behavior. The module fetches the certificate from the configured `url` at startup and then every `refresh_interval`, updating the in-memory TLS configuration if the certificate has changed. This mode works with a known domain in the host block.

## On-demand mode

On-demand mode defers certificate fetching until the **first TLS handshake** for a hostname. This is useful for wildcard domains, multi-tenant hosting, or when domains are not known at startup.

```ferron
*.example.com {
    tls {
        provider http
        url "https://cert-manager.internal.example.com/api/cert"
        on_demand
    }
}
```

When a TLS handshake arrives for a hostname without a cached certificate:

1. The module sends an on-demand request to the background listener.
2. If `on_demand_ask` is configured, the module calls the approval endpoint with `?domain=<encoded>` as a query parameter. A `200` response authorizes the fetch.
3. The module fetches the certificate from `url` with `?domain=<encoded>` appended to the URL.
4. The certificate is cached per SNI hostname in memory.
5. A per-SNI refresh task is spawned to re-fetch the certificate at the configured `refresh_interval`.

### On-demand approval endpoint

To prevent abuse, you can configure an approval endpoint. Before fetching a certificate, Ferron sends an HTTP GET request to the endpoint with `?domain=<sni>` as a query parameter. If the response is `200`, the certificate is fetched.

```ferron
*.example.com {
    tls {
        provider http
        url "https://cert-manager.internal.example.com/api/cert"
        on_demand
        on_demand_ask "https://internal-api.example.com/check-cert"
    }
}
```

## How it works

The `tls-http` module runs background tasks depending on the mode:

### Polling mode

1. **Polls** the configured URL at the specified `refresh_interval`
2. **Parses** the JSON response expecting `certificate` (PEM-encoded chain) and `private_key` (PEM-encoded key) fields
3. **Replaces** the current TLS certified key if the certificate has changed
4. **Continues** polling indefinitely until the server shuts down

### On-demand mode

1. **Listens** for on-demand requests triggered by TLS handshakes for unknown SNI hostnames
2. **Checks** the approval endpoint (if configured) to authorize the fetch
3. **Fetches** the certificate from the configured `url` with `?domain=<encoded>` appended
4. **Caches** the certificate per SNI hostname in the in-memory resolver
5. **Refreshes** each certificate independently at the configured `refresh_interval`

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

### Basic polling usage

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

### On-demand with approval endpoint

```ferron
*.example.com {
    tls {
        provider http
        url "https://cert-manager.internal.example.com/api/cert"
        on_demand
        on_demand_ask "https://internal-api.example.com/check-cert"
        on_demand_ask_auth "Bearer s3cr3t"
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

The `refresh_interval` directive controls how often certificates are refreshed. The default is **1 hour**. Shorter intervals mean faster certificate updates but more HTTP requests. Longer intervals reduce load on the certificate service but delay certificate rotation.

### Continuous operation

The refresh loops run indefinitely. If a request fails (network error, parse error, etc.), the module logs a warning and retries on the next interval. The currently loaded certificate remains in effect until a successful response is received.

## Observability

The `tls-http` module emits log events and metrics through the configured observability pipeline for troubleshooting and monitoring.

### Log events

| Level | Message | When |
|-------|---------|------|
| `INFO` | `TLS-HTTP certificate polling started for <url>` | Background polling task started |
| `INFO` | `TLS certificate refreshed successfully from HTTP endpoint` | Certificate updated (polling or on-demand refresh) |
| `INFO` | `On-demand certificate requested` | On-demand certificate request received |
| `INFO` | `On-demand certificate fetched` | On-demand certificate fetched successfully |
| `WARN` | `Failed to build HTTP request for 'tls-http': <error>` | Request construction failed |
| `WARN` | `Failed to send HTTP request for 'tls-http': <error>` | HTTP request failed |
| `WARN` | `Failed to parse the HTTP response from TLS certificate endpoint: <error>` | JSON parse error |
| `WARN` | `Failed to parse the TLS certificate chain from TLS endpoint response: <error>` | PEM chain parse error |
| `WARN` | `Failed to parse the TLS private key from TLS endpoint response: <error>` | PEM key parse error |
| `WARN` | `Failed to load the TLS private key: <error>` | Key loading error |
| `WARN` | `Can't build TLS client configuration for 'tls-http'` | Invalid TLS config |
| `ERROR` | `Certificate issuance denied` | Ask endpoint denied the request |
| `ERROR` | `Ask endpoint error` | Ask endpoint request failed |
| `ERROR` | `On-demand config not found` | No matching on-demand config for request |

### Structured logs

In OTLP `log_style modern`, the `summary` field is used as the log body and `attributes` are emitted as typed OpenTelemetry log record attributes.

| Summary | Level | Attributes |
|---------|-------|------------|
| TLS-HTTP polling started | INFO | `ferron.tls_http.url` (string) — certificate endpoint URL |
| TLS-HTTP client config build failed | WARN | — |
| TLS-HTTP request build failed | WARN | `error.message` (string) |
| TLS-HTTP request failed | WARN | `error.message` (string) |
| TLS-HTTP endpoint error | WARN | `http.status_code` (int) — HTTP status returned by endpoint |
| TLS-HTTP response read failed | WARN | `error.message` (string) |
| TLS-HTTP response parse failed | WARN | `error.message` (string) |
| TLS-HTTP certificate chain parse failed | WARN | `error.message` (string) |
| TLS-HTTP private key parse failed | WARN | `error.message` (string) |
| TLS-HTTP private key load failed | WARN | `error.message` (string) |
| TLS-HTTP certificate refreshed | INFO | `ferron.tls_http.host` (string) — hostname this certificate serves |
| On-demand certificate requested | INFO | `tls.sni` (string), `tls.port` (int) |
| On-demand certificate fetched | INFO | `tls.sni` (string), `tls.port` (int) |
| Certificate issuance denied | ERROR | `tls.sni` (string) — hostname blocked by ask endpoint |
| Ask endpoint error | ERROR | `tls.sni` (string), `error.message` (string) |
| On-demand config not found | ERROR | `tls.sni` (string), `tls.port` (int) |

### Metrics

| Metric | Type | Attributes | Description |
|--------|------|--------|-------------|
| `ferron.tls_http.requests_total` | Counter | `status` (`success`, `error`) | Total HTTP requests to the certificate endpoint |
| `ferron.tls_http.request_duration_seconds` | Histogram | `status` (`success`, `error`) | HTTP request duration in seconds |
| `ferron.tls_http.certificates_refreshed_total` | Counter | `status` (`success`, `error`) | Certificate refresh outcomes |
| `ferron.tls_http.on_demand_requests_total` | Counter | — | On-demand certificate requests |
| `ferron.tls.certificate_not_after` | Gauge | `ferron.host`, `ferron.tls.provider` (`http`), `crypto.certificate.serial_number` | Certificate `notAfter` as Unix epoch seconds |
| `ferron.tls_http.next_refresh_seconds` | Gauge | — | Seconds until next certificate refresh |

The certificate expiration gauge is shared across all TLS providers (manual, ACME, HTTP, local) and is emitted every time a certificate is mounted into the in-memory context.

## Security considerations

- **Private keys are never logged** or exposed in error messages.
- The certificate endpoint URL should be protected with authentication (e.g., API keys, mTLS) in production.
- Use `no_verification` only for internal endpoints with self-signed certificates — never for public endpoints.
- The private key is loaded into memory and used only for TLS — it is never written to disk by this module.
- If the certificate endpoint returns a valid but untrusted certificate chain, Ferron will still use it. Ensure your endpoint only returns certificates from trusted CAs.
- When using on-demand mode, always configure an `on_demand_ask` endpoint in production to prevent certificate fetching for arbitrary hostnames.
- The `url` endpoint receives the domain in the `?domain=` query parameter — ensure it validates and authenticates requests.

## Troubleshooting

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

### On-demand certificates not being fetched

If on-demand certificates aren't being fetched for new hostnames:

1. Verify `on_demand true` is set in the TLS block
2. Check the logs for `On-demand certificate requested` — if missing, the resolver may not be receiving handshakes
3. If `on_demand_ask` is configured, verify the endpoint returns `200` for the requested hostname
4. Check that the host block uses a wildcard pattern (e.g., `*:443`)

### Observability data missing

If metrics or logs aren't appearing:

1. Verify that an observability backend (Prometheus, OTLP, etc.) is configured
2. Check that the `observability` block is present in the global configuration
3. Verify that the `metrics-prometheus` or `observability-otlp` module is loaded

## See also

- [Security and TLS](/docs/v3/configuration/security/tls) — cipher suites, ECDH curves, mTLS
- [ACME automatic TLS](/docs/v3/configuration/security/acme) — production TLS certificates via ACME
- [TLS session ticket keys](/docs/v3/configuration/security/session-tickets) — session resumption
- [OCSP stapling](/docs/v3/configuration/security/ocsp) — OCSP response stapling
- [HTTP host directives](/docs/v3/configuration/server/host) — per-host TLS configuration

## Best practices

The following best-practice checks are reported by `ferron doctor` for directives on this page.

- **`url` with plain HTTP** — Certificate endpoints returning private keys should use HTTPS with authentication.
- **`no_verification` for certificate endpoint** — Disabling TLS verification for the certificate endpoint should only be used for strictly internal and otherwise authenticated endpoints.
- **`on_demand` without `on_demand_ask`** — On-demand certificate fetching without an approval endpoint allows certificate fetching for arbitrary hostnames. Configure `on_demand_ask` to approve requests.
- **`on_demand_ask_no_verification`** — Disabling TLS verification for the approval endpoint should only be used for strictly internal and otherwise authenticated endpoints.
