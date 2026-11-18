---
title: "Configuration: ACME automatic TLS"
description: "Automatic certificate issuance and renewal through ACME, with HTTP-01, TLS-ALPN-01, and DNS-01 challenges."
---

Ferron obtains and renews certificates from ACME CAs such as Let's Encrypt. Issuance runs at startup (eager mode) or on the first handshake for a hostname (`on_demand`). Ferron caches certificates in memory and on disk, and renews them before they expire.

| Challenge                  | How it works                                                              | Requirement                          |
| -------------------------- | ------------------------------------------------------------------------- | ------------------------------------ |
| `http-01` (default)        | Ferron serves a token at `/.well-known/acme-challenge/`.                  | Port 80 reachable by the CA.          |
| `tls-alpn-01`              | Ferron answers the handshake with a self-signed `acme-tls/1` certificate. | Port 443 reachable. No wildcards.     |
| `dns-01`                   | Ferron creates a `_acme-challenge.<domain>` TXT record.                   | A DNS provider. Supports wildcards.   |

Ferron enables ACME automatically for public hosts:

```ferron
example.com {
    # Automatic TLS is enabled by default, no explicit TLS directive needed
}
```

## Challenge types

### HTTP-01

Ferron listens on port 80 and serves `/.well-known/acme-challenge/<token>`. The CA must reach port 80 to validate the challenge.

```ferron
example.com {
    tls {
        provider acme
        challenge http-01
        contact admin@example.com
    }
}
```

### TLS-ALPN-01

Ferron responds with a self-signed certificate when the CA connects with the `acme-tls/1` ALPN protocol. It needs no extra port. The server must be reachable on port 443. This challenge does not support wildcard domains.

Set it with `challenge tls-alpn-01`.

### DNS-01

Ferron creates a TXT record through a DNS provider. This is the only challenge type that supports wildcard certificates.

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
}
```

The `dns` block must name a `provider` and supply its credentials. See [DNS providers](/docs/configuration/security/dns-providers) for the supported providers and their directives.

After creating the record, Ferron queries every address of the zone's authoritative nameservers every 2 seconds. It asks the CA to validate as soon as all of them serve the record, or after 60 seconds at most. If Ferron cannot resolve the zone's nameservers, for example for a private zone, it waits the full 60 seconds. Ferron sends these queries from its own network, so with anycast nameservers it only sees the sites nearest to it.

> [!note]
> If the log shows `DNS-01 record for ... isn't served by all authoritative nameservers after 60 seconds`, the provider is slow to publish, or Ferron cannot query the nameservers directly. Check that outbound DNS is allowed. Ferron requests validation anyway. If validation fails, it retries on later cycles.

## Options

| Parameter                       | Type                               | Default                                                                       | Description                                           |
| ------------------------------- | ---------------------------------- | ----------------------------------------------------------------------------- | ----------------------------------------------------- |
| `provider`                      | `acme`                             | none                                                                          | Must be `acme`.                                       |
| `challenge`                     | `http-01`, `tls-alpn-01`, `dns-01` | `http-01`                                                                     | ACME challenge type.                                  |
| `contact`                       | `<string>`                         | none                                                                          | Email for the ACME account.                           |
| `directory`                     | `<string>`                         | Let's Encrypt production                                                      | ACME directory URL.                                   |
| `profile`                       | `<string>`                         | none                                                                          | ACME profile name (optional).                         |
| `eab`                           | `"<key-id>" "<hmac>"`              | none                                                                          | External Account Binding.                             |
| `cache`                         | `<string>`                         | `/var/cache/ferron-acme` if on Unix and writable, otherwise platform data dir | Path for file-based certificate caching.              |
| `save`                          | `<cert> [key]`                     | none                                                                          | Save the certificate, and optionally the key, to disk. |
| `post_obtain_command`           | `<string>`                         | none                                                                          | Command to run after certificate issuance.            |
| `no_verification`               | `<bool>`                           | `false`                                                                       | Skip ACME directory TLS verification.                 |
| `on_demand`                     | `<bool>`                           | `false`                                                                       | Issue certificates on the first handshake.            |
| `on_demand_ask`                 | `<string>`                         | none                                                                          | Approval endpoint URL.                                |
| `on_demand_ask_auth`            | `<string>`                         | none                                                                          | Authorization header for the approval endpoint (optional). |
| `on_demand_ask_no_verification` | `<bool>`                           | `false`                                                                       | Skip TLS verification for the approval endpoint.      |
| `fallback`                      | `{ ... }`                          | none                                                                          | Fallback provider block (repeatable).                 |
| `key_algo`                      | `<string>`                         | `PKCS_ECDSA_P256_SHA256`, or `PKCS_RSA_SHA256` when `key_size` is set          | Key algorithm for certificate signing.                |
| `key_size`                      | `<int>`                            | none                                                                          | RSA key size in bits: `2048`, `3072`, `4096`. RSA algorithms only. |

