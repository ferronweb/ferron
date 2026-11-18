---
title: Edge caching (CDN)
description: "Deploy Ferron as a CDN edge node with HTTP caching, ACME TLS with fallback providers, cache purging, and GeoDNS routing."
---

Ferron can act as a CDN edge node. It caches responses close to users, terminates TLS, and receives cache purges from a central control plane.

## Automatic TLS with ACME fallback

If the primary CA is down or unreachable, certificate issuance and renewal fail. An edge node that serves hundreds of domains then loses TLS. Ferron can try fallback ACME providers when the primary provider fails.

```ferron
*.customer.example.com {
    tls {
        provider acme
        directory https://acme-v02.api.letsencrypt.org/directory
        challenge dns-01
        contact admin@example.com
        cache /var/cache/ferron-acme

        fallback {
            directory https://dv.acme-v02.api.pki.goog/directory
            contact admin@example.com
            eab my-key-id SMq9KpHkR7z... # Replace with your EAB credentials
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

Ferron tries the primary provider first, then each `fallback` block. A fallback starts when account creation with the previous provider fails. After a provider succeeds, later operations use the same provider.

## On-demand TLS for wildcard domains

Wildcard certificates normally require the DNS-01 challenge, so every edge node needs DNS provider credentials. On-demand mode defers issuance until the first TLS handshake for a hostname. You can then use HTTP-01 and avoid DNS credentials on each edge node.

```ferron
*.customer.example.com {
    tls {
        provider acme
        directory https://acme-v02.api.letsencrypt.org/directory
        challenge http-01
        contact admin@example.com
        cache /var/cache/ferron-acme
        on_demand
        on_demand_ask https://internal-api.example.com/check-cert

        fallback {
            directory https://dv.acme-v02.api.pki.goog/directory
            contact admin@example.com
            eab my-key-id SMq9KpHkR7z... # Replace with your EAB credentials
        }
    }

    cache {
        max_response_size 2097152
    }

    proxy http://origin.example.com:3000
}
```

> [!warning]
> Always configure `on_demand_ask` in production. Without an approval endpoint, Ferron issues certificates for any hostname under the wildcard. Attackers can exploit this for abuse.

The approval endpoint receives `?domain=<sni>` and must return `200` to approve issuance. You control which subdomains receive certificates, and you do not need DNS provider keys on the edge.

## Purge the global cache

A `PURGE` request clears one edge node. To clear all nodes, configure purge propagation through an external control plane:

```ferron
*.customer.example.com {
    proxy http://origin.example.com:3000
    cache {
        purge_method
        purge_allowed_ips "10.0.0.0/8"
        purge_propagation {
            control_plane_url http://control-plane:9090/cache/purge
            shared_secret "edge-to-plane-secret"
            node_id edge-fra1
        }
    }
}
```

When a client sends a `PURGE` request or an `X-LiteSpeed-Purge` header, the edge node notifies the control plane. The control plane sends the invalidation to every registered edge. One origin webhook or admin request then invalidates the content across the CDN.

> [!info]
> For details on the purge propagation protocol and control-plane implementation, see [HTTP caching: Purge propagation across instances](/docs/use-cases/content/caching#purge-propagation-across-instances).

## GeoDNS for traffic routing

To direct users to the nearest edge node, use GeoDNS (geographic DNS) in front of your Ferron instances. A GeoDNS provider like Amazon Route 53, Google Cloud DNS, or DNS Made Easy returns different `A`/`AAAA` records. It uses the location of the requester to select the response.

```text
us-east.example.com  A  203.0.113.10   (North America)
eu-west.example.com  A  198.51.100.20  (Europe)
ap-southeast.example.com  A  192.0.2.30  (Asia-Pacific)
```

You can deploy the same Ferron configuration and cache store on every node. GeoDNS handles routing. Ferron handles TLS termination, caching, and origin proxying at each location.

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
        directory https://acme-v02.api.letsencrypt.org/directory
        challenge dns-01
        contact admin@example.com
        cache /var/cache/ferron-acme

        fallback {
            directory https://dv.acme-v02.api.pki.goog/directory
            contact admin@example.com
            eab my-key-id SMq9KpHkR7z... # Replace with your EAB credentials
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
            control_plane_url http://control-plane:9090/cache/purge
            shared_secret "edge-to-plane-secret"
            node_id edge-fra1
        }
    }

    proxy {
        upstream http://origin.example.com:3000 {
            active_check {
                uri /health
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

- [HTTP caching](/docs/use-cases/content/caching): cache directives, stale-while-revalidate, stale-if-error
- [Automatic TLS](/docs/use-cases/https/automatic-tls): ACME challenge types and DNS provider configuration
- [Reverse proxying](/docs/use-cases/traffic/reverse-proxy): load balancing, health checks, circuit breaking
- [ACME configuration reference](/docs/configuration/security/acme): on-demand mode, fallback providers, and ACME directory URLs
