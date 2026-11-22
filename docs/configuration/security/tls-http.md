---
title: "Configuration: HTTP TLS provider"
description: "Fetch TLS certificates from a remote HTTP endpoint, with polling, on-demand fetching, and observability."
---

The `http` provider fetches a certificate chain and private key from a remote HTTP API. It does not issue certificates or solve ACME challenges. Use it when an external service, such as HashiCorp Vault, a custom PKI, or a cloud certificate manager, exposes certificates through a REST API.

| Mode                  | Behavior                                                        | Use it for                        |
| --------------------- | --------------------------------------------------------------- | --------------------------------- |
| Polling (default)     | Fetches one URL at a fixed interval.                            | Known domains.                    |
| On-demand             | Fetches on the first TLS handshake for each SNI hostname.       | Wildcards and multi-tenant hosts. |

```ferron
example.com {
    tls {
        provider http
        url https://cert-manager.internal.example.com/api/cert/example.com
    }
}
```

## Options

| Parameter                       | Type         | Default | Description                                        |
| ------------------------------- | ------------ | ------- | -------------------------------------------------- |
| `provider`                      | `http`       | none    | Must be `http`.                                    |
| `url`                           | `<string>`   | none    | URL to fetch the certificate from (required).      |
| `refresh_interval`              | `<duration>` | `1h`    | How often to poll or refresh certificates.         |
| `no_verification`               | `<bool>`     | `false` | Skip TLS verification for the certificate endpoint. |
| `on_demand`                     | `<bool>`     | `false` | Fetch certificates lazily on the first handshake.  |
| `on_demand_ask`                 | `<string>`   | none    | Approval endpoint URL for on-demand requests.      |
| `on_demand_ask_auth`            | `<string>`   | none    | Authorization header for the approval endpoint.    |
| `on_demand_ask_no_verification` | `<bool>`     | `false` | Skip TLS verification for the approval endpoint.   |

```ferron
example.com {
    tls {
        provider http
        url https://cert-manager.internal.example.com/api/cert/example.com
        refresh_interval "30m"

        ocsp
    }
}
```

## Polling mode

The provider fetches the certificate at startup and then at every `refresh_interval`. When the certificate changes, Ferron updates the in-memory TLS configuration. Polling continues until the server shuts down.

The HTTP client supports HTTP/1.1 and HTTP/2. TLS verification is enabled by default.

## On-demand mode

On-demand mode defers fetching until the first TLS handshake for a hostname. Use it for wildcard domains, multi-tenant hosting, or domains that are unknown at startup.

```ferron
*.example.com {
    tls {
        provider http
        url https://cert-manager.internal.example.com/api/cert
        on_demand
    }
}
```

When a handshake arrives for a hostname without a cached certificate:

1. The module sends an on-demand request to the background listener.
2. If `on_demand_ask` is set, the module calls the approval endpoint with `?domain=<encoded>`.
3. A `200` response authorizes the fetch.
4. The module fetches `url` with `?domain=<encoded>` appended.
5. The module caches the certificate per SNI hostname in memory and starts a per-SNI refresh task.

### Approval endpoint

An approval endpoint prevents certificate fetching for arbitrary hostnames. Ferron sends an HTTP GET request to the endpoint with `?domain=<sni>`. A `200` response authorizes the fetch.

```ferron
*.example.com {
    tls {
        provider http
        url https://cert-manager.internal.example.com/api/cert
        on_demand
        on_demand_ask https://internal-api.example.com/check-cert
        on_demand_ask_auth "Bearer s3cr3t"
    }
}
```

## Response format

The endpoint must return a JSON object:

```json
{
  "private_key": "-----BEGIN PRIVATE KEY-----\n...",
  "certificate": "-----BEGIN CERTIFICATE-----\n...\n-----END CERTIFICATE-----\n-----BEGIN CERTIFICATE-----\n...\n-----END CERTIFICATE-----"
}
```

- `private_key`: PEM-encoded private key. RSA, EC, and Ed25519 keys are supported.
- `certificate`: PEM-encoded certificate chain, with the leaf certificate first and intermediates after it.

