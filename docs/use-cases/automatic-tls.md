---
title: Automatic TLS
description: "Set up automatic TLS in Ferron with Let's Encrypt and ACME challenges (HTTP-01, TLS-ALPN-01, DNS-01)."
---

Ferron supports automatic TLS via ACME-compatible Certificate Authorities such as **Let's Encrypt**. It supports three challenge types:

- **HTTP-01** (default) — serves a token at `/.well-known/acme-challenge/`
- **TLS-ALPN-01** — responds with a self-signed cert during the TLS handshake
- **DNS-01** — creates a TXT record at `_acme-challenge.<domain>` (required for wildcard domains)

Certificates are cached and automatically renewed before expiration.

Below is the example Ferron configuration that configures automatic TLS with the production Let's Encrypt directory:

```ferron
example.com {
    tls {
        provider acme
        challenge http-01
        contact "admin@example.com"
    }

    root /var/www/html
}
```

Or simply (since automatic TLS via ACME is enabled by default in Ferron for public hosts):

```ferron
example.com {
    # Automatic TLS is enabled by default, no explicit TLS directive needed

    root /var/www/html
}
```

## Note about Cloudflare proxies (and other HTTPS proxies)

Ferron uses HTTP-01 ACME challenge by default, which requires the server to be reachable on port 80. If your website is behind a proxy that terminates TLS (like Cloudflare's proxy mode), the HTTP-01 challenge may not work unless port 80 is accessible.

You can use TLS-ALPN-01 challenge instead, which works at the TLS handshake level and only requires port 443:

```ferron
example.com {
    tls {
        provider acme
        challenge tls-alpn-01
        contact "admin@example.com"
    }

    root /var/www/html
}
```

## Using Ferron as an ACME client for other servers

If you run other servers (alongside Ferron) that support TLS, but not automatic TLS functionality, you can use Ferron as an ACME client to obtain TLS certificates for those servers:

```ferron
example.com {
    tls {
        provider acme
        challenge http-01
        contact "admin@example.com"

        save "/tmp/server.crt" "/tmp/server.key"

        # Optionally, run a command after obtaining the certificate:
        # post_obtain_command "/etc/reload-server.sh"
    }

    root /var/www/html
}
```

If only one path is given for `save`, the key path defaults to the certificate path with a `.key` extension. After a certificate is obtained, the private key is written with `0600` permissions on Unix.

## Automatic TLS on demand

Ferron can also obtain certificates on demand when a hostname is accessed for the first time (`on_demand`). This is useful for multi-tenant setups where hostnames are not fully known in advance.

When enabling on-demand issuance, configure `on_demand_ask` to avoid abuse. Ferron will call the configured URL with the `domain` query parameter, and your endpoint should allow or deny issuance for that domain.

```ferron
*.example.com {
    tls {
        provider acme
        challenge http-01
        contact "admin@example.com"

        on_demand
        on_demand_ask "https://auth.example.com/check-cert"
    }

    root /var/www/html
}
```

## DNS providers (DNS-01 challenge)

Ferron supports DNS-01 ACME challenge for automatic TLS, which is required for wildcard certificates. The DNS-01 challenge requires a DNS provider to be configured inside the `tls` block.

Below is an example configuration for DNS-01 with Cloudflare:

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

    root /var/www/html
}
```

For the reference of supported DNS providers and their configuration properties, see the [configuration reference](/docs/v3/configuration/tls-acme).

## Certificate caching

Certificates are cached both in-memory and on disk (when a `cache` path is configured). This ensures certificates survive restarts and are automatically renewed.

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

## Notes and troubleshooting

- The default HTTP-01 challenge requires port 80 to be reachable. TLS-ALPN-01 only needs port 443.
- Ensure your public DNS records point to the Ferron server before requesting certificates; ACME challenges will fail if traffic goes elsewhere.
- If your site is behind an HTTPS-terminating proxy (for example Cloudflare proxy mode), use `challenge tls-alpn-01` (or DNS-01) because HTTP-01 may not work through TLS termination unless port 80 is also accessible.
- If you need wildcard certificates, use DNS-01 challenge; HTTP-01 and TLS-ALPN-01 do not support wildcard domains.
- Keep `cache` on persistent storage and ensure Ferron can read/write it, otherwise certificate renewals may fail or repeat unnecessarily.
- For DNS-01 failures, verify provider credentials and allow time for DNS propagation before retrying.
- For cipher suites, ECDH curves, and mTLS, see [Security and TLS](/docs/v3/configuration/security-tls).
- For TLS session ticket keys, see [TLS session ticket keys](/docs/v3/configuration/tls-session-tickets).