```ferron
example.com {
    tls {
        provider acme
        challenge http-01
        contact admin@example.com

        directory https://acme-v02.api.letsencrypt.org/directory
        cache /var/cache/ferron-acme

        save /etc/ssl/certs/example.com.pem /etc/ssl/private/example.com.pem
        # `post_obtain_command` arg is a script/binary name + args, separated by spaces.
        post_obtain_command /var/lib/post_obtain_command.sh

        key_algo PKCS_ECDSA_P384_SHA384

        ocsp
    }
}
```

### Supported key algorithms

| Identifier               | Description                                  |
| ------------------------ | -------------------------------------------- |
| `PKCS_ECDSA_P256_SHA256` | ECDSA P-256 with SHA-256                     |
| `PKCS_ECDSA_P384_SHA384` | ECDSA P-384 with SHA-384                     |
| `PKCS_ECDSA_P521_SHA256` | ECDSA P-521 with SHA-256                     |
| `PKCS_ECDSA_P521_SHA384` | ECDSA P-521 with SHA-384                     |
| `PKCS_ECDSA_P521_SHA512` | ECDSA P-521 with SHA-512                     |
| `PKCS_ED25519`           | Ed25519                                      |
| `PKCS_ML_DSA_44`         | Post-quantum ML-DSA-44                       |
| `PKCS_ML_DSA_65`         | Post-quantum ML-DSA-65                       |
| `PKCS_ML_DSA_87`         | Post-quantum ML-DSA-87                       |
| `PKCS_RSA_SHA256`        | RSA with SHA-256                             |
| `PKCS_RSA_SHA384`        | RSA with SHA-384                             |
| `PKCS_RSA_SHA512`        | RSA with SHA-512                             |
| `ECDSA`                  | Alias for `PKCS_ECDSA_P256_SHA256`           |
| `RSA`                    | Alias for `PKCS_RSA_SHA256`                  |

## Modes

### Eager mode

Eager mode obtains certificates at startup, before client traffic arrives. Ferron uses it unless `on_demand` is set. It fits static configurations where you know every domain name in advance.

### On-demand mode

On-demand mode issues a certificate during the first TLS handshake for a hostname. Use it for wildcard domains, multi-tenant hosting, or domains you do not know at startup.

### Approval endpoint

To prevent abuse, configure an approval endpoint. Before issuing a certificate, Ferron sends an HTTP GET request to the endpoint with `?domain=<sni>`. A `200` response authorizes issuance.

```ferron
*.example.com {
    tls {
        provider acme
        challenge http-01
        contact admin@example.com
        on_demand
        on_demand_ask https://internal-api.example.com/check-cert
    }
}
```

## Certificate caching

Ferron writes certificates and account data to a file cache by default. On Unix it uses `/var/cache/ferron-acme` when that directory exists and is writable. Otherwise it uses the platform data directory. Without a writable location, the cache stays in memory and does not survive a restart.

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

### Cache layout

```text
/var/cache/ferron-acme/
├── account_<hash>          # ACME account credentials
├── certificate_<hash>      # Certificate chain + private key (JSON)
├── challenge_http_<hash>   # Shared HTTP-01 challenge (distributed mode)
├── challenge_tls_<hash>    # Shared TLS-ALPN-01 challenge (distributed mode)
└── lock_certificate_<hash> # Provisioning lockfile (distributed mode)
```

### Cache keys

