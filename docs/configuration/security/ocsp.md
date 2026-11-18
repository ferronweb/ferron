---
title: "Configuration: OCSP stapling"
description: "Attach signed OCSP responses to the TLS handshake."
---

OCSP stapling attaches a signed OCSP response to the TLS handshake. Clients then do not contact the CA responder themselves. Ferron fetches responses in the background and refreshes them before they expire. Stapling works with every TLS provider (`manual`, `acme`, `http`, and `local`).

Clients learn revocation status without a separate request, so:

- Their browsing habits stay private from the CA.
- Handshakes avoid an extra round-trip to the responder.
- Validation still works when the responder is unreachable.

## Enable or disable stapling

Stapling is enabled by default. You do not need to configure it.

| Directive    | Effect                    |
| ------------ | ------------------------- |
| `ocsp`       | Enable stapling (default) |
| `ocsp false` | Disable stapling          |

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

At startup, Ferron reads the OCSP responder URL from the certificate's Authority Information Access (AIA) extension, fetches a response, caches it, and staples it during handshakes. It refreshes responses before they expire. If the certificate has no OCSP URL, Ferron skips stapling for that certificate without an error.

> [!warning]
> A certificate with the OCSP Must-Staple extension (TLS Feature `status_request`, RFC 7633) requires a stapled response. Clients that enforce Must-Staple reject connections without one. Keep stapling enabled for these certificates. Ferron fetches the response at startup, so no extra setting is needed.

## Verify stapling

```bash
openssl s_client -connect example.com:443 -status -servername example.com </dev/null 2>/dev/null | grep -A 20 "OCSP response"
```

The output should include `OCSP Response Status: successful`.

## Observability

OCSP emits log events and metrics through the configured observability pipeline.

> [!note]
> When a fetch fails, Ferron retries with jitter. `OCSP fetch failed` names the certificate common name, or a SPKI hash prefix when the CN is unavailable. Common causes are network problems, an unavailable responder, or a certificate without an OCSP URL in its AIA extension.

### Logs

| Level   | Message                                                                | When                               |
| ------- | ---------------------------------------------------------------------- | ---------------------------------- |
| `INFO`  | `OCSP response cached for <ident> (<primary_san>), valid until <time>` | Successful OCSP fetch              |
| `DEBUG` | `OCSP fetch triggered for certificate <ident>`                         | Certificate preloaded into service |
| `DEBUG` | `OCSP stapling skipped — no OCSP URL in certificate <ident>`           | Certificate lacks OCSP URL         |
| `WARN`  | `OCSP fetch failed for <ident>: <error>`                               | Fetch error (retried with jitter)  |

### Structured logs

In OTLP `log_style modern`, the `summary` field is the log body. The `attributes` are typed OpenTelemetry log record attributes.

| Summary                          | Level | Attributes                                                                                                                                                                                                                                 |
| -------------------------------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| OCSP HTTPS initialization failed | INFO  | none                                                                                                                                                                                                                                       |
| OCSP response cached             | INFO  | `ferron.ocsp.cert.subject` (string), `ferron.ocsp.next_update` (int): Unix timestamp of next update, `ferron.ocsp.cert.primary_san` (string): first SAN, `ferron.ocsp.cert.status` (`good`, `revoked`, `unknown`): certificate OCSP status |
| OCSP fetch triggered             | DEBUG | `ferron.ocsp.cert.subject` (string): certificate subject                                                                                                                                                                                   |
| OCSP stapling skipped            | DEBUG | `ferron.ocsp.cert.subject` (string), `ferron.ocsp.reason` (string): reason for skipping                                                                                                                                                    |
| OCSP fetch failed                | WARN  | `ferron.ocsp.cert.subject` (string), `error.message` (string)                                                                                                                                                                              |

### Metrics

| Metric                                   | Type      | Attributes                                                                                                                    | Description                               |
| ---------------------------------------- | --------- | ----------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------- |
| `ferron.ocsp.fetches_total`              | Counter   | `ferron.ocsp.status` (`success`, `error`, `skipped`), `ferron.host`, `ferron.ocsp.cert.status` (`good`, `revoked`, `unknown`) | Total OCSP fetch attempts per host        |
| `ferron.ocsp.fetch_duration_seconds`     | Histogram | `ferron.host`                                                                                                                 | Time to fetch OCSP response               |
| `ferron.ocsp.stapling.hit_total`         | Counter   | `ferron.host`                                                                                                                 | OCSP responses served to clients per host |
| `ferron.ocsp.cached_certificates`        | Gauge     | None                                                                                                                          | Number of certificates tracked            |
| `ferron.ocsp.certificates_with_stapling` | Gauge     | None                                                                                                                          | Certificates with valid stapled responses |

## See also

- [Security and TLS](/docs/configuration/security/tls). Cipher suites, ECDH curves, mTLS.
- [ACME automatic TLS](/docs/configuration/security/acme). OCSP stapling with ACME-obtained certificates.
- [TLS session ticket keys](/docs/configuration/security/session-tickets)
