---
title: "Configuration: OCSP stapling"
description: OCSP stapling for TLS. Attaches signed OCSP responses during the TLS handshake.
---

This page documents OCSP stapling configuration (`ocsp-stapler` module). OCSP stapling allows the TLS server to attach a signed OCSP response during the TLS handshake. This eliminates the need for clients to contact the CA OCSP responder directly. This improves:

- **Privacy**. Clients no longer reveal their browsing habits to the CA.
- **Performance**. Eliminates the extra round-trip to the OCSP responder.
- **Reliability**. Works even when the CA OCSP responder is unreachable.

OCSP stapling works with all TLS providers (`manual`, `acme`, and so on).

## Default behavior (recommended)

OCSP stapling is enabled by default. You do not need any configuration.

```ferron
example.com {
    tls cert.pem key.pem
}
```

The server will:

1. Extract the OCSP responder URL from the AIA extension of the certificate
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
        ocsp
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
        ocsp false
    }
}
```

## OCSP responder URL

The responder URL comes from the Authority Information Access (AIA) extension of the certificate. Most CA-issued certificates include this automatically.

If the certificate has no OCSP URL, OCSP stapling is silently skipped for that certificate. The server does not raise an error.

> [!tip]
> The module automatically detects certificates with the OCSP Must-Staple extension (TLS Feature `status_request`, RFC 7633). Must-Staple certificates require a stapled OCSP response. Clients that enforce Must-Staple will reject connections without one. Preloading makes sure the service fetches the response immediately on startup.

## Troubleshooting

### "OCSP fetch failed: ..."

The OCSP responder returned an error or was unreachable. The service will retry with jitter. The log message includes the common name of the certificate subject. If the CN is unavailable, the message includes a SPKI hash prefix instead. This helps identify which certificate has the issue. Common causes:

- Network issues
- CA OCSP responder is down
- Certificate has no OCSP URL in AIA extension

### Verifying stapling

Use OpenSSL to verify that OCSP stapling works:

```bash
openssl s_client -connect example.com:443 -status -servername example.com </dev/null 2>/dev/null | grep -A 20 "OCSP response"
```

You should see a `OCSP Response Status: successful` in the output.

## Observability

The OCSP background task emits log events and metrics through the configured observability pipeline:

### Logs

| Level   | Message                                                                | When                               |
| ------- | ---------------------------------------------------------------------- | ---------------------------------- |
| `INFO`  | `OCSP response cached for <ident> (<primary_san>), valid until <time>` | Successful OCSP fetch              |
| `DEBUG` | `OCSP fetch triggered for certificate <ident>`                         | Certificate preloaded into service |
| `DEBUG` | `OCSP stapling skipped — no OCSP URL in certificate <ident>`           | Certificate lacks OCSP URL         |
| `WARN`  | `OCSP fetch failed for <ident>: <error>`                               | Fetch error (retried with jitter)  |

### Structured logs

In OTLP `log_style modern`, the `summary` field is the log body. The system types `attributes` as OpenTelemetry log record attributes.

| Summary                          | Level | Attributes                                                                                                                                              |
| -------------------------------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| OCSP HTTPS initialization failed | INFO  | none                                                                                                                                                    |
| OCSP response cached             | INFO  | `ferron.ocsp.cert.subject` (string), `ferron.ocsp.next_update` (int): Unix timestamp of next update, `ferron.ocsp.cert.primary_san` (string): first SAN |
| OCSP fetch triggered             | DEBUG | `ferron.ocsp.cert.subject` (string): certificate subject                                                                                                |
| OCSP stapling skipped            | DEBUG | `ferron.ocsp.cert.subject` (string), `ferron.ocsp.reason` (string): reason for skipping                                                                 |
| OCSP fetch failed                | WARN  | `ferron.ocsp.cert.subject` (string), `error.message` (string)                                                                                           |

### Metrics

| Metric                                   | Type      | Attributes                                                          | Description                               |
| ---------------------------------------- | --------- | ------------------------------------------------------------------- | ----------------------------------------- |
| `ferron.ocsp.fetches_total`              | Counter   | `ferron.ocsp.status` (`success`, `error`, `skipped`), `ferron.host` | Total OCSP fetch attempts per host        |
| `ferron.ocsp.fetch_duration_seconds`     | Histogram | `ferron.host`                                                       | Time to fetch OCSP response               |
| `ferron.ocsp.stapling.hit_total`         | Counter   | `ferron.host`                                                       | OCSP responses served to clients per host |
| `ferron.ocsp.cached_certificates`        | Gauge     | None                                                                | Number of certificates tracked            |
| `ferron.ocsp.certificates_with_stapling` | Gauge     | None                                                                | Certificates with valid stapled responses |

## See also

- [Security and TLS](/docs/v3/configuration/security/tls). Cipher suites, ECDH curves, mTLS.
- [ACME automatic TLS](/docs/v3/configuration/security/acme). OCSP stapling with ACME-obtained certificates.
- [TLS session ticket keys](/docs/v3/configuration/security/session-tickets)
