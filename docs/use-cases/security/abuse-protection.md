---
title: Abuse protection
description: "Ban IPs temporarily after repeated rate limit breaches, failed logins, error responses, or custom events."
---

`abuse_protection` bans an IP in memory when it exceeds a threshold. It counts rate limit breaches, failed logins, error responses, and custom `abuse_event` events. Each ban expires after `ban_duration`.

For full directive details, see [Configuration: abuse protection](/docs/configuration/content/abuse-ban).

> [!important]
> Bans exist only in memory. They do not survive a restart, and there is no manual unban. Wait for the ban to expire.
>
> If Ferron bans your own IP, check the thresholds. `events 1` or a short `window` is easy to trip. Add trusted addresses to `allowlist`.
>
> Behind a reverse proxy, set `client_ip_from_header`. Without it, Ferron bans the proxy IP. See [HTTP host directives](/docs/configuration/server/host).

## Default thresholds

```ferron
example.com {
    abuse_protection

    root /var/www/html
}
```

A default ban lasts 15 minutes and starts when either condition occurs:

- 5 rate limit breaches within 5 minutes.
- 3 brute force failures within 2 minutes.

## Stricter login protection

```ferron
auth.example.com {
    abuse_protection {
        ban_duration "30m"

        rate_limit_threshold {
            events 3
            window "60s"
        }

        brute_force_threshold {
            events 2
            window "120s"
        }
    }

    location /login {
        basic_auth {
            realm "Admin Area"
            users {
                admin "$argon2id$v=19$m=19456,t=2,p=1$..."
            }
        }

        root /var/www/admin
    }
}
```

This bans an IP for 30 minutes after 3 rate limit breaches in 60 seconds, or 2 brute force failures in 120 seconds.

## Lenient API protection

```ferron
api.example.com {
    abuse_protection {
        ban_duration "5m"

        rate_limit_threshold {
            events 10
            window "600s"
        }

        brute_force_threshold {
            events 10
            window "600s"
        }
    }

    proxy http://backend:3000
}
```

This bans an IP for 5 minutes after 10 rate limit breaches or 10 brute force failures in 10 minutes. Higher thresholds and shorter bans suit public APIs with variable client behavior.

## Exempt trusted clients

```ferron
example.com {
    abuse_protection {
        allowlist "10.0.0.0/8" "172.16.0.0/12" "192.168.0.0/16"
        allowlist "203.0.113.50"
    }

    proxy http://backend:3000
}
```

Add IP addresses or CIDR ranges. Repeat `allowlist` to add more entries. Ferron never bans a matching IP, even when it exceeds a threshold.

## Combine with rate limiting

Rate limiting handles individual requests. Abuse protection bans clients that keep exceeding the limit:

```ferron
example.com {
    abuse_protection {
        ban_duration "15m"

        rate_limit_threshold {
            events 5
            window "300s"
        }
    }

    location / {
        rate_limit {
            rate 10
            burst 20
            key remote_address
        }

        proxy http://backend:3000
    }
}
```

1. The rate limiter throttles a client that runs out of tokens.
2. Ferron records each breach as an abuse event.
3. After enough breaches in the window, Ferron bans the client IP.
4. A banned client gets `403 Forbidden` with a `Retry-After` header.

## Detect automated scans by path

Use a matcher and a custom event to ban scanners:

```ferron
example.com {
    abuse_protection {
        custom_threshold {
            events 5
            window "300s"
        }
    }

    match AUTOMATED_SCAN {
        request.uri.path ~ r".*\.(?:php|asp|jsp|cgi|sql)(?:\b|$)|^/wp-(?:admin|login)(?:\b|$)|^/\.|^/(?:administrator|admin|login|logon)(?:\b|$)"
    }

    if AUTOMATED_SCAN {
        abuse_event automated_scan
    }

    root /var/www/html
}
```

Ferron bans the IP after 5 matching requests in 300 seconds. The ban uses `ban_duration`.

## Detect scans by error rate

`error_rate_threshold` bans clients that cause many error responses, such as scanners that probe for old vulnerabilities:

```ferron
example.com {
    abuse_protection {
        ban_duration "15m"

        error_rate_threshold {
            events 10
            window "60s"
            status_codes "404" "403"
        }
    }

    root /var/www/html
}
```

Ferron counts all listed status codes together. For example, 6 responses with 404 and 4 with 403 within the window trigger the ban.

For stricter protection, use `events 5`, `window "30s"`, and include `405`:

```ferron
example.com {
    abuse_protection {
        ban_duration "1h"

        error_rate_threshold {
            events 5
            window "30s"
            status_codes "404" "403" "405"
        }
    }

    root /var/www/html
}
```

## Disable abuse protection

```ferron
example.com {
    abuse_protection false

    root /var/www/html
}
```

## Report bans to AbuseIPDB

A sidecar can tail the Ferron WARN log and report banned IPs to [AbuseIPDB](https://www.abuseipdb.com/). The sidecar watches for `Ban triggered` lines and extracts the IP address and reason.

A log line looks like this:

```text
[2026-06-22 19:46:28.902 WARN] [trace=4dae55577f57aac23bdcffa24b38a31a] Ban triggered: IP ::1 - Custom abuse event: example_ban
```

- The `[trace=...]` block appears only when tracing is on.
- The IP address follows `IP `.
- The reason follows `-`.

| Source                | Example reason                       |
| --------------------- | ------------------------------------ |
| Rate limiting         | `Rate limit 10 req/s exceeded`       |
| Basic authentication  | `Brute-force failure for user admin` |
| `abuse_event`         | `Custom abuse event: wordpress_scan` |
| Error rate threshold  | `Error rate: 404 responses`          |

| Reason prefix         | AbuseIPDB category       | Category ID |
| --------------------- | ------------------------ | ----------- |
| `Rate limit`          | Web App Attack           | 21          |
| `Brute-force`         | Brute-Force              | 18          |
| `Custom abuse event:` | Web App Attack (default) | 21          |

The sidecar must read the log file and call `https://api.abuseipdb.com/api/v2/report` with your API key. You can run it as a systemd service next to Ferron or as a container that shares the log volume.

> [!note]
> AbuseIPDB API quotas apply. A simple sidecar is best-effort: it does not retry failed reports or keep a queue. Bans disappear when Ferron restarts, but the sidecar has already reported them.

## See also

- [Configuration: abuse protection](/docs/configuration/content/abuse-ban)
- [Rate limiting](/docs/use-cases/security/rate-limiting)
- [Fail2ban integration](/docs/use-cases/security/fail2ban)
- [HTTP basic authentication](/docs/configuration/security/basic-auth)