- Account cache key: hash of `contact emails + directory URL`.
- Certificate cache key: hash of `sorted domains + profile name`.
- Challenge keys: hash of the challenge `token` (`challenge_http_*`) or identifier (`challenge_tls_*`).
- Lock keys: `lock_` plus the paired `certificate_*`, `account_*`, or `hostname_*` key, so `ls lock_*` groups all locks.

## Distributed cache

Point every Ferron instance at the same shared `cache` directory (NFS, EFS, or CephFS) to run ACME behind a load balancer. Use the file cache settings above and give every node the same path.

- Challenge sync: the node that creates an order publishes `challenge_http_*` or `challenge_tls_*` files with a 15-minute TTL. Peers serve CA validation from these files when their in-memory locks miss, so validation succeeds whichever node the CA reaches.
- One order at a time: before ordering, a node takes the paired `lock_certificate_*` lockfile with an atomic create. Peers skip that cycle and pick up the resulting `certificate_*` file later. On-demand `hostname_*` appends take `lock_hostname_*` the same way.
- Self-healing locks: the holder refreshes a heartbeat every 30 seconds. A lock without a heartbeat for 5 minutes is treated as stale and broken by the next contender, which logs a warning. A configuration reload cancels an order and removes its lock at once, so the next cycle can order again. A release deletes only its own lock, never a peer's.
- Herd damping: skipped cycles return to the 10-second loop, and cycle sleeps include jitter so restarted nodes desynchronize.

All nodes need one writable `cache` path with atomic create and rename semantics, and roughly synchronized clocks. In-memory caching cannot coordinate nodes.

Challenge and key material is `0600` on Unix, but any node with cache access can read it. Restrict cache access to your Ferron hosts.

DNS-01 needs no challenge files because the TXT record is already shared. The provisioning lock still prevents duplicate orders.

To debug, run `ls lock_*`. Stuck entries show the owner as `host-pid`. Also check the `ACME lock stale, broken` and `ACME provisioning skipped, peer holds lock` log messages.

## Renewal

Ferron checks for renewal in the background every 10 seconds. It uses the ACME `renewalInfo` endpoint (RFC 9773) when available. Otherwise it falls back to half of the certificate lifetime, capped at 24 hours before expiry.

A configuration reload does not cancel an order in progress. The order finishes under the previous configuration, or is cancelled if it is still running after 5 minutes. With a file-based `cache` (the default), the reloaded configuration then installs the certificate from the cache.

The background task obtains and renews up to 4 certificates at once. Within one Ferron instance, orders that use the same ACME account create it only once. Orders that share a name run one after another, for example `example.com` and `*.example.com` in separate host blocks, because they share an `_acme-challenge` record.

> [!warning]
> `ACME certificate provisioning error: ...` means issuance failed. The message includes the affected domains. Check DNS resolution and the ACME server response. At debug log level, Ferron also logs each step: account loading, order creation, challenge solving, and certificate installation.

## External Account Binding (EAB)

Some CAs, especially enterprise or internal ACME servers, require External Account Binding. Set the key ID and HMAC secret:

```ferron
example.com {
    tls {
        provider acme
        challenge http-01
        contact admin@example.com
        eab my-key-id "SMq9KpHkR7z..."
        directory https://acme.internal.example.com/directory
    }
}
```

The HMAC secret must be base64url-encoded without padding.

## Fallback providers

If the primary provider fails, Ferron tries the configured fallback providers in order. A failure can be an unreachable CA or a failed provisioning attempt.

```ferron
example.com {
    tls {
        provider acme
        challenge http-01
        contact admin@example.com
        directory https://acme-v02.api.letsencrypt.org/directory

        fallback {
            directory https://acme-staging-v02.api.letsencrypt.org/directory
            contact admin@example.com
        }

        fallback {
            directory https://other-ca.example.com/directory
            eab my-key-id SMq9KpHkR7z...
            profile my-profile
        }
    }
}
```

Each `fallback` block accepts `directory`, `contact`, `eab`, and `profile`. It inherits the primary provider settings, so specify only the fields that differ.

## Save certificates

Use `save` to write certificates for other tools or for backup:

