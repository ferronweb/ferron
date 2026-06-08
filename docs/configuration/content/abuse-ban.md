---
title: "Configuration: abuse protection"
description: "Lightweight Fail2ban-like IP banning with temporary lockouts for abusive behavior."
---

This page documents the `abuse_protection` directive for configuring lightweight IP-based abuse protection. When a client exceeds configured thresholds (e.g., rate limit breaches, brute-force failures), Ferron temporarily bans the IP address.

The abuse protection module works by tracking abuse events emitted by other HTTP modules (such as rate limiting and basic authentication) and temporarily banning IPs that exceed configured thresholds within time windows. Bans are stored in memory and automatically expire after the configured duration.

## `abuse_protection`

```ferron
example.com {
    abuse_protection {
        ban_duration "15m"

        rate_limit_threshold {
            events 5
            window "300s"
        }

        brute_force_threshold {
            events 3
            window "120s"
        }
    }
}
```

The `abuse_protection` block can be placed inside an HTTP host block.

| Nested directive | Arguments | Description | Default |
| --- | --- | --- | --- |
| `ban_duration` | `<duration>` | How long to ban an IP. | `"15m"` (15 minutes) |
| `rate_limit_threshold` | block | Ban after N rate limit events in window. | 5 in 300s |
| `brute_force_threshold` | block | Ban after N brute force failures in window. | 3 in 120s |
| `custom_threshold` | block | Ban after N custom events in window. | none |
| `allowlist` | `<string> [<string> ...]` | IP addresses or CIDR ranges exempt from bans. This directive can be specified multiple times. | none |

### Threshold blocks

Each threshold block (`rate_limit_threshold`, `brute_force_threshold`, `custom_threshold`) configures when a specific event type triggers a ban:

| Nested directive | Arguments | Description |
| --- | --- | --- |
| `events` | `<int>` | Number of events required to trigger a ban (required). |
| `window` | `<duration>` | Time window for counting events (required). |

**Configuration example — stricter thresholds:**

```ferron
example.com {
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
}
```

Ban after:

- 3 rate limit events in 60s (30-min ban), OR
- 2 brute force failures in 120s (30-min ban)

**Configuration example — lenient thresholds:**

```ferron
example.com {
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
}
```

Ban after:

- 10 rate limit events in 10 minutes (5-min ban), OR
- 10 brute force failures in 10 minutes (5-min ban)

**Configuration example — trusted IP list:**

```ferron
example.com {
    abuse_protection {
        allowlist "10.0.0.0/8" "192.168.1.1"
    }
}
```

IPs added to the `allowlist` are never banned, even if they exceed thresholds. This is useful for protecting internal networks, monitoring systems, or other trusted infrastructure.

## Behavior

### Event tracking

The module tracks events per IP address and event type:

- **Rate limit events** — emitted by the rate limiting module when a client exceeds their rate limit.
- **Brute force events** — emitted by the basic authentication module when a client has repeated failed authentication attempts.
- **Custom events** — available for other modules to emit custom abuse events.

Events are stored in a sliding time window. When the number of events within the window reaches the configured threshold, the IP is immediately banned.

### Ban mechanics

- **Ban duration** — fixed duration (default 15 minutes); independent from the event counting window.
- **TTL-based expiry** — bans automatically expire after the configured duration; no background eviction threads are needed.
- **Per-IP tracking** — each IP address is tracked independently. Different event types are tracked separately for the same IP.
- **No persistence** — bans are stored in memory and are **not** preserved across server restarts.

> [!tip]
> If your IP is banned immediately, check configured thresholds — you may have `events 1` or `events 2`, or very short `window` values (e.g., 10s) that are too aggressive. Reduce the `ban_duration` to shorten ban times, or increase the `events` threshold to require more violations before banning.

> [!warning]
> For manual unbanning, you must wait for the ban to expire naturally.

### Request flow

1. Client sends a request.
2. The abuse protection stage runs early in the pipeline (after `client_ip_from_header`, before `rate_limit` and `basicauth`).
3. The stage checks if the client's IP is currently banned.
4. If banned: returns **403 Forbidden** with a `Retry-After` header indicating seconds until ban expiry.
5. If not banned: request continues through the pipeline normally.
6. When other modules detect abuse (rate limit breach, brute force attempt), they emit events to the abuse registry.
7. If an event causes a threshold to be exceeded, the IP is immediately banned.

### Allowlist behavior

Any IP that matches an entry in the `allowlist` is skipped before ban checks are performed. This allows you to protect internal services, monitoring systems, or known-trusted infrastructure from accidental bans.

## Examples

### Basic abuse protection (defaults)

```ferron
example.com {
    abuse_protection
}
```

Uses default thresholds: bans IPs for 15 minutes if:

- 5 rate limit breaches in 5 minutes, OR
- 3 brute force failures in 2 minutes

### Disable abuse protection

```ferron
example.com {
    abuse_protection false
}
```

## Observability

### Metrics

The abuse protection module emits the following metrics:

| Metric | Type | Attributes | Description |
|--------|------|------------|-------------|
| `ferron.abuseban.rejected` | Counter | `ferron.abuseban.reason` (`"rate_limit"`, `"brute_force"`) | Requests rejected due to IP ban |

### Logs

- **`DEBUG`**: logged when a ban rejection occurs, including the banned IP address and reason.

### Structured logs

| Description (summary) | Level | Attributes |
|-----------------------|-------|------------|
| Ban rejection         | DEBUG | `client.address` (client's IP address), `ferron.abuseban.reason` (`"rate_limit"`, `"brute_force"`), `ferron.abuseban.remaining_secs` (remaining seconds before ban expires) |

## See also

- [Rate limiting](/docs/v3/configuration/content/rate-limit)
- [HTTP basic authentication](/docs/v3/configuration/security/basic-auth)
- [Routing and URL processing](/docs/v3/configuration/routing/url-processing)

## Best practices

The following best-practice checks are reported by `ferron doctor` for directives on this page.

- **`abuse_protection false`** — Disabling IP banning for repeated abuse events removes a layer of protection. Keep it enabled unless another layer handles abusive clients.
- **`allowlist` with wildcard** — Exempting every source address from abuse protection should be restricted to known trusted clients.