> [!warning]
> A refresh fails when the endpoint returns invalid JSON, or when PEM data does not parse. The log reports one of these messages:
>
> - `Failed to parse the HTTP response from TLS certificate endpoint: ...`: the response is not valid JSON. Check that the endpoint returns `application/json` without a BOM, and that the body has `private_key` and `certificate` fields.
> - `Failed to parse the TLS certificate chain from TLS endpoint response: ...`: the `certificate` field is not valid PEM. Check that it starts with `-----BEGIN CERTIFICATE-----`, with the leaf certificate first and intermediates after it.
> - `Failed to parse the TLS private key from TLS endpoint response: ...`: the `private_key` field is not valid PEM. Check that it starts with `-----BEGIN PRIVATE KEY-----` or a supported equivalent, and that the key is RSA, EC, or Ed25519.

## Refresh behavior

Ferron compares each newly fetched chain with the loaded one. Identical certificates do not update the TLS configuration, so clients are not disconnected unnecessarily.

A shorter `refresh_interval` applies certificate changes sooner but sends more requests. A longer interval reduces load on the certificate service but delays rotation.

If a fetch fails, Ferron logs a warning and retries at the next interval. The loaded certificate stays in effect until a fetch succeeds.

### Certificate not updating

1. Look for `TLS certificate refreshed successfully` in the logs. If it is absent, the fetch is failing.
2. Check that the endpoint URL is correct and reachable.
3. Read `ferron.tls.certificate_not_after` with `ferron.tls.provider="http"` to see when the loaded certificate expires.
4. Check that `refresh_interval` is not too long for your rotation schedule.

### On-demand fetch not triggered

1. Check that the TLS block sets `on_demand`.
2. Look for `On-demand certificate requested` in the logs. If it is absent, the resolver may not receive handshakes.
3. If `on_demand_ask` is set, check that the endpoint returns `200` for the hostname.
4. Check that the host block uses a wildcard pattern, for example `*:443`.

## Security considerations

- Ferron never logs private keys or exposes them in error messages.
- The provider loads the private key into memory for TLS only. It never writes the key to disk.
- Protect the certificate endpoint with authentication, such as API keys or mTLS.
- Use `no_verification` only for internal endpoints with self-signed certificates.
- Ferron uses a fetched chain even when it is not trusted. Make sure the endpoint returns certificates from trusted CAs.
- In on-demand mode, always configure `on_demand_ask` in production.
- The endpoint receives the domain in `?domain=`. It must validate and authenticate requests.

## Observability

### Log events

| Level   | Message                                                                         | When                                               |
| ------- | ------------------------------------------------------------------------------- | -------------------------------------------------- |
| `INFO`  | `TLS-HTTP certificate polling started for <url>`                                | Background polling task started                    |
| `INFO`  | `TLS certificate refreshed successfully from HTTP endpoint`                     | Certificate updated (polling or on-demand refresh) |
| `INFO`  | `On-demand certificate requested`                                               | On-demand certificate request received             |
| `INFO`  | `On-demand certificate fetched`                                                 | On-demand certificate fetched successfully         |
| `WARN`  | `Failed to build HTTP request for 'tls-http': <error>`                          | Request construction failed                        |
| `WARN`  | `Failed to send HTTP request for 'tls-http': <error>`                           | HTTP request failed                                |
| `WARN`  | `Failed to parse the HTTP response from TLS certificate endpoint: <error>`      | JSON parse error                                   |
| `WARN`  | `Failed to parse the TLS certificate chain from TLS endpoint response: <error>` | PEM chain parse error                              |
| `WARN`  | `Failed to parse the TLS private key from TLS endpoint response: <error>`       | PEM key parse error                                |
| `WARN`  | `Failed to load the TLS private key: <error>`                                   | Key loading error                                  |
| `WARN`  | `Can't build TLS client configuration for 'tls-http'`                           | Invalid TLS config                                 |
| `ERROR` | `Certificate issuance denied`                                                   | Ask endpoint denied the request                    |
| `ERROR` | `Ask endpoint error`                                                            | Ask endpoint request failed                        |
| `ERROR` | `On-demand config not found`                                                    | No matching on-demand config for request           |

### Structured logs

