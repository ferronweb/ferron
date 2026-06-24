---
title: Edge caching (CDN)
description: "Deploy Ferron as a CDN edge node with HTTP caching, ACME TLS with fallback providers for resilience, global cache purging, and GeoDNS routing."
---

Ferron can be deployed as an edge caching node in a content delivery network (CDN). Its HTTP cache, automatic TLS, and purge propagation features make it suitable for serving cached content close to end users with centralized cache management.

## Automatic TLS with ACME fallback

When operating at CDN scale, ACME provider availability is critical. If the primary CA is down or unreachable, certificate issuance and renewal fail, leaving edge nodes without valid TLS certificates. Ferron supports **ACME fallback providers** to handle this — if the primary provider fails, the next configured fallback is tried automatically.

This is especially important for CDN deployments where a single edge node may terminate TLS for hundreds of domains and cannot afford downtime from CA outages.

```ferron
*.customer.example.com {
    tls {
        provider acme
        directory "https://acme-v02.api.letsencrypt.org/directory"
        challenge dns-01
        contact "admin@example.com"
        cache "/var/cache/ferron-acme"

        fallback {
            directory "https://dv.acme-v02.api.pki.goog/directory"
            contact "admin@example.com"
            eab "my-key-id" "SMq9KpHkR7z..." # Replace with your EAB credentials
        }

        dns {
            provider cloudflare
            api_key "EXAMPLE_API_KEY" # Replace with your API key, or preferably an environment variable interpolation
        }
    }

    cache {
        max_response_size 2097152
    }

    proxy http://origin.example.com:3000
}
```

Providers are tried **sequentially**: the primary is attempted first, followed by each `fallback` block in order. A fallback is triggered when account creation with the previous provider fails. Once a provider succeeds, subsequent operations (order creation, challenge solving, certificate installation) use that same provider.

## On-demand TLS for wildcard domains

Wildcard certificates normally require the **DNS-01** challenge, which means configuring and securing DNS provider API credentials on every edge node. **On-demand mode** avoids this by deferring certificate issuance until the first TLS handshake for a hostname. This lets you use the simpler **HTTP-01** challenge while still covering arbitrary subdomains under a wildcard.

```ferron
*.customer.example.com {
    tls {
        provider acme
        directory "https://acme-v02.api.letsencrypt.org/directory"
        challenge http-01
        contact "admin@example.com"
        cache "/var/cache/ferron-acme"
        on_demand
        on_demand_ask "https://internal-api.example.com/check-cert"

        fallback {
            directory "https://dv.acme-v02.api.pki.goog/directory"
            contact "admin@example.com"
            eab "my-key-id" "SMq9KpHkR7z..." # Replace with your EAB credentials
        }
    }

    cache {
        max_response_size 2097152
    }

    proxy http://origin.example.com:3000
}
```

> [!warning]
> Always configure `on_demand_ask` in production. Without an approval endpoint, Ferron will issue certificates for any hostname under the wildcard, which can be exploited for abuse.

The approval endpoint receives `?domain=<sni>` as a query parameter and must return `200` to approve issuance. This gives you control over which subdomains receive certificates without needing DNS provider keys on the edge.

## Global cache purging

A single edge node's cache can be purged locally with `PURGE` requests, but a CDN spans many nodes. Ferron supports multi-instance cache purge propagation via an external control-plane:

```ferron
*.customer.example.com {
    proxy http://origin.example.com:3000
    cache {
        purge_method
        purge_allowed_ips "10.0.0.0/8"
        purge_propagation {
            control_plane_url "http://control-plane:9090/cache/purge"
            shared_secret "edge-to-plane-secret"
            node_id "edge-fra1"
        }
    }
}
```

When a purge is triggered (via `PURGE` request or `X-LiteSpeed-Purge` header), the edge node notifies the control-plane, which fans out the invalidation to every registered edge. This means a single origin webhook or admin request invalidates content across the entire CDN.

> [!info]
> For a full explanation of the purge propagation protocol and control-plane implementation, see [HTTP caching — Multi-instance cache purge propagation](/docs/v3/use-cases/content/caching#multi-instance-cache-purge-propagation).

## GeoDNS for traffic routing

To direct users to the nearest edge node, use **GeoDNS** (geographic DNS) in front of your Ferron instances. A GeoDNS provider like Amazon Route 53, Google Cloud DNS, or DNS Made Easy returns different `A`/`AAAA` records based on the requester's location.

```text
us-east.example.com  A  203.0.113.10   (North America)
eu-west.example.com  A  198.51.100.20  (Europe)
ap-southeast.example.com  A  192.0.2.30  (Asia-Pacific)
```

The same Ferron configuration and cache store can be deployed on every node. GeoDNS handles routing; Ferron handles TLS termination, caching, and origin proxying at each location.

## Full edge node configuration

```ferron
# /etc/ferron/ferron.conf
{
    admin {
        listen "127.0.0.1:2080"
    }

    cache {
        max_entries 10000
    }
}

*.customer.example.com {
    tls {
        provider acme
        directory "https://acme-v02.api.letsencrypt.org/directory"
        challenge dns-01
        contact "admin@example.com"
        cache "/var/cache/ferron-acme"

        fallback {
            directory "https://dv.acme-v02.api.pki.goog/directory"
            contact "admin@example.com"
            eab "my-key-id" "SMq9KpHkR7z..." # Replace with your EAB credentials
        }

        dns {
            provider cloudflare
            api_key "EXAMPLE_API_KEY" # Replace with your API key, or preferably an environment variable interpolation
        }
    }

    cache {
        max_response_size 2097152
        vary Accept-Encoding

        purge_method
        purge_allowed_ips "10.0.0.0/8"
        purge_propagation {
            control_plane_url "http://control-plane:9090/cache/purge"
            shared_secret "edge-to-plane-secret"
            node_id "edge-fra1"
        }
    }

    proxy {
        upstream http://origin.example.com:3000 {
            active_check {
                uri "/health"
                interval "10s"
            }
        }
    }
}
```

This configuration creates a CDN edge node that:

- Terminates TLS with automatically renewed certificates, with fallback providers for CA resilience.
- Issues wildcard certificates on demand via HTTP-01, avoiding DNS provider setup on every edge node.
- Caches origin responses in memory (up to 10,000 entries, 2 MB each) with Vary support.
- Propagates cache purges to all other edge nodes via the control-plane.
- Actively health-checks the origin and proxies requests.

## See also

- [HTTP caching](/docs/v3/use-cases/content/caching) — cache directives, stale-while-revalidate, stale-if-error
- [Automatic TLS](/docs/v3/use-cases/security/automatic-tls) — ACME challenge types and DNS provider configuration
- [Reverse proxying](/docs/v3/use-cases/traffic/reverse-proxy) — load balancing, health checks, circuit breaking
- [ACME configuration reference](/docs/v3/configuration/security/acme) — on-demand mode, fallback providers, and ACME directory URLs
