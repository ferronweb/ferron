---
title: mTLS (mutual TLS)
description: "Require client TLS certificates in Ferron for internal/admin traffic and service-to-service access."
---

Mutual TLS (mTLS) adds client certificate verification on top of normal server TLS. This is useful for internal admin panels, partner integrations, and service-to-service traffic.

## Require client certificates

Configure client certificate validation against your internal CA inside the `tls` block:

```ferron
// Replace "admin.example.com" with your domain name.
admin.example.com:443 {
    tls {
        provider "manual"
        cert "/etc/ssl/certs/admin.example.com.crt"
        key "/etc/ssl/private/admin.example.com.key"

        client_auth true
        client_auth_ca "/etc/ssl/internal-client-ca.pem"
    }

    location / {
        proxy http://127.0.0.1:9000
    }
}
```

You can also use the OS trust store or Mozilla's root bundle:

```ferron
admin.example.com:443 {
    tls {
        provider "manual"
        cert "/etc/ssl/certs/admin.example.com.crt"
        key "/etc/ssl/private/admin.example.com.key"

        client_auth true
        client_auth_ca system  // or "webpki" for Mozilla's root bundle
    }

    location / {
        proxy http://127.0.0.1:9000
    }
}
```

## mTLS with TLS 1.3 only

For maximum security, combine mTLS with TLS 1.3-only settings:

```ferron
internal-api.example.com:443 {
    tls {
        provider "manual"
        cert "/etc/ssl/certs/internal-api.example.com.crt"
        key "/etc/ssl/private/internal-api.example.com.key"

        min_version TLSv1.3
        max_version TLSv1.3

        client_auth true
        client_auth_ca "/etc/ssl/internal-ca-bundle.pem"
    }

    location / {
        proxy http://127.0.0.1:9000
    }
}
```

## Scope planning for admin/internal endpoints

`client_auth` is configured inside a `tls` block, which is scoped to a specific host. This means you can enable mTLS for some hosts while keeping others public — no separate Ferron instance is needed.

```ferron
// Public website — no client auth
example.com:443 {
    tls {
        provider "acme"
        challenge http-01
        contact "admin@example.com"
    }

    location / {
        root /var/www/html
    }
}

// Internal admin — requires client certificate
admin.example.com:443 {
    tls {
        provider "manual"
        cert "/etc/ssl/certs/admin.example.com.crt"
        key "/etc/ssl/private/admin.example.com.key"

        client_auth true
        client_auth_ca "/etc/ssl/internal-client-ca.pem"
    }

    location / {
        proxy http://127.0.0.1:9000
    }
}
```

## Notes and troubleshooting

- Ensure the client certificate chain is issued by the CA you configured in `client_auth_ca`.
- When `client_auth_ca system` is used, the OS trust store includes all OS-trusted root CAs — use it only when you want to accept client certificates from any publicly trusted CA (rarely the right choice for mTLS).
- For internal mTLS deployments, use a private CA and set `client_auth_ca` to the CA bundle file path.
- Keep private internal CA material protected and rotate client certificates regularly.
- If requests fail during TLS handshake, verify certificate validity dates and CA chain.
- If you get `"native-certs feature not enabled"` or `"webpki-roots feature not enabled"`, the corresponding `client_auth_ca` mode requires a feature that is not compiled in.
- For directive details, see [Configuration: security and TLS](/docs/v3/configuration/security-tls#client-certificate-authentication-mtls).
