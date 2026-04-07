# ACME Automatic TLS

## Overview

The `acme` TLS provider automatically obtains TLS certificates from ACME-compatible Certificate Authorities (CAs) such as **Let's Encrypt**. It supports both **eager** (startup-time) and **on-demand** (lazy, first-connection) certificate issuance, with three challenge types:

- **HTTP-01** — Serves a token at `/.well-known/acme-challenge/` (default)
- **TLS-ALPN-01** — Responds with a self-signed cert during the TLS handshake
- **DNS-01** — Creates a TXT record at `_acme-challenge.<domain>`

Certificates are **cached** (both in-memory and file-based) and **automatically renewed** before expiration. Account credentials are also cached to avoid unnecessary re-registration.

## Eager Mode (Recommended for Known Domains)

Eager mode obtains certificates at **server startup**, before any client traffic is received. This is ideal for static configurations where all domain names are known in advance.

```ferron
example.com:443 {
    tls {
        provider "acme"
        challenge http-01
        contact "admin@example.com"
    }
    root "/var/www/example.com"
    file_server
}
```

### Challenge Types

#### HTTP-01 (Default)

The simplest challenge type. The server listens on port 80 (or shares the existing HTTP listener) to serve `/.well-known/acme-challenge/<token>`.

```ferron
example.com:443 {
    tls {
        provider "acme"
        challenge http-01
        contact "admin@example.com"
    }
}
```

**Requirements**: The server must be reachable on port 80 for the ACME CA to validate the challenge.

#### TLS-ALPN-01

Responds with a self-signed certificate when the CA connects with the `acme-tls/1` ALPN protocol. No additional port is needed.

```ferron
example.com:443 {
    tls {
        provider "acme"
        challenge tls-alpn-01
        contact "admin@example.com"
    }
}
```

**Requirements**: The server must be reachable on port 443. Does not support wildcard domains.

#### DNS-01 (Required for Wildcard Domains)

Creates a `_acme-challenge` TXT record via a DNS provider. The only challenge type that supports wildcard certificates.

```ferron
*.example.com:443 {
    tls {
        provider "acme"
        challenge dns-01
        contact "admin@example.com"
        dns "cloudflare" {
            api_key "EXAMPLE_API_KEY"
        }
    }
}
```

**Requirements**: A DNS provider module must be configured (e.g., `cloudflare`, `route53`, etc.). Wildcard domains are supported.

### Complete Eager Configuration

```ferron
example.com:443 {
    tls {
        provider "acme"
        challenge http-01
        contact "admin@example.com"

        # ACME directory URL (default: Let's Encrypt Production)
        directory "https://acme-v02.api.letsencrypt.org/directory"

        # ACME profile (optional)
        # profile "preferred"

        # External Account Binding (for CAs that require it)
        # eab "key-id" "hmac-base64-secret"

        # File-based cache (default: platform data directory)
        cache "/var/cache/ferron-acme"

        # Save obtained certificates to disk (optional)
        save "/etc/ssl/certs/example.com.pem" "/etc/ssl/private/example.com.pem"

        # Run a command after certificate issuance (optional)
        # Environment variables: FERRON_ACME_DOMAIN, FERRON_ACME_CERT_PATH, FERRON_ACME_KEY_PATH
        post_obtain_command "systemctl reload ferron"

        # Disable ACME directory certificate verification (testing only)
        # no_verification false

        # OCSP stapling (enabled by default)
        ocsp {
            enabled true
        }

        # TLS session ticket keys (optional)
        # ticket_keys {
        #     file "/etc/ferron/session_tickets.keys"
        #     auto_rotate true
        #     rotation_interval "12h"
        #     max_keys 3
        # }
    }
    root "/var/www/example.com"
    file_server
}
```

## On-Demand Mode

On-demand mode defers certificate issuance until the **first TLS handshake** for a hostname. This is useful for wildcard domains, multi-tenant hosting, or when domains are not known at startup.

```ferron
*.example.com:443 {
    tls {
        provider "acme"
        challenge dns-01
        contact "admin@example.com"
        dns "cloudflare" {
            api_key "EXAMPLE_API_KEY"
        }
        on_demand
    }
    root "/var/www/multi-tenant"
    file_server
}
```

### On-Demand Approval Endpoint

To prevent abuse, you can configure an approval endpoint. Before issuing a certificate, Ferron sends an HTTP GET request to the endpoint with `?domain=<sni>` as a query parameter. If the response is `200`, the certificate is issued.