```ferron
example.com {
    tls {
        provider acme
        challenge http-01
        contact admin@example.com
        save /etc/ssl/certs/example.com.pem /etc/ssl/private/example.com.pem
    }
}
```

With one path, the key path is the certificate path with a `.key` extension. After issuance, Ferron writes the private key with `0600` permissions on Unix.

## Verify certificates

```bash
# Check the certificate served by Ferron
echo | openssl s_client -connect example.com -servername example.com 2>/dev/null | openssl x509 -noout -subject -dates -issuer

# Verify OCSP stapling
openssl s_client -connect example.com -status -servername example.com </dev/null 2>/dev/null | grep -A 5 "OCSP response"
```

## Security

- Ferron never logs private keys or exposes them in error messages.
- When Ferron saves keys to disk, it writes them with `0600` permissions on Unix.
- In on-demand mode, always configure `on_demand_ask` in production. Without it, clients can request certificates for arbitrary hostnames.

## Observability

The ACME background task emits log events and metrics through the configured observability pipeline.

> [!note]
> ACME log events are global-only. Ferron does not emit them through per-host observability sinks.

### Logs

| Level   | Message                                                                   | When                                    |
| ------- | ------------------------------------------------------------------------- | --------------------------------------- |
| `INFO`  | `ACME background task started with N configuration(s) for domains: ...`   | Service initialization                  |
| `INFO`  | `On-demand certificate requested for SNI <host>:<port>`                   | On-demand certificate request received  |
| `INFO`  | `On-demand certificate pre-loaded for SNI <host>:<port>`                  | On-demand certificate loaded from cache |
| `INFO`  | `ACME certificate issued for domains: ...`                                | Successful certificate issuance         |
| `INFO`  | `ACME account created for directory ..., contact: ...`                    | New ACME account registration           |
| `INFO`  | `Post-obtain command started for ...: <cmd>`                              | Post-obtain hook execution              |
| `WARN`  | `ACME certificate provisioning error for ...: <error>`                    | Certificate issuance failure            |
| `WARN`  | `ACME account not found on server for ..., recreating`                    | Account expired or removed on CA side   |
| `WARN`  | `Post-obtain command failed for ...: <error>`                             | Post-obtain hook error                  |
| `WARN`  | `Post-obtain command has malformed quoting for ...`                       | Post-obtain hook malformed command      |
| `WARN`  | `Failed to save ACME account cache: <error>`                              | Account cache write failure             |
| `WARN`  | `Failed to save ACME certificate cache: <error>`                          | Certificate cache write failure         |
| `WARN`  | `Broke a stale ACME provisioning lock for ...`                            | Stale lockfile cleared (holder crashed) |
| `WARN`  | `Failed to acquire ACME provisioning lock for ..., proceeding without it` | Lock I/O failure, fail-open             |
| `DEBUG` | `Skipping ACME provisioning for ...: a peer holds the provisioning lock`  | Contended cycle skipped (herd damping)  |
| `DEBUG` | `ACME provisioning cycle started — checking N configurations`             | Each background loop iteration          |
| `DEBUG` | `ACME account loaded from cache for ...`                                  | Account reused from cache               |
| `DEBUG` | `ACME order created for domains: ...`                                     | New order placed with CA                |
| `DEBUG` | `ACME <type> challenge initiated for ...`                                 | Challenge setup started                 |
| `DEBUG` | `ACME <type> challenge solved for ...`                                    | Challenge ready for validation          |
| `DEBUG` | `DNS-01 record created for _acme-challenge.<domain>, TTL <ttl>`           | DNS record published                    |
| `DEBUG` | `DNS-01 record for _acme-challenge.<domain> is served by all authoritative nameservers` | DNS record propagated |
| `DEBUG` | `Couldn't resolve the authoritative nameservers for _acme-challenge.<domain>, waiting out the 60-second DNS propagation delay` | Propagation not checked, fixed wait |
| `WARN`  | `DNS-01 record for _acme-challenge.<domain> isn't served by all authoritative nameservers after 60 seconds, requesting validation anyway` | Slow DNS propagation |
| `DEBUG` | `DNS-01 record cleanup completed for _acme-challenge.<domain>`            | DNS record removed                      |
| `DEBUG` | `Certificate installed for ..., chain length: N`                          | Certificate loaded into TLS config      |

