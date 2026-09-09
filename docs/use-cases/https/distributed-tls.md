---
title: Distributed TLS
description: "Run multiple Ferron nodes behind a load balancer with a shared ACME cache and shared TLS session ticket keys."
---

When you run two or more Ferron servers behind a load balancer, each node terminates TLS on its own. Without shared state, two problems occur. The ACME server may send a challenge to a node that did not start the order, so validation fails. And all nodes may order certificates at the same time, which wastes resources and risks CA rate limits. Session resumption breaks across nodes for the same reason: a ticket that one node issues is unreadable on the other.

Point all nodes at one shared directory to fix both problems. Ferron syncs ACME challenges through the shared `cache` path and coordinates orders with lockfiles. It also shares TLS session ticket keys through a common key file.

## Prerequisites

- A shared filesystem that all nodes mount at the same path (NFS, EFS, or CephFS).
- The same Ferron configuration on every node, with the same `cache` path.
- Roughly synchronized clocks (NTP).
- Ports 80 and 443 reachable on every node for HTTP-01 challenges.

> [!important]
> Restrict access to the shared directory to your Ferron hosts. It holds private keys, account credentials, and challenge data.

## Share the ACME cache

Set the same `cache` path on every node:

```ferron
example.com {
    tls {
        provider acme
        challenge http-01
        contact "admin@example.com"
        cache "/var/cache/ferron-acme"
    }

    root /var/www/html
}
```

That is the only change most setups need. In-memory caching cannot coordinate, so always set `cache` on multi-node deployments.

How Ferron coordinates:

- The node that starts an order publishes `challenge_http_*` or `challenge_tls_*` files. Peers answer CA validation from these files when their own memory has no challenge data.
- Before an order starts, a node takes the paired `lock_certificate_*` lockfile. Peers skip that cycle and read the resulting `certificate_*` file on a later cycle.
- A lock without a heartbeat for 5 minutes counts as stale. The next contender clears it and logs a warning, so a crashed node never blocks issuance for long.

> [!info]
> For file names, TTLs, lock behavior, and observability signals, see the [configuration reference](/docs/configuration/security/acme).

## Share session ticket keys

Add one `ticket_keys` block with a file on the shared volume. Every node then decrypts tickets that any node issued:

```ferron
example.com {
    tls {
        provider acme
        challenge http-01
        contact "admin@example.com"
        cache "/var/cache/ferron-acme"

        ticket_keys {
            file "/var/cache/ferron-acme-shared/tickets.keys"
            auto_rotate
            rotation_interval "12h"
            max_keys 3
        }
    }

    root /var/www/html
}
```

Each node watches the file for changes. When one node rotates keys, the others pick up the new file without a restart.

> [!tip]
> Keep `auto_rotate` on in production. Rotation limits the impact of a key compromise.

## Verify the setup

Check that each node serves the same certificate:

```bash
echo | openssl s_client -connect node1.example.com:443 -servername example.com 2>/dev/null | openssl x509 -noout -serial -dates
echo | openssl s_client -connect node2.example.com:443 -servername example.com 2>/dev/null | openssl x509 -noout -serial -dates
```

Both commands must print the same serial number. Then check session resumption across nodes:

```bash
echo | openssl s_client -connect node1.example.com:443 -servername example.com -sess_out /tmp/sess.pem 2>/dev/null >/dev/null
echo | openssl s_client -connect node2.example.com:443 -servername example.com -sess_in /tmp/sess.pem 2>/dev/null | grep -i "reused"
```

The second command must report that the session was reused. Also confirm that no lockfiles linger after issuance:

```bash
ls /var/cache/ferron-acme/lock_*
```

This command must return no files. A leftover `lock_certificate_*` entry shows its owner in `host-pid` form. Ferron clears stale entries automatically, so a persistent entry points to clock skew or a read-only mount.

## See also

- [Configuration: ACME automatic TLS](/docs/configuration/security/acme): challenge sync, lockfiles, and lock metrics
- [Configuration: TLS session ticket keys](/docs/configuration/security/session-tickets): rotation settings and key file format
- [Automatic TLS](/docs/use-cases/https/automatic-tls): single-node ACME setup
