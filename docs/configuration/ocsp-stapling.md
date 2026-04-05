# OCSP Stapling

## Overview

OCSP stapling allows the TLS server to **attach a signed OCSP response** during the TLS handshake, eliminating the need for clients to contact the CA's OCSP responder directly. This improves:

- **Privacy**: Clients no longer reveal their browsing habits to the CA
- **Performance**: Eliminates the extra round-trip to the OCSP responder
- **Reliability**: Works even when the CA's OCSP responder is unreachable

In Titanium, OCSP stapling is managed through a dedicated background service that fetches and caches OCSP responses over HTTPS. It works with all TLS providers (`manual`, `acme`, etc.).

For the `manual` provider, certificates are **preloaded** when the configuration is loaded. For the `acme` provider, certificates are preloaded as soon as they are obtained from the ACME CA.

## Configuration

### Default Behavior (Recommended)

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

### Explicit Configuration

#### Enable OCSP Stapling

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

#### Disable OCSP Stapling

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

#### Bare Directive

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

### Configuration Parameters

| Parameter | Type | Default | Required | Description |
|-----------|------|---------|----------|-------------|
| `enabled` | bool | `true` | No | Whether OCSP stapling is active |

## How It Works

### Startup Sequence

1. The OCSP module initializes a background service on the secondary tokio runtime
2. The TLS provider (manual, acme, etc.) loads or obtains certificates
3. The provider wraps its certificate resolver with `OcspStapler`
4. The certificate is **preloaded** into the OCSP service immediately
   - For `manual`: preloaded on config load
   - For `acme`: preloaded as soon as the certificate is obtained
5. The background task fetches an OCSP response from the CA's responder
6. The response is cached and attached to subsequent TLS handshakes

### Refresh Cycle

The background task maintains fresh OCSP responses:

1. **Initial fetch**: Triggered by preloading on config load
2. **Safety margin**: Responses are refreshed before expiry (25% of validity period)
3. **Jitter**: Randomized delay (up to 5 minutes) prevents refresh storms
4. **Error handling**: Failed fetches are retried with exponential backoff

### OCSP Must-Staple

Certificates with the **OCSP Must-Staple** extension (TLS Feature `status_request`, RFC 7633) are automatically detected. A log message is emitted:

```
INFO OCSP stapling enabled — Must-Staple detected, preloading certificate
```

Must-Staple certificates **require** a stapled OCSP response — clients that enforce Must-Staple will reject connections without one. Preloading ensures the response is fetched immediately on startup.

## OCSP Responder URL

The responder URL is extracted from the certificate's **Authority Information Access (AIA)** extension. Most CA-issued certificates include this automatically.

If no OCSP URL is found in the certificate, OCSP stapling is silently skipped for that certificate (no error is raised).

## Architecture

### Service Lifecycle

```
OcspStaplerModuleLoader
    └─ registers OcspStaplerModule
         └─ Module::start()
              └─ init_ocsp_service(&runtime)
                   └─ spawns background_ocsp_task() on secondary tokio runtime
```

### Data Flow

```
config load
    └─ provider::execute()
         └─ load_certs() / load_private_key()
              └─ get_service_handle() → OcspServiceHandle
                   └─ build_certified_key() → CertifiedKey
                        └─ ocsp_handle.preload(certified_key)
                             └─ send to background task channel
                                  └─ extract_ocsp_url(cert)
                                       └─ create_ocsp_request()
                                            └─ HTTPS POST to OCSP responder
                                                 └─ cache response
                                                      └─ OcspStapler::resolve() attaches it
```

### Key Design Decisions

- **Single background task**: One service for the entire process, shared by all TLS providers
- **Eager channel creation**: The sender channel is created on first access, so certs can be queued before the background task spawns
- **parking_lot::RwLock**: Cache uses `parking_lot::RwLock` for safe access from both vibeio (primary) and tokio (secondary) runtimes
- **SHA-256 with SHA-1 fallback**: OCSP requests use SHA-256 hashes first, falling back to SHA-1 for compatibility

## Security Considerations

### HTTPS Only

OCSP requests are sent over **HTTPS only**. The HTTPS client uses the webpki root certificate store for server verification.

### No Stale Responses by Default

If the OCSP responder is unreachable, the last cached response is kept and used until a new one is fetched. However, the service does not serve responses past their `nextUpdate` time — it will keep retrying until a fresh response is obtained.

### Certificate Privacy

The certificate chain (including the leaf certificate) is sent to the CA's OCSP responder. This is inherent to the OCSP protocol and is not a Ferron-specific concern.

## Troubleshooting

### Common Log Messages

#### "OCSP stapling service initialized"

The background service started successfully.

#### "OCSP stapling enabled for this TLS configuration"

OCSP stapling was enabled for this host's TLS configuration.

#### "OCSP stapling enabled — Must-Staple detected, preloading certificate"

The leaf certificate has the OCSP Must-Staple extension. The certificate was preloaded for immediate fetching.

#### "OCSP fetch failed: ..."

The OCSP responder returned an error or was unreachable. The service will retry with jitter. Common causes:

- Network issues
- CA's OCSP responder is down
- Certificate has no OCSP URL in AIA extension

### Verifying Stapling

Use OpenSSL to verify that OCSP stapling is working:

```bash
openssl s_client -connect example.com:443 -status -servername example.com </dev/null 2>/dev/null | grep -A 20 "OCSP response"
```

You should see a `OCSP Response Status: successful` in the output.

## Integration with Config Reload

On configuration reload (SIGHUP or file change):

1. The OCSP service is reused (not recreated) via `AlreadyInitialized` check
2. The TLS provider re-executes and preloads the new certificates
   - For `manual`: new certificates are loaded from disk
   - For `acme`: renewed certificates are obtained from the ACME CA
3. The background task picks up new certificates from the channel
4. OCSP responses are fetched for the new certificates
5. Zero downtime — old connections continue with old responses

## References

- [RFC 6960: X.509 Internet Public Key Infrastructure Online Certificate Status Protocol](https://tools.ietf.org/html/rfc6960)
- [RFC 7633: X.509v3 Transport Layer Security (TLS) Feature Extension](https://tools.ietf.org/html/rfc7633)
- [OCSP Stapling Explained](https://sslinsights.com/ocsp-stapling-explained/)
- [ACME Automatic TLS](./tls-acme.md) — OCSP stapling with ACME-obtained certificates