### Structured logs

In OTLP `log_style modern`, the `summary` field acts as the log body and Ferron emits `attributes` as typed OpenTelemetry log record attributes.

| Summary                             | Level | Attributes                                                                                                                          |
| ----------------------------------- | ----- | ----------------------------------------------------------------------------------------------------------------------------------- |
| ACME background task started        | INFO  | `ferron.acme.config_count` (int), `ferron.acme.domains` (string)                                                                    |
| ACME account created                | INFO  | `ferron.acme.directory` (string): ACME directory URL, `ferron.acme.contact` (string): account email, `ferron.acme.profile` (string) |
| ACME certificate issued             | INFO  | `ferron.acme.domains` (string)                                                                                                      |
| ACME post-obtain command started    | INFO  | `ferron.acme.domains` (string)                                                                                                      |
| On-demand certificate pre-loaded    | INFO  | `tls.sni` (string), `tls.port` (int)                                                                                                |
| On-demand certificate requested     | INFO  | `tls.sni` (string), `tls.port` (int)                                                                                                |
| ACME account recreated              | WARN  | `ferron.acme.domains` (string), `ferron.acme.directory` (string)                                                                    |
| ACME certificate provisioning error | WARN  | `ferron.acme.domains` (string), `error.message` (string)                                                                            |
| ACME post-obtain command malformed  | WARN  | `ferron.acme.domains` (string)                                                                                                      |
| ACME post-obtain command failed     | WARN  | `ferron.acme.domains` (string), `error.message` (string)                                                                            |
| ACME post-obtain command empty      | WARN  | `ferron.acme.domains` (string)                                                                                                      |
| ACME account cache save failed      | WARN  | `error.message` (string)                                                                                                            |
| ACME certificate cache save failed  | WARN  | `error.message` (string)                                                                                                            |
| ACME provisioning cycle started     | DEBUG | `ferron.acme.config_count` (int)                                                                                                    |
| ACME account loaded from cache      | DEBUG | `ferron.acme.domains` (string), `ferron.acme.provider` (string)                                                                     |
| ACME order created                  | DEBUG | `ferron.acme.domains` (string)                                                                                                      |
| ACME certificate installed          | DEBUG | `ferron.acme.domains` (string), `ferron.acme.chain_length` (int)                                                                    |
| ACME challenge initiated            | DEBUG | `ferron.acme.domains` (string), `ferron.acme.challenge_type` (string)                                                               |
| ACME challenge solved               | DEBUG | `ferron.acme.domains` (string), `ferron.acme.challenge_type` (string)                                                               |
| ACME DNS-01 record created          | DEBUG | `ferron.acme.dns_challenge_domain` (string), `ferron.acme.dns_ttl` (int)                                                            |
| ACME DNS-01 record propagated       | DEBUG | `ferron.acme.dns_challenge_domain` (string)                                                                                         |
| ACME DNS-01 propagation unchecked   | DEBUG | `ferron.acme.dns_challenge_domain` (string)                                                                                         |
| ACME DNS-01 record not propagated   | WARN  | `ferron.acme.dns_challenge_domain` (string)                                                                                         |
| ACME DNS-01 record cleanup          | DEBUG | `ferron.acme.dns_challenge_domain` (string)                                                                                         |
| ACME lock stale, broken             | WARN  | `ferron.acme.domains` (string), `ferron.acme.lock_key` (string): stale lockfile cleared after heartbeat lease expiry                |
| ACME provisioning skipped           | DEBUG | `ferron.acme.domains` (string): peer holds the provisioning lock; this cycle is skipped                                             |
| ACME lock acquisition failed        | WARN  | `ferron.acme.domains` (string), `error.message` (string): lock I/O failed, provisioning proceeds without coordination               |
| ACME account creation failed        | WARN  | `ferron.acme.domains` (string), `ferron.acme.provider` (string), `error.message` (string)                                           |
| ACME account load/create failed     | ERROR | `ferron.acme.domains` (string), `error.message` (string)                                                                            |
| ACME order creation failed          | ERROR | `ferron.acme.domains` (string), `error.message` (string), `ferron.acme.provider` (string)                                           |
| ACME authorization failed           | ERROR | `ferron.acme.domains` (string), `ferron.acme.auth_status` (string), `ferron.acme.provider` (string)                                 |
| ACME challenge type unsupported     | ERROR | `ferron.acme.domains` (string), `ferron.acme.challenge_type` (string), `ferron.acme.provider` (string)                              |
| ACME identifier type unsupported    | ERROR | `ferron.acme.domains` (string), `ferron.acme.identifier_type` (string), `ferron.acme.provider` (string)                             |
| ACME challenge ready failed         | ERROR | `ferron.acme.domains` (string), `error.message` (string), `ferron.acme.provider` (string)                                           |
| ACME order finalization failed      | ERROR | `ferron.acme.domains` (string), `error.message` (string), `ferron.acme.provider` (string)                                           |
| ACME order invalid                  | ERROR | `ferron.acme.domains` (string)                                                                                                      |
| ACME order not ready                | ERROR | `ferron.acme.domains` (string), `ferron.acme.order_status` (string)                                                                 |
| ACME finalize failed                | ERROR | `ferron.acme.domains` (string), `error.message` (string), `ferron.acme.provider` (string)                                           |
| ACME certificate obtain failed      | ERROR | `ferron.acme.domains` (string), `error.message` (string), `ferron.acme.provider` (string)                                           |
| Certificate issuance denied         | ERROR | `tls.sni` (string): hostname blocked by ask endpoint                                                                                |
| Ask endpoint error                  | ERROR | `tls.sni` (string), `error.message` (string)                                                                                        |

