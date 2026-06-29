---
title: "Configuration: ACME automatic TLS"
description: "Automatic TLS certificate issuance via ACME, including HTTP-01, TLS-ALPN-01, and DNS-01 challenges."
---

This page documents the ACME TLS provider (`tls-acme` module), which automatically obtains TLS certificates from ACME-compatible Certificate Authorities (CAs) such as **Let's Encrypt**. It supports both **eager** (startup-time) and **on-demand** (lazy, first-connection) certificate issuance, with three challenge types:

- **HTTP-01** — serves a token at `/.well-known/acme-challenge/` (default)
- **TLS-ALPN-01** — responds with a self-signed cert during the TLS handshake
- **DNS-01** — creates a TXT record at `_acme-challenge.<domain>`

Certificates are **cached** (both in-memory and file-based) and **automatically renewed** before expiration.

Automatic TLS via ACME is enabled by default in Ferron for public hosts:

```ferron
example.com {
    # Automatic TLS is enabled by default, no explicit TLS directive needed
}
```

## Directives

### Challenge types

#### HTTP-01 (default)

The simplest challenge type. The server listens on port 80 to serve `/.well-known/acme-challenge/<token>`.

```ferron
example.com {
    tls {
        provider acme
        challenge http-01
        contact "admin@example.com"
    }
}
```

**Requirements:** The server must be reachable on port 80 for the ACME CA to validate the challenge.

#### TLS-ALPN-01

Responds with a self-signed certificate when the CA connects with the `acme-tls/1` ALPN protocol. No additional port is needed.

```ferron
example.com {
    tls {
        provider acme
        challenge tls-alpn-01
        contact "admin@example.com"
    }
}
```

**Requirements:** The server must be reachable on port 443. Does not support wildcard domains.

#### DNS-01 (required for wildcard domains)

