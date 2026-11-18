---
title: Automatic TLS
description: "Get and renew certificates with ACME, using HTTP-01, TLS-ALPN-01, or DNS-01 challenges."
---

Ferron gets certificates from an ACME certificate authority such as Let's Encrypt. It caches the certificates and renews them before expiry.

| Challenge      | How it works                                                        | Use it when                          |
| -------------- | ------------------------------------------------------------------- | ------------------------------------ |
| `http-01`      | Serves a token at `/.well-known/acme-challenge/`.                   | Port 80 is reachable. This is the default. |
| `tls-alpn-01`  | Answers the challenge during the TLS handshake.                     | Only port 443 is reachable.          |
| `dns-01`       | Creates a TXT record at `_acme-challenge.<domain>`.                 | You need a wildcard certificate.     |

## Basic configuration

```ferron
example.com {
    tls {
        provider acme
        challenge http-01
        contact admin@example.com
    }

    root /var/www/html
}
```

For a public host, you can omit the `tls` block. Ferron enables automatic TLS through ACME by default:

```ferron
example.com {
    root /var/www/html
}
```

> [!important]
> Keep the ACME cache on persistent storage. Ferron must read and write the cache directory. Without persistent storage, renewals can fail or repeat after a restart.

## Behind a TLS-terminating proxy

HTTP-01 needs port 80. When a proxy such as Cloudflare terminates TLS, port 80 can be unreachable. Use TLS-ALPN-01 instead. It needs only port 443.

```ferron
example.com {
    tls {
        provider acme
        challenge tls-alpn-01
        contact admin@example.com
    }

    root /var/www/html
}
```

## Get certificates for other servers

Ferron can act as an ACME client for another server that supports TLS but not ACME. Set `save` to write the certificate and key:

```ferron
example.com {
    tls {
        provider acme
        challenge http-01
        contact admin@example.com

        save /tmp/server.crt /tmp/server.key

        # Optional: run a command after issuance
        # post_obtain_command "/etc/reload-server.sh"
    }

    root /var/www/html
}
```

If you give one path to `save`, the key path is the certificate path with a `.key` extension. On Unix, Ferron writes the private key with mode `0600`.

## Issue certificates on demand

Ferron can get a certificate when a client first requests a hostname. This fits multi-tenant setups where you do not know every hostname in advance.

Set `on_demand_ask` to an authorization endpoint. Ferron calls it with the `domain` query parameter. The endpoint must allow or deny issuance for that domain.

```ferron
*.example.com {
    tls {
        provider acme
        challenge http-01
        contact admin@example.com

        on_demand
        on_demand_ask https://auth.example.com/check-cert
    }

    root /var/www/html
}
```

## Wildcard certificates with DNS-01

A wildcard certificate needs the DNS-01 challenge. Add a `dns` block with the provider and its credentials:

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01
        contact admin@example.com

        dns {
            provider cloudflare
            api_key "EXAMPLE_API_KEY"
        }
    }

    root /var/www/html
}
```

> [!important]
> Point the public DNS records at the Ferron server before you request certificates. ACME validation fails when traffic goes elsewhere.

> [!info]
> See [DNS providers](/docs/configuration/security/dns-providers) for the supported providers and their options.

## Certificate cache

Set `cache` to keep certificates on disk across restarts:

```ferron
example.com {
    tls {
        provider acme
        challenge http-01
        contact admin@example.com
        cache /var/cache/ferron-acme
    }
}
```

Ferron caches certificates in memory and on disk. A disk cache also lets several nodes share one ACME setup.

## RSA certificates

Ferron uses ECDSA with P-256 and SHA-256 by default. To use RSA, set `key_algo` and `key_size`:

```ferron
example.com {
    tls {
        provider acme
        challenge http-01
        contact admin@example.com
        cache /var/cache/ferron-acme

        key_algo PKCS_RSA_SHA256
        key_size 4096
    }
}
```

> [!info]
> For all ACME options, see [Configuration: ACME](/docs/configuration/security/acme). To share one ACME cache across nodes, see [Distributed TLS](/docs/use-cases/https/distributed-tls).