### Metrics

| Metric                                  | Type    | Attributes                                                                        | Description                                  |
| --------------------------------------- | ------- | --------------------------------------------------------------------------------- | -------------------------------------------- |
| `ferron.acme.certificates_issued_total` | Counter | `ferron.acme.status` (`success`, `error`), `ferron.acme.challenge_type`           | Certificate issuance outcomes                |
| `ferron.acme.on_demand_requests_total`  | Counter | None                                                                              | On-demand certificate requests               |
| `ferron.acme.lock_acquired_total`       | Counter | `ferron.acme.domains`, `ferron.acme.lock_key`                                     | Provisioning locks acquired                  |
| `ferron.acme.lock_contention_total`     | Counter | `ferron.acme.domains`, `ferron.acme.lock_key`                                     | Cycles skipped, peer holds the lock          |
| `ferron.acme.lock_breaks_total`         | Counter | `ferron.acme.domains`, `ferron.acme.lock_key`                                     | Stale locks broken (crashed holder)          |
| `ferron.tls.certificate_not_after`      | Gauge   | `ferron.host`, `ferron.tls.provider` (`acme`), `crypto.certificate.serial_number` | Certificate `notAfter` as Unix epoch seconds |

## See also

- [DNS providers](/docs/configuration/security/dns-providers): all supported DNS-01 provider backends and their configuration
- [Security and TLS](/docs/configuration/security/tls): cipher suites, ECDH curves, mTLS
- [TLS session ticket keys](/docs/configuration/security/session-tickets): session resumption
- [OCSP stapling](/docs/configuration/security/ocsp): OCSP response stapling
- [HTTP host directives](/docs/configuration/server/host): per-host TLS configuration

## Best practices

`ferron doctor` reports the following best-practice checks for directives on this page.

- `no_verification` for ACME directory: Only disable TLS verification for the ACME directory in testing.
- `on_demand` without `on_demand_ask`: On-demand issuance without an approval endpoint allows certificate issuance for arbitrary hostnames. Configure `on_demand_ask` to approve requests.
- `on_demand_ask_no_verification`: Only disable TLS verification for the approval endpoint on strictly internal and otherwise authenticated endpoints.
- Missing `contact`: Without an ACME account email, the certificate authority cannot send expiry or account notices.
- Non-public domain: Domains with non-public TLDs (`.local`, `.internal`, `.home`, `.lan`, `.test`, `.localhost`) or bare IP addresses are unlikely to be publicly resolvable. ACME issuance fails because the CA cannot complete domain validation. This check applies to explicit `domains` directives and to the host block hostname when you use `on_demand` TLS.
