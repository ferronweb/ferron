---
title: Abuse protection
description: "Protect Ferron from brute-force attacks, rate limit abuse, and other malicious behavior with temporary IP banning."
---

The `abuse_protection` directive of Ferron implements lightweight, Fail2ban-style IP banning. When a client exceeds configured thresholds, Ferron temporarily bans the client IP. Repeated rate limit breaches or failed login attempts are examples. Ferron stores bans in memory. They expire automatically after the configured duration.

This page covers common deployment patterns. For full configuration details, see [Configuration: abuse protection](/docs/v3/configuration/content/abuse-ban).

> [!important]
>
> - Bans exist in memory only and do not survive server restarts. No admin API exists for a manual unban, so you must wait for the ban to expire naturally.
> - If Ferron bans your IP immediately, check your thresholds. You may have `events 1` or very short `window` values that are too strict.
> - If Ferron bans legitimate clients, add their IP or CIDR range to the `allowlist`.
> - If Ferron runs behind a reverse proxy, configure `client_ip_from_header` so it sees the real client IP, not the proxy IP. See [HTTP host directives](/docs/v3/configuration/server/host).

## Basic abuse protection

Enable abuse protection with default thresholds on a host:

```ferron
example.com {
    abuse_protection

    root /var/www/html
}
```

With defaults, Ferron bans an IP for 15 minutes if:

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

This bans an IP for 30 minutes if:

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

This bans an IP for 5 minutes only after:

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

Ferron never bans IPs in the allowlist, even if they exceed thresholds. You can specify individual IPs or CIDR ranges. Use `allowlist` multiple times to add more entries.

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
2. Ferron records each rate limit breach as an abuse event.
3. If the client accumulates enough breaches within the window, Ferron bans the client IP.
4. While banned, the client receives a 403 Forbidden response with a `Retry-After` header.

## Detecting automated scans by URL pattern

Use `match` with a regex on `request.uri.path` combined with `abuse_event` to detect and ban automated scanners probing for vulnerable paths:

```ferron
example.com {
    # Anti-abuse
    abuse_protection {
        custom_threshold {
            events 5
            window "300s"
        }
    }

    # Detect automated scans targeting dynamic content, admin panels,
    # CMS paths, hidden files, and database files
    match AUTOMATED_SCAN {
        request.uri.path ~ ".*\\.(?:php|asp|jsp|cgi|sql)(?:\\b|$)|^/wp-(?:admin|login)(?:\\b|$)|^/\\.|^/(?:administrator|admin|login|logon)(?:\\b|$)"
    }

    if AUTOMATED_SCAN {
        abuse_event "automated_scan"
    }

    # Other directives go here...
    root /var/www/html
}
```

Requests matching this pattern trigger a custom abuse event. Ferron bans the client IP after 5 such events within 300 seconds. The ban duration follows the default or configured `ban_duration`.

## Detecting hostile scanning by error rate

Use `error_rate_threshold` to automatically ban IPs that generate an abnormal number of error responses. This detects hostile scanning behavior such as probing for old vulnerabilities or non-existent plugin paths:

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

Ferron bans an IP for 15 minutes after enough error responses. The threshold is 10 or more `404 Not Found` or `403 Forbidden` responses within 60 seconds. The threshold counts all matching status codes together. For example, 6 responses with 404 and 4 with 403 within the window would trigger the ban.

**Stricter threshold for vulnerability scanners:**

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

Ferron bans an IP for 1 hour after just 5 error responses within 30 seconds. The count includes `405 Method Not Allowed` responses.

## Disabling abuse protection

To disable abuse protection on a host:

```ferron
example.com {
    abuse_protection false

    root /var/www/html
}
```

## Reporting to AbuseIPDB