```ferron
*.example.com:443 {
    tls {
        provider "acme"
        challenge http-01
        contact "admin@example.com"
        on_demand
        on_demand_ask "https://internal-api.example.com/check-cert"
    }
}
```

To skip TLS verification for the approval endpoint (e.g., for self-signed internal services):

```ferron
        on_demand_ask "https://internal-api.example.com/check-cert"
        on_demand_ask_no_verification true
```

## Configuration Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `provider` | `acme` | — | Must be set to `"acme"` |
| `challenge` | `http-01`, `tls-alpn-01`, `dns-01` | `http-01` | ACME challenge type |
| `contact` | string | — | Email for ACME account (e.g., `admin@example.com`) |
| `directory` | string | LE Production | ACME directory URL |
| `profile` | string | — | ACME profile name (optional) |
| `eab` | `"<key-id>" "<hmac>"` | — | External Account Binding |
| `dns_provider` | string | — | DNS provider name (required for `dns-01`) |
| `cache` | string | platform data dir | Path for file-based certificate caching |
| `save` | `<cert> [key]` | — | Save cert (and optionally key) to disk |
| `post_obtain_command` | string | — | Command to run after certificate issuance |
| `no_verification` | bool | `false` | Skip ACME directory TLS verification |
| `on_demand` | bool | `false` | Enable on-demand certificate issuance |
| `on_demand_ask` | string | — | Approval endpoint URL |
| `on_demand_ask_no_verification` | bool | `false` | Skip TLS verification for approval endpoint |
| `ocsp` | `{ ... }` | Enabled | OCSP stapling configuration |
| `ticket_keys` | `{ ... }` | Default ticketer | Session ticket key management |

## Certificate Caching

### In-Memory Cache (Default)

When no `cache` path is specified, certificates and account data are stored in memory. This is sufficient for eager mode with static configurations.

### File-Based Cache

Setting a `cache` path persists certificates and accounts to disk, surviving restarts:

```ferron
tls {
    provider "acme"
    challenge http-01
    contact "admin@example.com"
    cache "/var/cache/ferron-acme"
}
```

The cache directory structure:
```
/var/cache/ferron-acme/
├── account_<hash>          # ACME account credentials
└── certificate_<hash>      # Certificate chain + private key (JSON)
```

Certificate files contain the full chain in PEM format. Private keys are stored alongside the certificate.

### Cache Key Derivation

- **Account cache key**: Hash of `contact emails + directory URL`
- **Certificate cache key**: Hash of `sorted domains + profile name`

This ensures the same account is reused across domains with the same CA, and different domain sets get separate certificates.

## Certificate Renewal

Certificates are automatically renewed before expiration. The renewal check runs every **10 seconds** in the background. Ferron uses the ACME `renewalInfo` endpoint (RFC 9773) when available, falling back to a heuristic of 50% of certificate lifetime (capped at 24 hours before expiry).

## External Account Binding (EAB)

Some CAs (especially enterprise/internal ACME servers) require External Account Binding. Provide the key ID and HMAC secret:

```ferron
tls {
    provider "acme"
    challenge http-01
    contact "admin@example.com"
    eab "my-key-id" "SMq9KpHkR7z..."
    directory "https://acme.internal.example.com/directory"
}
```

The HMAC secret must be base64url-encoded (without padding).

## Saving Certificates to Disk

To persist obtained certificates for use by other tools or backup:

```ferron
tls {
    provider "acme"
    challenge http-01
    contact "admin@example.com"
    save "/etc/ssl/certs/example.com.pem" "/etc/ssl/private/example.com.pem"
}
```

If only one path is given, the key path defaults to the certificate path with a `.key` extension. After a certificate is obtained:

1. The certificate chain is written to the cert path
2. The private key is written to the key path (with `0600` permissions on Unix)
3. If `post_obtain_command` is set, it is executed with environment variables:
   - `FERRON_ACME_DOMAIN` — comma-separated domain list
   - `FERRON_ACME_CERT_PATH` — path to the certificate file
   - `FERRON_ACME_KEY_PATH` — path to the private key file

Example post-obtain command:

```ferron
post_obtain_command "systemctl reload ferron"
```

## OCSP Stapling

OCSP stapling is **enabled by default** for ACME-obtained certificates. When a new certificate is obtained, it is automatically **preloaded** into the OCSP service, which fetches and caches the OCSP response from the CA's responder.

```ferron
tls {
    provider "acme"
    challenge http-01
    contact "admin@example.com"
    ocsp {
        enabled true
    }
}
```

To disable:

```ferron
    ocsp {
        enabled false
    }
```

