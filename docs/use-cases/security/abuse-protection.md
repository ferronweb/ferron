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

### Log format

A log line looks like this:

```text
[2026-06-22 19:46:28.902 WARN] [trace=4dae55577f57aac23bdcffa24b38a31a] Ban triggered: IP ::1 - Custom abuse event: example_ban
```

- The `[trace=...]` block appears only when tracing is on.
- The IP address follows `IP `.
- The reason follows `-` and varies by event source in the table below:

| Source               | Example reason                       |
| -------------------- | ------------------------------------ |
| Rate limiting        | `Rate limit 10 req/s exceeded`       |
| Basic authentication | `Brute-force failure for user admin` |
| `abuse_event`        | `Custom abuse event: wordpress_scan` |
| Error rate threshold | `Error rate: 404 responses`          |

### Reason-to-category mapping

| Reason prefix         | AbuseIPDB category       | Category ID |
| --------------------- | ------------------------ | ----------- |
| `Rate limit`          | Web App Attack           | 21          |
| `Brute-force`         | Brute-Force              | 18          |
| `Custom abuse event:` | Web App Attack (default) | 21          |

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
    "Rate limit": 21,       # Web App Attack
    "Brute-force": 18,      # Brute-Force
}
DEFAULT_CATEGORY = 21

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

- AbuseIPDB API quotas apply.
- This script is best-effort. It does not retry failed reports or maintain a queue.
- Bans disappear when Ferron restarts, but the sidecar already reported them at this point.

## See also

- [Configuration: abuse protection](/docs/configuration/content/abuse-ban)
- [Rate limiting](/docs/use-cases/security/rate-limiting)
- [Fail2ban integration](/docs/use-cases/security/fail2ban)
- [HTTP basic authentication](/docs/configuration/security/basic-auth)
