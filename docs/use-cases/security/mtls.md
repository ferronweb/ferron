---
title: mTLS (mutual TLS)
description: "Require client TLS certificates in Ferron for internal/admin traffic and service-to-service access, and present client certificates to upstream backends."
---

Mutual TLS (mTLS) adds client certificate verification on top of normal server TLS. Ferron supports mTLS in two directions.

- **Inbound**: require clients connecting to Ferron to present a valid certificate (server-side mTLS).
- **Outbound**: present a client certificate when Ferron connects to an upstream backend via the reverse proxy (client-side mTLS).

## Require client certificates

Configure client certificate validation against your internal CA inside the `tls` block:

```ferron
# Replace "admin.example.com" with your domain name.
admin.example.com:443 {
    tls {
        provider manual
        cert "/etc/ssl/certs/admin.example.com.crt"
        key "/etc/ssl/private/admin.example.com.key"

        client_auth
        client_auth_ca "/etc/ssl/internal-client-ca.pem"
    }

    proxy http://127.0.0.1:9000
}
```

You can also use the OS trust store or the Mozilla root bundle:

```ferron
admin.example.com:443 {
    tls {
        provider manual
        cert "/etc/ssl/certs/admin.example.com.crt"
        key "/etc/ssl/private/admin.example.com.key"

        client_auth true
        client_auth_ca system  // or "webpki" for Mozilla's root bundle
    }

    proxy http://127.0.0.1:9000
}
```

> [!warning]
> When you use `client_auth_ca system`, the OS trust store includes all OS-trusted root CAs. Use it only when you want to accept client certificates from any publicly trusted CA. This is rarely the right choice for mTLS.

> [!important]
> For internal mTLS deployments, use a private CA and set `client_auth_ca` to the CA bundle file path. Protect private internal CA material and rotate client certificates regularly.

## mTLS with TLS 1.3 only

For maximum security, combine mTLS with TLS 1.3-only settings:

```ferron
internal-api.example.com:443 {
    tls {
        provider manual
        cert "/etc/ssl/certs/internal-api.example.com.crt"
        key "/etc/ssl/private/internal-api.example.com.key"

        min_version TLSv1.3
        max_version TLSv1.3

        client_auth true
        client_auth_ca "/etc/ssl/internal-ca-bundle.pem"
    }

    proxy http://127.0.0.1:9000
}
```

## Scope planning for admin/internal endpoints

`client_auth` lives inside a `tls` block, which scopes to a specific host. You can enable mTLS for some hosts while keeping others public. A separate Ferron instance is unnecessary.

```ferron
# Public website - no client auth
example.com:443 {
    tls {
        provider acme
        challenge http-01
        contact "admin@example.com"
    }

    root /var/www/html
}

# Internal admin - requires client certificate
admin.example.com:443 {
    tls {
        provider manual
        cert "/etc/ssl/certs/admin.example.com.crt"
        key "/etc/ssl/private/admin.example.com.key"

        client_auth
        client_auth_ca "/etc/ssl/internal-client-ca.pem"
    }

    proxy http://127.0.0.1:9000
}
```

## Presenting client certificates to upstream backends

When the reverse proxy connects to an HTTPS upstream, Ferron can present a client certificate. This authenticates the proxy to the backend. Configure `cert` and `key` on the upstream block:

```ferron
example.com {
    proxy {
        upstream https://backend.internal:443 {
            cert "/etc/ferron/client-cert.pem"
            key "/etc/ferron/client-key.pem"
        }
    }
}
```

You must supply both `cert` and `key` for mTLS to activate. The certificate chain and private key must be PEM-encoded.

### Per-upstream credentials

mTLS credentials scope per-upstream. Different backends can require different client certificates:

```ferron
example.com {
    proxy {
        upstream https://service-a.internal:443 {
            cert "/etc/ferron/service-a-client.crt"
            key "/etc/ferron/service-a-client.key"
        }
        upstream https://service-b.internal:443 {
            cert "/etc/ferron/service-b-client.crt"
            key "/etc/ferron/service-b-client.key"
        }
    }
}
```

### mTLS with SRV upstreams

When using SRV-record-based upstreams, the same mTLS credentials apply to all backends resolved from that SRV record:

```ferron
example.com {
    proxy {
        srv _https._tcp.backend.internal {
            cert "/etc/ferron/client-cert.pem"
            key "/etc/ferron/client-key.pem"
        }
    }
}
```

### Health checks and mTLS

Active health check probes use the same mTLS credentials configured on the upstream. Probes then accurately reflect backend reachability with client authentication.