See [OCSP Stapling](./ocsp-stapling.md) for details.

## TLS Session Ticket Keys

Session ticket key management is fully supported with the ACME provider:

```ferron
tls {
    provider "acme"
    challenge http-01
    contact "admin@example.com"
    ticket_keys {
        file "/etc/ferron/session_tickets.keys"
        auto_rotate true
        rotation_interval "12h"
        max_keys 3
    }
}
```

See [TLS Session Ticket Keys](./tls-session-tickets.md) for details.

## ACME Directory Verification

By default, Ferron verifies the ACME server's TLS certificate using the system root certificate store (with webpki-roots as fallback). For testing environments or internal CAs with non-public certificates, disable verification:

```ferron
tls {
    provider "acme"
    challenge http-01
    contact "admin@example.com"
    directory "https://pebble:14000/dir"
    no_verification true
}
```

**Warning**: This should only be used in testing or trusted internal networks.

## Architecture

### Background Task

The ACME background task runs on the **secondary Tokio runtime**:

1. **Eager configs**: Provisioned at startup, renewed every 10 seconds
2. **On-demand requests**: Received via async channel when a new SNI hostname is encountered
3. **On-demand conversion**: On-demand configs are converted to eager configs when a new hostname is requested

### HTTP-01 Challenge Server

The HTTP-01 challenge is handled by a **pipeline stage** (`AcmeHttp01ChallengeStage`) that intercepts requests to `/.well-known/acme-challenge/` early in the HTTP pipeline, before other request handlers. The challenge token is looked up from the shared ACME task state.

### TLS-ALPN-01 Challenge

The `TcpTlsAcmeResolver::handshake()` method uses `LazyConfigAcceptor` to inspect the `ClientHello` before committing to a `ServerConfig`. If the `acme-tls/1` ALPN is detected, the resolver looks up the matching self-signed challenge certificate from the shared TLS-ALPN-01 locks.

### DNS-01 Challenge

The `DnsProvider` trait (from the DNS provider registry) is used to create and remove `_acme-challenge` TXT records. After the ACME order is ready, the TXT records are cleaned up.

## Security Considerations

### Private Key Protection

- Private keys are never logged or exposed in error messages
- When saved to disk, keys are written with `0600` permissions on Unix
- Cached certificate files (including private keys) are stored with `0600` permissions

### ACME Account Security

- Account keys are cached separately from certificate keys
- The same account is reused for all domains with the same ACME directory URL
- EAB keys are only used during initial account creation

### On-Demand Abuse Prevention

When using on-demand mode, always configure an `on_demand_ask` endpoint in production to prevent certificate issuance for arbitrary hostnames.

## Troubleshooting

### Common Log Messages

#### "ACME background task started with X configuration(s)"

The background provisioning loop has started.

#### "ACME certificate provisioning error: ..."

Certificate issuance failed. Check the error message for details (DNS resolution, ACME server errors, etc.).

#### "TLS-ALPN-01 challenge requested for unknown domain"

The ACME CA sent a TLS-ALPN-01 challenge for a domain that doesn't match any pending order. This is typically a transient issue during challenge validation.

#### "Error during TLS handshake: ..."

A TLS handshake error occurred. This is logged at warn level and the connection is closed.

### Verifying Certificates

```bash
# Check the certificate served by Ferron
echo | openssl s_client -connect example.com:443 -servername example.com 2>/dev/null | openssl x509 -noout -subject -dates -issuer

# Verify OCSP stapling
openssl s_client -connect example.com:443 -status -servername example.com </dev/null 2>/dev/null | grep -A 5 "OCSP response"

# Check HTTP-01 challenge endpoint
curl -I http://example.com/.well-known/acme-challenge/test-token
```

### DNS-01 Issues

- Ensure the DNS provider is configured correctly with valid credentials
- Check that the provider has permission to create TXT records for the domain
- DNS propagation may take longer than 60 seconds for some providers — the ACME CA will retry validation

## References

- [RFC 8555: Automatic Certificate Management Environment (ACME)](https://tools.ietf.org/html/rfc8555)
- [RFC 8737: TLS Application-Layer Protocol Negotiation Extension](https://tools.ietf.org/html/rfc8737)
- [RFC 8738: ACME DNS Challenge Extension](https://tools.ietf.org/html/rfc8738)
- [RFC 9773: ACME Renewal Information (ARI) Extension](https://tools.ietf.org/html/rfc9773)
- [Let's Encrypt Documentation](https://letsencrypt.org/docs/)
- [instant-acme crate](https://docs.rs/instant-acme/)
