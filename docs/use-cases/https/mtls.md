---
title: mTLS (mutual TLS)
description: "Require client certificates for inbound access, or present a client certificate to an upstream backend."
---

Mutual TLS (mTLS) adds client certificate verification to normal server TLS. Ferron supports mTLS in two directions:

- Inbound: clients must present a valid certificate to connect to Ferron.
- Outbound: Ferron presents a client certificate when it connects to an HTTPS backend.

## Require client certificates

Verify client certificates against your internal CA:

```ferron
admin.example.com:443 {
    tls {
        provider manual
        cert /etc/ssl/certs/admin.example.com.crt
        key /etc/ssl/private/admin.example.com.key

        client_auth
        client_auth_ca /etc/ssl/internal-client-ca.pem
    }

    proxy http://127.0.0.1:9000
}
```

You can also use the OS trust store or the Mozilla root bundle:

```ferron
admin.example.com:443 {
    tls {
        provider manual
        cert /etc/ssl/certs/admin.example.com.crt
        key /etc/ssl/private/admin.example.com.key

        client_auth
        client_auth_ca system
    }

    proxy http://127.0.0.1:9000
}
```

> [!warning]
> `client_auth_ca system` accepts client certificates from any CA in the OS trust store. That includes public root CAs. For mTLS, use a private CA bundle instead.

> [!important]
> For internal mTLS, use a private CA and set `client_auth_ca` to the CA bundle file. Protect the CA material and rotate client certificates regularly.

## TLS 1.3 only

```ferron
internal-api.example.com:443 {
    tls {
        provider manual
        cert /etc/ssl/certs/internal-api.example.com.crt
        key /etc/ssl/private/internal-api.example.com.key

        min_version TLSv1.3
        max_version TLSv1.3

        client_auth
        client_auth_ca /etc/ssl/internal-ca-bundle.pem
    }

    proxy http://127.0.0.1:9000
}
```

## Mix public and internal hosts

`client_auth` belongs to the `tls` block, so it applies to one host. You can protect some hosts and leave others public:

```ferron
example.com:443 {
    tls {
        provider acme
        challenge http-01
        contact admin@example.com
    }

    root /var/www/html
}

admin.example.com:443 {
    tls {
        provider manual
        cert /etc/ssl/certs/admin.example.com.crt
        key /etc/ssl/private/admin.example.com.key

        client_auth
        client_auth_ca /etc/ssl/internal-client-ca.pem
    }

    proxy http://127.0.0.1:9000
}
```

## Present a client certificate to a backend

Set `cert` and `key` on the upstream. Ferron presents the certificate when it connects to the HTTPS backend:

```ferron
example.com {
    proxy {
        upstream https://backend.internal:443 {
            cert /etc/ferron/client-cert.pem
            key /etc/ferron/client-key.pem
        }
    }
}
```

Both directives are required. The certificate chain and private key must use PEM encoding.

### One credential per upstream

Each upstream can use its own certificate:

```ferron
example.com {
    proxy {
        upstream https://service-a.internal:443 {
            cert /etc/ferron/service-a-client.crt
            key /etc/ferron/service-a-client.key
        }
        upstream https://service-b.internal:443 {
            cert /etc/ferron/service-b-client.crt
            key /etc/ferron/service-b-client.key
        }
    }
}
```

### SRV upstreams

For an SRV-based upstream, the same credential applies to every backend the record resolves:

```ferron
example.com {
    proxy {
        srv _https._tcp.backend.internal {
            cert /etc/ferron/client-cert.pem
            key /etc/ferron/client-key.pem
        }
    }
}
```

### Health checks

Active health checks use the same mTLS credentials as normal requests. A probe reflects reachability with client authentication.

> [!info]
> See [Configuration: TLS](/docs/configuration/security/tls) for all mTLS options.
