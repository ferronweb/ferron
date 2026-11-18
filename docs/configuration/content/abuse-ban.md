---
title: "Configuration: abuse protection"
description: "Temporary IP bans for clients that exceed abuse thresholds."
---

`abuse_protection` bans an IP address for a fixed time when it exceeds a threshold. Ferron counts abuse events from rate limiting, basic authentication, error responses, and custom `abuse_event` rules. Bans live in memory and expire on their own.

```ferron
example.com {
    abuse_protection {
        ban_duration "15m"
        log_rejections

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

Place the `abuse_protection` block inside an HTTP host block. A bare `abuse_protection` enables it with defaults.

| Nested directive        | Arguments                 | Description                                   | Default              |
| ----------------------- | ------------------------- | --------------------------------------------- | -------------------- |
| `ban_duration`          | `<duration>`              | How long to ban an IP.                        | `"15m"` (15 minutes) |
| `rate_limit_threshold`  | block                     | Ban after N rate limit events in the window.  | 5 in 300s            |
| `brute_force_threshold` | block                     | Ban after N brute force failures in the window. | 3 in 120s          |
| `custom_threshold`      | block                     | Ban after N custom events in the window.      | none                 |
| `error_rate_threshold`  | block                     | Ban after N error responses in the window.    | none                 |
| `allowlist`             | `<string> [<string> ...]` | IP addresses or CIDR ranges exempt from bans. | none                 |
| `log_rejections`        | `[bool]`                  | Per-request ban rejection debug logs.         | `false`              |

You can specify `allowlist` more than once. Ban starts and expirations always log. Per-request rejections stay quiet by default because a ban wave would flood the log. Set `log_rejections` to get `DEBUG` rejection logs, and watch the `ferron.abuseban.rejected` counter for rejection volume.

### Threshold blocks

`rate_limit_threshold`, `brute_force_threshold`, and `custom_threshold` share the same shape. A threshold fires when the event count inside the window reaches `events`. Ferron bans the IP immediately.

| Nested directive | Arguments    | Description                            |
| ---------------- | ------------ | -------------------------------------- |
| `events`         | `<int>`      | Events needed to trigger a ban. Required. |
| `window`         | `<duration>` | Time window for counting events. Required. |

### Error rate threshold

`error_rate_threshold` bans clients that produce many error responses, such as vulnerability scanners.

| Nested directive | Arguments                 | Description                              | Default |
| ---------------- | ------------------------- | ---------------------------------------- | ------- |
| `events`         | `<int>`                   | Error responses needed to trigger a ban. | 50      |
| `window`         | `<duration>`              | Time window for counting errors.         | `"60s"` |
| `status_codes`   | `<string> [<string> ...]` | Status codes that count as errors.       | `"404"` |

```ferron
example.com {
    abuse_protection {
        error_rate_threshold {
            events 10
            window "60s"
            status_codes "404" "403"
        }
    }
}
```

This configuration bans an IP after 10 responses with status 404 or 403 within 60 seconds.

### Tune the thresholds

These thresholds ban an IP for 30 minutes after 3 rate limit events in 60 seconds, or after 2 brute force failures in 120 seconds.

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

### Exempt trusted clients

```ferron
example.com {
    abuse_protection {
        allowlist "10.0.0.0/8" "192.168.1.1"
    }
}
```

Ferron never bans an IP in the `allowlist`, even when it exceeds a threshold. Use this for internal networks, monitoring systems, and other trusted infrastructure.

### Disable abuse protection

```ferron
example.com {
    abuse_protection false
}
```

## `abuse_event`

`abuse_event <name>` registers a custom abuse event. The name is a label for the event, and all custom events count toward the `custom_threshold` block.

```ferron
example.com {
    abuse_protection {
        custom_threshold {
            events 5
            window "300s"
        }
    }

    match WORDPRESS_SCAN {
        request.uri.path ~ r"/wp-(?:admin|login)(?:.php$|/|$)"
    }

    if WORDPRESS_SCAN {
        abuse_event wordpress_scan # <-- register WordPress scanning
    }
}
```

Place `abuse_event` inside an HTTP host block. Use it to protect a site from automated scanners, or to build a honeypot inside Ferron.

## Behavior

Ferron tracks events per IP address and per event type:

- Rate limit events: emitted by the rate limiting module when a client exceeds its limit.
- Brute force events: emitted by the basic authentication module after repeated failed logins.
- Custom events: emitted by other modules or by `abuse_event`.
- Error rate events: counted by the abuse protection module after the pipeline completes, when the final status code matches `status_codes`. For example, 404 and 403.

Events sit in a sliding window. When the count inside the window reaches the threshold, Ferron bans the IP.

Bans have these properties:

- Fixed duration, set by `ban_duration`. The ban duration is independent from the event counting window.
- Expiry by TTL. No background eviction task is needed.
- Per-IP and per-event-type tracking. One IP can have separate counts and bans for each event type.
- Memory only. Bans do not survive a server restart.

> [!warning]
> You cannot lift a ban manually. Wait for it to expire.

If Ferron bans your own IP immediately, check the thresholds. `events 1`, `events 2`, or a short `window` such as 10 seconds are easy to trip. Increase `events` to require more violations, or lower `ban_duration` to shorten the ban.

The allowlist check runs before the ban check. Matching IPs skip ban checks entirely.

## Observability

### Metrics

| Metric                        | Type    | Attributes                                                                 | Description                       |
| ----------------------------- | ------- | -------------------------------------------------------------------------- | --------------------------------- |
| `ferron.abuseban.rejected`    | Counter | `ferron.abuseban.reason` (`rate_limit`, `brute_force`)                     | Requests rejected by an IP ban.   |
| `ferron.abuseban.triggered`   | Counter | `ferron.abuseban.reason` (`rate_limit`, `brute_force`, `error_rate`)       | Requests that triggered a ban.    |
| `ferron.abuseban.expired`     | Counter | `ferron.abuseban.reason`                                                   | Bans that expired.                |
| `ferron.abuseban.active_bans` | Gauge   | none                                                                       | Current number of active bans.    |

### Logs

| Event          | Level | Attributes                                                                                                   |
| -------------- | ----- | ------------------------------------------------------------------------------------------------------------ |
| Ban rejection  | DEBUG | `client.address`, `ferron.abuseban.reason`, `ferron.abuseban.remaining_secs`. Only when `log_rejections` is set. |
| Ban triggered  | WARN  | `client.address`, `ferron.abuseban.reason`                                                                   |
| Ban expired    | INFO  | `client.address`, `ferron.abuseban.reason`                                                                   |

`ferron.abuseban.reason` is `rate_limit`, `brute_force`, or `error_rate`, depending on the source. `ferron.abuseban.remaining_secs` counts the seconds left on the ban. `client.address` holds the client IP address.

### Access log fields

| Field                            | Type   | Description                                  |
| -------------------------------- | ------ | -------------------------------------------- |
| `ferron.abuseban.action`         | string | `skip` (not banned) or `rejected` (banned).  |
| `ferron.abuseban.reason`         | string | Ban reason. Set on rejection only.           |
| `ferron.abuseban.remaining_secs` | int    | Seconds remaining on the ban. Rejection only. |

### Trace spans

The abuse protection stage sets these attributes on its `ferron.stage.abuse_protection` span.

| Attribute                        | Type   | Description                                                            |
| -------------------------------- | ------ | ---------------------------------------------------------------------- |
| `ferron.abuseban.action`         | string | `skip` (not banned) or `rejected` (banned).                            |
| `ferron.abuseban.reason`         | string | Ban reason, when rejected.                                             |
| `ferron.abuseban.remaining_secs` | int    | Remaining ban duration in seconds, when rejected.                      |
| `error.type`                     | string | `ip_banned` when Ferron rejects the request. Highlights the span in the trace UI. |

## Best practices

`ferron doctor` reports these checks for directives on this page.

- `abuse_protection false`: disabling IP banning removes a layer of protection against repeated abuse. Keep it enabled unless another layer handles abusive clients.
- `allowlist` with a wildcard: exempting every source address applies only to known trusted clients.

## See also

- [Rate limiting](/docs/configuration/content/rate-limit)
- [HTTP basic authentication](/docs/configuration/security/basic-auth)
- [Routing and URL processing](/docs/configuration/routing/url-processing)