Creates a `_acme-challenge` TXT record via a DNS provider. The only challenge type that supports wildcard certificates.

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01
        contact "admin@example.com"
        dns {
            provider cloudflare
            api_key "EXAMPLE_API_KEY"
        }
    }
}
```

**Requirements:** A DNS provider module must be configured. Wildcard domains are supported. The `dns` block must specify the `provider` name and any provider-specific credentials. See [DNS providers](/docs/v3/configuration/security/dns-providers) for the full list of supported providers and their directives.

### Configuration parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `provider` | `acme` | — | Must be set to `"acme"` |
| `challenge` | `http-01`, `tls-alpn-01`, `dns-01` | `http-01` | ACME challenge type |
| `contact` | `<string>` | — | Email for ACME account |
| `directory` | `<string>` | LE Production | ACME directory URL |
| `profile` | `<string>` | — | ACME profile name (optional) |
| `eab` | `"<key-id>" "<hmac>"` | — | External Account Binding |
| `cache` | `<string>` | `/var/cache/ferron-acme` if on Unix and writable, otherwise platform data dir | Path for file-based certificate caching |
| `save` | `<cert> [key]` | — | Save cert (and optionally key) to disk |
| `post_obtain_command` | `<string>` | — | Command to run after certificate issuance |
| `no_verification` | `<bool>` | `false` | Skip ACME directory TLS verification |
| `on_demand` | `<bool>` | `false` | Enable on-demand certificate issuance |
| `on_demand_ask` | `<string>` | — | Approval endpoint URL |
| `on_demand_ask_auth` | `<string>` | — | Authorization header for approval endpoint (optional) |
| `on_demand_ask_no_verification` | `<bool>` | `false` | Skip TLS verification for approval endpoint |
| `fallback` | `{ ... }` | — | Fallback provider block (repeatable) |

**Configuration example:**

```ferron
example.com {
    tls {
        provider acme
        challenge http-01
        contact "admin@example.com"

        directory "https://acme-v02.api.letsencrypt.org/directory"
        cache "/var/cache/ferron-acme"

        save "/etc/ssl/certs/example.com.pem" "/etc/ssl/private/example.com.pem"
        # `post_obtain_command` arg is a script/binary name + args, separated by spaces.
        post_obtain_command "/var/lib/post_obtain_command.sh"

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
*.example.com {
    tls {
        provider acme
        challenge dns-01
        contact "admin@example.com"
        on_demand
    }
}
```

### On-demand approval endpoint

To prevent abuse, you can configure an approval endpoint. Before issuing a certificate, Ferron sends an HTTP GET request to the endpoint with `?domain=<sni>` as a query parameter. If the response is `200`, the certificate is issued.

```ferron
*.example.com {
    tls {
        provider acme
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
example.com {
    tls {
        provider acme
        challenge http-01
        contact "admin@example.com"
        cache "/var/cache/ferron-acme"
    }
}
```

The cache directory structure:

```text
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
    provider acme
    challenge http-01
    contact "admin@example.com"
    eab "my-key-id" "SMq9KpHkR7z..."
    directory "https://acme.internal.example.com/directory"
}
```

The HMAC secret must be base64url-encoded (without padding).

## Fallback providers

When high availability is critical, you can configure fallback ACME providers. If the primary provider fails (e.g., the CA is down or unreachable), Ferron automatically tries the next configured fallback provider.

```ferron
example.com {
    tls {
        provider acme
        challenge http-01
        contact "admin@example.com"
        directory "https://acme-v02.api.letsencrypt.org/directory"

        fallback {
            directory "https://acme-staging-v02.api.letsencrypt.org/directory"
            contact "admin@example.com"
        }

        fallback {
            directory "https://other-ca.example.com/directory"
            eab "my-key-id" "SMq9KpHkR7z..."
            profile "my-profile"
        }
    }
}
```

Each `fallback` block accepts the same provider-level directives as the primary configuration: `directory`, `contact`, `eab`, and `profile`. The primary provider's settings are inherited — you only need to specify the fields that differ.

### How fallback works

- Providers are tried **sequentially**: the primary is attempted first, followed by each `fallback` block in order.
- A fallback is triggered when account creation with the previous provider fails.
- Once a provider succeeds, subsequent operations (order creation, challenge solving, certificate installation) use that same provider.
- Account cache keys are scoped per-provider, so each provider maintains its own cached credentials.

### Example: primary with staging fallback

A common pattern is to configure production as the primary and staging as a fallback for testing or when production is unavailable:

```ferron
example.com {
    tls {
        provider acme
        challenge http-01
        contact "admin@example.com"
        directory "https://acme-v02.api.letsencrypt.org/directory"

        fallback {
            directory "https://acme-staging-v02.api.letsencrypt.org/directory"
            contact "admin@example.com"
        }
    }
}
```

## Saving certificates to disk

To persist obtained certificates for use by other tools or backup:

```ferron
tls {
    provider acme
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

## Troubleshooting

### "ACME certificate provisioning error: ..."

Certificate issuance failed. The log message includes the affected domains. Check the error message for details (DNS resolution, ACME server errors, etc.). At debug log level (`--verbose`), you'll also see per-step messages for account loading, order creation, challenge solving, and certificate installation.

### DNS-01 issues

- Ensure the DNS provider is configured correctly with valid credentials.
- Check that the provider has permission to create TXT records for the domain.
- DNS propagation may take longer than 60 seconds for some providers — the ACME CA will retry validation.

### Verifying certificates

```bash
# Check the certificate served by Ferron
echo | openssl s_client -connect example.com -servername example.com 2>/dev/null | openssl x509 -noout -subject -dates -issuer

# Verify OCSP stapling
openssl s_client -connect example.com -status -servername example.com </dev/null 2>/dev/null | grep -A 5 "OCSP response"
```

## Observability

The ACME background task emits log events and metrics through the configured observability pipeline:

> [!note]
> The ACME log events are global-only, that means they won't be emitted via per-host observability sinks.

### Logs

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
| `DEBUG` | `ACME order created for domains: ...` | New order placed with CA |
| `DEBUG` | `ACME <type> challenge initiated for ...` | Challenge setup started |
| `DEBUG` | `ACME <type> challenge solved for ...` | Challenge ready for validation |
| `DEBUG` | `DNS-01 record created for _acme-challenge.<domain>, TTL <ttl>` | DNS record published |
| `DEBUG` | `DNS-01 record cleanup completed for _acme-challenge.<domain>` | DNS record removed |
| `DEBUG` | `Certificate installed for ..., chain length: N` | Certificate loaded into TLS config |

### Structured logs

In OTLP `log_style modern`, the `summary` field is used as the log body and `attributes` are emitted as typed OpenTelemetry log record attributes.

| Summary | Level | Attributes |
|---------|-------|------------|
| ACME background task started | INFO | `ferron.acme.config_count` (int), `ferron.acme.domains` (string) |
| ACME account created | INFO | `ferron.acme.directory` (string) — ACME directory URL, `ferron.acme.contact` (string) — account email |
| ACME certificate issued | INFO | `ferron.acme.domains` (string) |
| ACME post-obtain command started | INFO | `ferron.acme.domains` (string) |
| On-demand certificate pre-loaded | INFO | `tls.sni` (string), `tls.port` (int) |
| On-demand certificate requested | INFO | `tls.sni` (string), `tls.port` (int) |
| ACME account recreated | WARN | `ferron.acme.domains` (string), `ferron.acme.directory` (string) |
| ACME certificate provisioning error | WARN | `ferron.acme.domains` (string), `error.message` (string) |
| ACME post-obtain command malformed | WARN | `ferron.acme.domains` (string) |
| ACME post-obtain command failed | WARN | `ferron.acme.domains` (string), `error.message` (string) |
| ACME post-obtain command empty | WARN | `ferron.acme.domains` (string) |
| ACME account cache save failed | WARN | `error.message` (string) |
| ACME certificate cache save failed | WARN | `error.message` (string) |
| ACME provisioning cycle started | DEBUG | `ferron.acme.config_count` (int) |
| ACME account loaded from cache | DEBUG | `ferron.acme.domains` (string) |
| ACME order created | DEBUG | `ferron.acme.domains` (string) |
| ACME certificate installed | DEBUG | `ferron.acme.domains` (string), `ferron.acme.chain_length` (int) |
| ACME challenge initiated | DEBUG | `ferron.acme.domains` (string), `ferron.acme.challenge_type` (string) |
| ACME challenge solved | DEBUG | `ferron.acme.domains` (string), `ferron.acme.challenge_type` (string) |
| ACME DNS-01 record created | DEBUG | `ferron.acme.dns_challenge_domain` (string), `ferron.acme.dns_ttl` (int) |
| ACME DNS-01 record cleanup | DEBUG | `ferron.acme.dns_challenge_domain` (string) |
| ACME account load/create failed | ERROR | `ferron.acme.domains` (string), `error.message` (string) |
| ACME order creation failed | ERROR | `ferron.acme.domains` (string), `error.message` (string) |
| ACME authorization failed | ERROR | `ferron.acme.domains` (string), `ferron.acme.auth_status` (string) |
| ACME challenge type unsupported | ERROR | `ferron.acme.domains` (string), `ferron.acme.challenge_type` (string) |
| ACME identifier type unsupported | ERROR | `ferron.acme.domains` (string), `ferron.acme.identifier_type` (string) |
| ACME challenge ready failed | ERROR | `ferron.acme.domains` (string), `error.message` (string) |
| ACME order finalization failed | ERROR | `ferron.acme.domains` (string), `error.message` (string) |
| ACME order invalid | ERROR | `ferron.acme.domains` (string) |
| ACME order not ready | ERROR | `ferron.acme.domains` (string), `ferron.acme.order_status` (string) |
| ACME finalize failed | ERROR | `ferron.acme.domains` (string), `error.message` (string) |
| ACME certificate obtain failed | ERROR | `ferron.acme.domains` (string), `error.message` (string) |
| Certificate issuance denied | ERROR | `tls.sni` (string) — hostname blocked by ask endpoint |
| Ask endpoint error | ERROR | `tls.sni` (string), `error.message` (string) |

### Metrics

| Metric | Type | Attributes | Description |
|--------|------|--------|-------------|
| `ferron.acme.certificates_issued_total` | Counter | `status` (`success`, `error`), `challenge_type` | Certificate issuance outcomes |
| `ferron.acme.on_demand_requests_total` | Counter | — | On-demand certificate requests |
| `ferron.tls.certificate_not_after` | Gauge | `ferron.host`, `ferron.tls.provider` (`http`), `crypto.certificate.serial_number` | Certificate `notAfter` as Unix epoch seconds |

### Trace spans

The ACME HTTP-01 challenge stage sets the following attributes on its `ferron.stage.acme_http01` span:

| Attribute | Type | Description |
| --- | --- | --- |
| `ferron.acme.domain` | string | The domain being validated. |
| `ferron.acme.challenge_type` | string | Challenge type (`http01`). |

## See also

- [DNS providers](/docs/v3/configuration/security/dns-providers) — all supported DNS-01 provider backends and their configuration
- [Security and TLS](/docs/v3/configuration/security/tls) — cipher suites, ECDH curves, mTLS
- [TLS session ticket keys](/docs/v3/configuration/security/session-tickets) — session resumption
- [OCSP stapling](/docs/v3/configuration/security/ocsp) — OCSP response stapling
- [HTTP host directives](/docs/v3/configuration/server/host) — per-host TLS configuration

## Best practices

The following best-practice checks are reported by `ferron doctor` for directives on this page.

- **`no_verification` for ACME directory** — Disabling TLS verification for the ACME directory should only be used for testing.
- **`on_demand` without `on_demand_ask`** — On-demand certificate issuance without an approval endpoint allows certificate issuance for arbitrary hostnames. Configure `on_demand_ask` to approve requests.
- **`on_demand_ask_no_verification`** — Disabling TLS verification for the approval endpoint should only be used for strictly internal and otherwise authenticated endpoints.
- **Missing `contact`** — Without an ACME account email, the certificate authority cannot send expiry or account notices.
- **Non-public domain** — Domains using non-public TLDs (`.local`, `.internal`, `.home`, `.lan`, `.test`, `.localhost`) or bare IP addresses are unlikely to be publicly resolvable. Certificate issuance via ACME will fail because the CA cannot complete domain validation. This check applies to both explicit `domains` directives and to the host block's hostname when `on_demand` TLS is used.
