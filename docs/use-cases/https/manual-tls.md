---
title: Manual TLS
description: "Use existing certificate and key files when another system manages issuance and renewal."
---

Use manual TLS when you already have a certificate and private key. Automatic ACME is easier when you do not.

## Basic configuration

Set the `manual` provider and the certificate and key paths:

```ferron
manual-tls.example.com {
    tls {
        provider manual
        cert /etc/ssl/certs/manual-tls.example.com.crt
        key /etc/ssl/private/manual-tls.example.com.key
    }

    root /var/www/html
}
```

The short form takes the certificate and key paths directly:

```ferron
manual-tls.example.com {
    tls /etc/ssl/certs/manual-tls.example.com.crt /etc/ssl/private/manual-tls.example.com.key

    root /var/www/html
}
```

Paths support environment variable interpolation:

```ferron
manual-tls.example.com {
    tls {
        provider manual
        cert "{{env.TLS_CERT}}"
        key "{{env.TLS_KEY}}"
    }

    root /var/www/html
}
```

> [!important]
> Ferron must read both files. The certificate file must include the intermediate certificates that the CA requires. The certificate and key must match. A mismatch makes every TLS handshake fail.

> [!note]
> If another system rotates the files, reload or restart Ferron. Ferron does not read the files again on its own.

## One certificate per host

```ferron
example.com {
    tls /etc/ssl/certs/example.com.crt /etc/ssl/private/example.com.key

    root /var/www/example
}

api.example.com {
    tls /etc/ssl/certs/api.example.com.crt /etc/ssl/private/api.example.com.key

    proxy http://localhost:3000
}
```

## Custom crypto settings

Combine manual TLS with protocol versions, cipher suites, and ECDH curves:

```ferron
api.example.com {
    tls {
        provider manual
        cert /etc/ssl/certs/api.example.com.crt
        key /etc/ssl/private/api.example.com.key

        min_version TLSv1.3
        max_version TLSv1.3

        cipher_suite TLS_AES_256_GCM_SHA384
        cipher_suite TLS_CHACHA20_POLY1305_SHA256

        ecdh_curve x25519
    }

    proxy http://localhost:3000
}
```

> [!info]
> See [Security and TLS](/docs/configuration/security/tls) for cipher suites, curves, versions, and mTLS.
