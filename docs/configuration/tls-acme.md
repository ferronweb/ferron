---
title: "Configuration: ACME automatic TLS"
description: "Automatic TLS certificate issuance via ACME, including HTTP-01, TLS-ALPN-01, and DNS-01 challenges."
---

This page documents the ACME TLS provider, which automatically obtains TLS certificates from ACME-compatible Certificate Authorities (CAs) such as **Let's Encrypt**. It supports both **eager** (startup-time) and **on-demand** (lazy, first-connection) certificate issuance, with three challenge types:

- **HTTP-01** — serves a token at `/.well-known/acme-challenge/` (default)
- **TLS-ALPN-01** — responds with a self-signed cert during the TLS handshake
- **DNS-01** — creates a TXT record at `_acme-challenge.<domain>`

Certificates are **cached** (both in-memory and file-based) and **automatically renewed** before expiration.

## Directives

### Challenge types

#### HTTP-01 (default)

The simplest challenge type. The server listens on port 80 to serve `/.well-known/acme-challenge/<token>`.

```ferron
example.com:443 {
    tls {
        provider "acme"
        challenge http-01
        contact "admin@example.com"
    }
}
```

**Requirements:** The server must be reachable on port 80 for the ACME CA to validate the challenge.

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

**Requirements:** The server must be reachable on port 443. Does not support wildcard domains.

#### DNS-01 (required for wildcard domains)

Creates a `_acme-challenge` TXT record via a DNS provider. The only challenge type that supports wildcard certificates.

> **Note:** No DNS provider modules are currently implemented. The DNS-01 challenge type is defined but requires a DNS provider module (e.g. Cloudflare, Route 53) to function. These modules are planned for a future release.

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

**Requirements:** A DNS provider module must be configured. Wildcard domains are supported.

### Configuration parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `provider` | `acme` | — | Must be set to `"acme"` |
| `challenge` | `http-01`, `tls-alpn-01`, `dns-01` | `http-01` | ACME challenge type |
| `contact` | `<string>` | — | Email for ACME account |
| `directory` | `<string>` | LE Production | ACME directory URL |
| `profile` | `<string>` | — | ACME profile name (optional) |
| `eab` | `"<key-id>" "<hmac>"` | — | External Account Binding |
| `cache` | `<string>` | platform data dir | Path for file-based certificate caching |
| `save` | `<cert> [key]` | — | Save cert (and optionally key) to disk |
| `post_obtain_command` | `<string>` | — | Command to run after certificate issuance |
| `no_verification` | `<bool>` | `false` | Skip ACME directory TLS verification |
| `on_demand` | `<bool>` | `false` | Enable on-demand certificate issuance |
| `on_demand_ask` | `<string>` | — | Approval endpoint URL |
| `on_demand_ask_no_verification` | `<bool>` | `false` | Skip TLS verification for approval endpoint |

**Configuration example:**

```ferron
example.com:443 {
    tls {
        provider "acme"
        challenge http-01
        contact "admin@example.com"

        directory "https://acme-v02.api.letsencrypt.org/directory"
        cache "/var/cache/ferron-acme"

        save "/etc/ssl/certs/example.com.pem" "/etc/ssl/private/example.com.pem"
        post_obtain_command "systemctl reload ferron"

        ocsp {
            enabled true
        }
    }
}
```

## Eager mode (recommended for known domains)

Eager mode obtains certificates at **server startup**, before any client traffic is received. This is ideal for static configurations where all domain names are known in advance.

## On-demand mode

On-demand mode defers certificate issuance until the **first TLS handshake** for a hostname. This is useful for wildcard domains, multi-tenant hosting, or when domains are not known at startup.

```ferron
*.example.com:443 {
    tls {
        provider "acme"
        challenge dns-01
        contact "admin@example.com"
        on_demand
    }
}
```

### On-demand approval endpoint

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

## Certificate caching

### In-memory cache (default)

When no `cache` path is specified, certificates and account data are stored in memory.

### File-based cache

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

### Cache key derivation

- **Account cache key**: hash of `contact emails + directory URL`
- **Certificate cache key**: hash of `sorted domains + profile name`

## Certificate renewal

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

## Saving certificates to disk

To persist obtained certificates for use by other tools or backup:

