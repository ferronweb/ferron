---
title: Abuse protection
description: "Protect Ferron from brute-force attacks, rate limit abuse, and other malicious behavior with temporary IP banning."
---

Ferron's `abuse_protection` directive provides lightweight, Fail2ban-style IP banning. When a client exceeds configured thresholds (e.g., repeated rate limit breaches or failed login attempts), Ferron temporarily bans their IP address. Bans are stored in memory and automatically expire after the configured duration.

This page covers common deployment patterns. For full configuration details, see [Configuration: abuse protection](/docs/v3/configuration/content/abuse-ban).

## Basic abuse protection

Enable abuse protection with default thresholds on a host:

```ferron
example.com {
    abuse_protection

    root /var/www/html
}
```

With defaults, Ferron bans an IP for **15 minutes** if:

- 5 rate limit breaches occur within 5 minutes, OR
- 3 brute force failures occur within 2 minutes

## Stricter protection for login endpoints

Tighten thresholds for hosts that handle authentication:

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

This bans an IP for **30 minutes** if:

- 3 rate limit breaches occur within 60 seconds, OR
- 2 brute force failures occur within 120 seconds

## Lenient protection for public-facing APIs

Use higher thresholds and shorter bans for public APIs where aggressive banning could impact legitimate traffic:

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

This bans an IP for **5 minutes** only after:

- 10 rate limit breaches within 10 minutes, OR
- 10 brute force failures within 10 minutes

## Exempting trusted IPs from abuse protection

Exclude internal networks, monitoring systems, or known-trusted IPs from bans:

```ferron
example.com {
    abuse_protection {
        allowlist "10.0.0.0/8" "172.16.0.0/12" "192.168.0.0/16"
        allowlist "203.0.113.50"
    }

    proxy http://backend:3000
}
```

IPs in the allowlist are **never banned**, even if they exceed thresholds. You can specify individual IPs or CIDR ranges. Use `allowlist` multiple times to add more entries.

## Combining with rate limiting for defense in depth

Use `abuse_protection` alongside `rate_limit` to add a second layer of protection. The rate limiter throttles traffic, while abuse protection bans repeat offenders:

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

The flow works as follows:

1. The rate limiter throttles individual clients that exceed their token bucket.
2. Each rate limit breach is recorded as an abuse event.
3. If the client accumulates enough breaches within the window, their IP is banned.
4. While banned, the client receives a **403 Forbidden** response with a `Retry-After` header.

## Disabling abuse protection

To disable abuse protection on a host:

```ferron
example.com {
    abuse_protection false

    root /var/www/html
}
```

## Notes and troubleshooting

- **Bans are in-memory only** — they are not preserved across server restarts.
- **No admin API for manual unban** — you must wait for the ban to expire naturally.
- **If your IP is banned immediately**, check your thresholds — you may have `events 1` or very short `window` values that are too aggressive.
- **If Ferron is behind a reverse proxy**, configure `client_ip_from_header` so Ferron sees the real client IP, not the proxy's IP. See [HTTP host directives](/docs/v3/configuration/server/host).
- **If legitimate clients are being banned**, add their IP or CIDR range to the `allowlist`.
- **If you see high ban counts**, consider increasing the `events` threshold or the `window` duration to reduce false positives.
- **For multiple event types**, each IP is tracked independently per event type. A client can be banned for rate limiting while their brute force count remains below threshold.
- **For directive details**, see [Configuration: abuse protection](/docs/v3/configuration/content/abuse-ban).