Ferron bans IPs in memory for a limited duration with its native abuse protection. For persistent threat intelligence sharing, you can run a lightweight sidecar. The sidecar tails the WARN-level logs of Ferron and reports banned IPs to [AbuseIPDB](https://www.abuseipdb.com/).

The sidecar watches for `Ban triggered` log lines and extracts the IP address and reason. It maps the reason to an [AbuseIPDB category](https://www.abuseipdb.com/categories) and posts a report to the AbuseIPDB API.

### Log format

The sidecar parses lines matching this pattern:

```text
[2026-06-22 19:46:28.902 WARN] [trace=4dae55577f57aac23bdcffa24b38a31a] Ban triggered: IP ::1 - Custom abuse event: example_ban
```

- The `[trace=...]` block is optional. It appears only when tracing is on.
- The IP address follows `IP `.
- The reason follows `-` and varies by event source:

| Source                         | Example reason                       |
| ------------------------------ | ------------------------------------ |
| Rate limiting                  | `Rate limit 10 req/s exceeded`       |
| Basic authentication           | `Brute-force failure for user admin` |
| Custom `abuse_event` directive | `Custom abuse event: wordpress_scan` |
| Error rate threshold           | `Error rate: 404 responses`          |

### Reason-to-category mapping

| Reason prefix         | AbuseIPDB category       | Category ID |
| --------------------- | ------------------------ | ----------- |
| `Rate limit`          | Web App Attack           | 14          |
| `Brute-force`         | Brute-Force              | 21          |
| `Custom abuse event:` | Web App Attack (default) | 14          |

### Sample script

The script reads the `ABUSEIPDB_API_KEY` environment variable. It tails the Ferron log file and reports each banned IP.

```python
#!/usr/bin/env python3
"""Tail Ferron's ban log and report IPs to AbuseIPDB."""

import os
import sys
import re
import time
import json
import argparse
import urllib.request
import urllib.error

ABUSEIPDB_URL = "https://api.abuseipdb.com/api/v2/report"

CATEGORY_MAP = {
    "Rate limit": 14,       # Web App Attack
    "Brute-force": 21,      # Brute-Force
}
DEFAULT_CATEGORY = 14

LOG_PATTERN = re.compile(
    r"^\[.* WARN\] (\[trace=[^\]]+\] )?Ban triggered: IP (\S+) - (.+)$"
)


def report_ip(api_key, ip, categories, comment):
    data = json.dumps({
        "ip": ip,
        "categories": categories,
        "comment": comment,
    }).encode()
    req = urllib.request.Request(
        ABUSEIPDB_URL,
        data=data,
        headers={
            "Key": api_key,
            "Content-Type": "application/json",
            "Accept": "application/json",
        },
    )
    try:
        with urllib.request.urlopen(req) as resp:
            return json.loads(resp.read())
    except urllib.error.HTTPError as e:
        print(f"AbuseIPDB API error: {e.code} {e.read().decode()}", file=sys.stderr)
        return None


def main():
    parser = argparse.ArgumentParser(
        description="Report Ferron banned IPs to AbuseIPDB"
    )
    parser.add_argument("logfile", help="Path to Ferron log file")
    parser.add_argument(
        "--api-key",
        help="AbuseIPDB API key (default: $ABUSEIPDB_API_KEY)",
    )
    args = parser.parse_args()

    api_key = args.api_key or os.environ.get("ABUSEIPDB_API_KEY")
    if not api_key:
        print(
            "Error: API key required via --api-key or ABUSEIPDB_API_KEY env var",
            file=sys.stderr,
        )
        sys.exit(1)

    with open(args.logfile) as f:
        f.seek(0, os.SEEK_END)
        while True:
            line = f.readline()
            if not line:
                time.sleep(0.5)
                continue
            m = LOG_PATTERN.match(line)
            if not m:
                continue
            ip = m.group(2).removeprefix("::ffff:")
            reason = m.group(3).strip()

            category = DEFAULT_CATEGORY
            for prefix, cat in CATEGORY_MAP.items():
                if reason.startswith(prefix):
                    category = cat
                    break

            print(f"Reporting {ip} ({reason}) -> AbuseIPDB category {category}")
            result = report_ip(
                api_key, ip, [category], f"Ferron abuse ban: {reason}"
            )
            if result:
                report_id = result.get("data", {}).get("ipReportId")
                print(f"  Reported: {report_id}")
            time.sleep(1)


if __name__ == "__main__":
    main()
```

### Deployment

**Systemd service**: run the sidecar alongside Ferron:

```ini
[Unit]
Description=Ferron AbuseIPDB reporter
After=ferron.service
BindsTo=ferron.service

[Service]
Type=simple
Environment=ABUSEIPDB_API_KEY=your_key_here
ExecStart=/usr/local/bin/ferron-abuseipdb /var/log/ferron/ferron.log
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

**Docker**: run the sidecar as a container. It shares the log volume or uses a logging driver that writes to a file.

### Limitations

- AbuseIPDB API daily quotas apply. Plan your thresholds accordingly.
- The script is best-effort. It does not retry failed reports or maintain a queue.
- There is no bidirectional sync. The sidecar cannot query or clear the internal ban state of Ferron.
- Bans disappear when Ferron restarts, but the sidecar already reported them by that point.

## See also

- [Configuration: abuse protection](/docs/v3/configuration/content/abuse-ban)
- [Rate limiting](/docs/v3/configuration/content/rate-limit)
- [HTTP basic authentication](/docs/v3/configuration/security/basic-auth)