In OTLP `log_style modern`, the `summary` field becomes the log body. The `attributes` are typed OpenTelemetry log record attributes.

| Summary                                 | Level | Attributes                                                        |
| --------------------------------------- | ----- | ----------------------------------------------------------------- |
| TLS-HTTP polling started                | INFO  | `ferron.tls_http.url` (string): certificate endpoint URL          |
| TLS-HTTP client config build failed     | WARN  | none                                                              |
| TLS-HTTP request build failed           | WARN  | `error.message` (string)                                          |
| TLS-HTTP request failed                 | WARN  | `error.message` (string)                                          |
| TLS-HTTP endpoint error                 | WARN  | `http.status_code` (int): HTTP status returned by endpoint        |
| TLS-HTTP response read failed           | WARN  | `error.message` (string)                                          |
| TLS-HTTP response parse failed          | WARN  | `error.message` (string)                                          |
| TLS-HTTP certificate chain parse failed | WARN  | `error.message` (string)                                          |
| TLS-HTTP private key parse failed       | WARN  | `error.message` (string)                                          |
| TLS-HTTP private key load failed        | WARN  | `error.message` (string)                                          |
| TLS-HTTP certificate refreshed          | INFO  | `ferron.tls_http.host` (string): hostname this certificate serves |
| On-demand certificate requested         | INFO  | `tls.sni` (string), `tls.port` (int)                              |
| On-demand certificate fetched           | INFO  | `tls.sni` (string), `tls.port` (int)                              |
| Certificate issuance denied             | ERROR | `tls.sni` (string): hostname blocked by ask endpoint              |
| Ask endpoint error                      | ERROR | `tls.sni` (string), `error.message` (string)                      |
| On-demand config not found              | ERROR | `tls.sni` (string), `tls.port` (int)                              |

### Metrics

| Metric                                         | Type      | Attributes                                                                        | Description                                     |
| ---------------------------------------------- | --------- | --------------------------------------------------------------------------------- | ----------------------------------------------- |
| `ferron.tls_http.requests_total`               | Counter   | `status` (`success`, `error`)                                                     | Total HTTP requests to the certificate endpoint |
| `ferron.tls_http.request_duration_seconds`     | Histogram | `status` (`success`, `error`)                                                     | HTTP request duration in seconds                |
| `ferron.tls_http.certificates_refreshed_total` | Counter   | `status` (`success`, `error`)                                                     | Certificate refresh outcomes                    |
| `ferron.tls_http.on_demand_requests_total`     | Counter   | None                                                                              | On-demand certificate requests                  |
| `ferron.tls.certificate_not_after`             | Gauge     | `ferron.host`, `ferron.tls.provider` (`http`), `crypto.certificate.serial_number` | Certificate `notAfter` as Unix epoch seconds    |
| `ferron.tls_http.next_refresh_seconds`         | Gauge     | None                                                                              | Seconds until next certificate refresh          |

All TLS providers share the certificate expiration gauge (manual, ACME, HTTP, and local). Ferron emits the gauge each time it mounts a certificate into the in-memory context.

## See also

- [Security and TLS](/docs/configuration/security/tls): cipher suites, ECDH curves, mTLS
- [ACME automatic TLS](/docs/configuration/security/acme): production TLS certificates via ACME
- [TLS session ticket keys](/docs/configuration/security/session-tickets): session resumption
- [OCSP stapling](/docs/configuration/security/ocsp): OCSP response stapling
- [HTTP host directives](/docs/configuration/server/host): per-host TLS configuration

## Best practices

`ferron doctor` reports the following best-practice checks for directives on this page.

- `url` with plain HTTP: Certificate endpoints that return private keys should use HTTPS with authentication.
- `no_verification` for the certificate endpoint: Disable TLS verification only when the endpoint is strictly internal and otherwise authenticated.
- `on_demand` without `on_demand_ask`: On-demand certificate fetching without an approval endpoint allows fetching for arbitrary hostnames. Configure `on_demand_ask` to approve requests.
- `on_demand_ask_no_verification`: Disable TLS verification for the approval endpoint only when the endpoint is strictly internal and otherwise authenticated.
