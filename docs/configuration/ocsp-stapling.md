---
title: "Configuration: OCSP stapling"
description: "OCSP stapling for TLS — attaching signed OCSP responses during the TLS handshake."
---

This page documents OCSP stapling configuration. OCSP stapling allows the TLS server to **attach a signed OCSP response** during the TLS handshake, eliminating the need for clients to contact the CA's OCSP responder directly. This improves:

- **Privacy** — clients no longer reveal their browsing habits to the CA
- **Performance** — eliminates the extra round-trip to the OCSP responder
- **Reliability** — works even when the CA's OCSP responder is unreachable

OCSP stapling works with all TLS providers (`manual`, `acme`, etc.).

## Default behavior (recommended)

OCSP stapling is **enabled by default**. No configuration is required:

```ferron
example.com {
    tls {
        provider manual
        cert "cert.pem"
        key "key.pem"
    }
}
```

The server will:
1. Extract the OCSP responder URL from the certificate's AIA extension
2. Fetch an OCSP response on startup
3. Cache and staple the response during TLS handshakes
4. Automatically refresh responses before they expire

## Explicit configuration

### Enable OCSP stapling

```ferron
example.com {
    tls {
        provider manual
        cert "cert.pem"
        key "key.pem"
        ocsp {
            enabled true
        }
    }
}
```

### Disable OCSP stapling

```ferron
example.com {
    tls {
        provider manual
        cert "cert.pem"
        key "key.pem"
        ocsp {
            enabled false
        }
    }
}
```

### Bare directive

A bare `ocsp` directive (without a nested block) also enables stapling, equivalent to the default:

```ferron
example.com {
    tls {
        provider manual
        cert "cert.pem"
        key "key.pem"
        ocsp
    }
}
```

### Configuration parameters

| Parameter | Type | Default | Required | Description |
|-----------|------|---------|----------|-------------|
| `enabled` | `<bool>` | `true` | No | Whether OCSP stapling is active |

## How it works

### Startup sequence

1. The OCSP module initializes a background service on the secondary tokio runtime
2. The TLS provider loads or obtains certificates
3. The certificate is **preloaded** into the OCSP service immediately
4. The background task fetches an OCSP response from the CA's responder
5. The response is cached and attached to subsequent TLS handshakes

### Refresh cycle

The background task maintains fresh OCSP responses:

1. **Initial fetch**: triggered by preloading on config load
2. **Safety margin**: responses are refreshed before expiry (25% of validity period)
3. **Jitter**: randomized delay (up to 5 minutes) prevents refresh storms
4. **Error handling**: failed fetches are retried with exponential backoff

### OCSP Must-Staple

Certificates with the **OCSP Must-Staple** extension (TLS Feature `status_request`, RFC 7633) are automatically detected. Must-Staple certificates **require** a stapled OCSP response — clients that enforce Must-Staple will reject connections without one. Preloading ensures the response is fetched immediately on startup.

## OCSP responder URL

The responder URL is extracted from the certificate's **Authority Information Access (AIA)** extension. Most CA-issued certificates include this automatically.

If no OCSP URL is found in the certificate, OCSP stapling is silently skipped for that certificate (no error is raised).

## Security considerations

- OCSP requests are sent over **HTTPS only**. The HTTPS client uses the webpki root certificate store for server verification.
- If the OCSP responder is unreachable, the last cached response is kept and used until a new one is fetched. However, the service does not serve responses past their `nextUpdate` time — it will keep retrying until a fresh response is obtained.

## Notes and troubleshooting

### "OCSP fetch failed: ..."

The OCSP responder returned an error or was unreachable. The service will retry with jitter. The log message includes the certificate's subject common name (or a SPKI hash prefix if the CN is unavailable) to help identify which certificate is affected. Common causes:

- Network issues
- CA's OCSP responder is down
- Certificate has no OCSP URL in AIA extension

### Observability

The OCSP background task emits log events and metrics through the configured observability pipeline:

**Log events:**

| Level | Message | When |
|-------|---------|------|
| `INFO` | `OCSP background task started` | Service initialization |
| `INFO` | `OCSP background task shutting down` | Graceful shutdown |
| `DEBUG` | `OCSP fetch triggered for certificate <ident>` | Certificate preloaded into service |
| `DEBUG` | `OCSP response cached for <ident>, valid until <time>` | Successful OCSP fetch |
| `DEBUG` | `OCSP stapling skipped — no OCSP URL in certificate <ident>` | Certificate lacks OCSP URL |
| `WARN` | `OCSP fetch failed for <ident>: <error>` | Fetch error (retried with jitter) |

**Metrics:**

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `ferron.ocsp.fetches_total` | Counter | `status` (`success`, `error`, `skipped`) | Total OCSP fetch attempts |
| `ferron.ocsp.fetch_duration_seconds` | Histogram | — | Time to fetch OCSP response |
| `ferron.ocsp.cached_certificates` | Gauge | — | Number of certificates tracked |
| `ferron.ocsp.certificates_with_stapling` | Gauge | — | Certificates with valid stapled responses |

### Verifying stapling

Use OpenSSL to verify that OCSP stapling is working:

```bash
openssl s_client -connect example.com:443 -status -servername example.com </dev/null 2>/dev/null | grep -A 20 "OCSP response"
```

You should see a `OCSP Response Status: successful` in the output.

## Integration with config reload

On configuration reload (SIGHUP or file change), the OCSP service is reused (not recreated), the TLS provider re-executes and preloads the new certificates, and the background task picks up new certificates from the channel. Zero downtime — old connections continue with old responses.

## See also

- [Security and TLS](/docs/v3/configuration/security-tls) — cipher suites, ECDH curves, mTLS
- [ACME automatic TLS](/docs/v3/configuration/tls-acme) — OCSP stapling with ACME-obtained certificates
- [TLS session ticket keys](/docs/v3/configuration/tls-session-tickets)