```ferron
tls {
    provider "acme"
    challenge http-01
    contact "admin@example.com"
    save "/etc/ssl/certs/example.com.pem" "/etc/ssl/private/example.com.pem"
}
```

If only one path is given, the key path defaults to the certificate path with a `.key` extension. After a certificate is obtained, the private key is written with `0600` permissions on Unix.

## Security considerations

- **Private keys are never logged** or exposed in error messages.
- When saved to disk, keys are written with `0600` permissions on Unix.
- When using on-demand mode, always configure an `on_demand_ask` endpoint in production to prevent certificate issuance for arbitrary hostnames.

## Notes and troubleshooting

### "ACME certificate provisioning error: ..."

Certificate issuance failed. The log message includes the affected domains. Check the error message for details (DNS resolution, ACME server errors, etc.). At debug log level (`--verbose`), you'll also see per-step messages for account loading, order creation, challenge solving, and certificate installation.

### DNS-01 issues

> **Note:** DNS provider modules are not yet implemented. The DNS-01 challenge is defined in the ACME module but has no available DNS provider backends. See the [Status and limitations](/docs/v3/status-and-limitations) page for details.

- Ensure the DNS provider is configured correctly with valid credentials.
- Check that the provider has permission to create TXT records for the domain.
- DNS propagation may take longer than 60 seconds for some providers — the ACME CA will retry validation.

### Observability

The ACME background task emits log events and metrics through the configured observability pipeline:

**Log events:**

| Level | Message | When |
|-------|---------|------|
| `INFO` | `ACME background task started with N configuration(s) for domains: ...` | Service initialization |
| `INFO` | `On-demand certificate requested for SNI <host>:<port>` | On-demand certificate request received |
| `INFO` | `ACME certificate issued for domains: ...` | Successful certificate issuance |
| `INFO` | `ACME account created for directory ..., contact: ...` | New ACME account registration |
| `INFO` | `Post-obtain command started for ...: <cmd>` | Post-obtain hook execution |
| `WARN` | `ACME certificate provisioning error for ...: <error>` | Certificate issuance failure |
| `WARN` | `ACME account not found on server for ..., recreating` | Account expired/removed on CA side |
| `WARN` | `Post-obtain command failed for ...: <error>` | Post-obtain hook error |
| `DEBUG` | `ACME provisioning cycle started — checking N configurations` | Each background loop iteration |
| `DEBUG` | `ACME account loaded from cache for ...` | Account reused from cache |
| `DEBUG` | `ACME certificate still valid or loaded from cache for ...` | No issuance needed |
| `DEBUG` | `ACME order created for domains: ...` | New order placed with CA |
| `DEBUG` | `ACME <type> challenge initiated for ...` | Challenge setup started |
| `DEBUG` | `ACME <type> challenge solved for ...` | Challenge ready for validation |
| `DEBUG` | `DNS-01 record created for _acme-challenge.<domain>, TTL <ttl>` | DNS record published |
| `DEBUG` | `DNS-01 record cleanup completed for _acme-challenge.<domain>` | DNS record removed |
| `DEBUG` | `Certificate installed for ..., chain length: N` | Certificate loaded into TLS config |

**Metrics:**

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `ferron.acme.certificates_issued_total` | Counter | `status` (`success`, `error`), `challenge_type` | Certificate issuance outcomes |
| `ferron.acme.on_demand_requests_total` | Counter | — | On-demand certificate requests |

### Verifying certificates

```bash
# Check the certificate served by Ferron
echo | openssl s_client -connect example.com:443 -servername example.com 2>/dev/null | openssl x509 -noout -subject -dates -issuer

# Verify OCSP stapling
openssl s_client -connect example.com:443 -status -servername example.com </dev/null 2>/dev/null | grep -A 5 "OCSP response"
```

## See also

- [Security and TLS](/docs/v3/configuration/security-tls) — cipher suites, ECDH curves, mTLS
- [TLS session ticket keys](/docs/v3/configuration/tls-session-tickets) — session resumption
- [OCSP stapling](/docs/v3/configuration/ocsp-stapling) — OCSP response stapling
- [HTTP host directives](/docs/v3/configuration/http-host) — per-host TLS configuration
