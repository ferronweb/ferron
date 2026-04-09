---
title: Manual TLS
description: "Configure Ferron with existing TLS certificates and private keys, without automatic ACME certificate issuance."
---

Manual TLS is useful when you already have certificates and private keys, or when certificate issuance and renewal is handled outside Ferron.

For many use cases, automatic TLS with ACME is the easiest way to get HTTPS working. However, if you have specific requirements or existing certificates, you can configure Ferron to use them directly.

To enable manual TLS for a host, configure the `tls` directive with the `"manual"` provider and certificate/private key paths:

```ferron
# Replace "manual-tls.example.com" with your domain name.
manual-tls.example.com:443 {
    tls {
        provider "manual"
        cert "/etc/ssl/certs/manual-tls.example.com.crt"
        key "/etc/ssl/private/manual-tls.example.com.key"
    }

    root /var/www/html
}
```

You can also use environment variable interpolation for the paths:

```ferron
manual-tls.example.com:443 {
    tls {
        provider "manual"
        cert "{{env.TLS_CERT}}"
        key "{{env.TLS_KEY}}"
    }

    root /var/www/html
}
```

The certificate and private key must match. If they do not match, TLS handshakes will fail.

## Manual TLS with multiple hosts

You can configure manual TLS per virtual host. This is useful when each domain uses different certificates:

```ferron
example.com:443 {
    tls {
        provider "manual"
        cert "/etc/ssl/certs/example.com.crt"
        key "/etc/ssl/private/example.com.key"
    }

    location / {
        root /var/www/example
    }
}

api.example.com:443 {
    tls {
        provider "manual"
        cert "/etc/ssl/certs/api.example.com.crt"
        key "/etc/ssl/private/api.example.com.key"
    }

    location / {
        proxy http://localhost:3000
    }
}
```

## Manual TLS with custom crypto settings

You can combine manual TLS with custom cipher suites, ECDH curves, and protocol version restrictions:

```ferron
api.example.com:443 {
    tls {
        provider "manual"
        cert "/etc/ssl/certs/api.example.com.crt"
        key "/etc/ssl/private/api.example.com.key"

        min_version TLSv1.3
        max_version TLSv1.3

        cipher_suite TLS_AES_256_GCM_SHA384
        cipher_suite TLS_CHACHA20_POLY1305_SHA256

        ecdh_curve x25519
    }

    location / {
        proxy http://localhost:3000
    }
}
```

## Notes and troubleshooting

- Make sure Ferron can read both the certificate and key files.
- Ensure the certificate file includes any required intermediate certificates when needed by your CA.
- If you rotate certificates externally, reload or restart Ferron so updated files are used.
- If you do not want automatic TLS on a host, do not use `provider "acme"` — use `provider "manual"` instead.
- For all TLS-related directives (`cipher_suite`, `ecdh_curve`, `min_version`, `max_version`, `client_auth`), see [Security and TLS](/docs/v3/configuration/security-tls).
- For ACME automatic TLS, see [ACME automatic TLS](/docs/v3/configuration/tls-acme).
