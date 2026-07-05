---
title: Fail2ban integration
description: "Block abusive IPs at the firewall level by combining Ferron's access logs with Fail2ban filters and actions."
---

Ferron's access logs use an extended variant of Combined Log Format (which in turn is used by NGINX and Apache) by default, making them easy to parse with [Fail2ban](https://github.com/fail2ban/fail2ban). Fail2ban monitors log files, matches patterns with regex filters, and executes actions (typically `iptables` or `nftables` rules) to ban offending IPs at the firewall level.

Unlike Ferron's built-in [abuse protection](/docs/v3/use-cases/security/abuse-protection), which bans IPs in memory for a fixed duration, Fail2ban provides persistent bans managed by the OS firewall. This is useful when you want bans to survive Ferron restarts or when you need to block IPs across multiple services.

> [!note]
>
> - Ferron's built-in `abuse_protection` and external Fail2ban serve complementary purposes. Use `abuse_protection` for fast, in-process banning with fine-grained HTTP thresholds, and Fail2ban for firewall-level bans that persist across restarts.
> - If you only need temporary bans and do not require firewall integration, prefer `abuse_protection` — it is simpler and does not require external software.

## Prerequisites

Install Fail2ban on your system:

```bash
# Debian/Ubuntu
sudo apt install fail2ban

# RHEL/Fedora
sudo dnf install fail2ban
```

Ensure Ferron writes access logs to a file. The default text format works out of the box:

```ferron
example.com {
    log "/var/log/ferron/access.log"

    root /var/www/html
}
```

## Basic filter: ban after repeated 404 errors

Vulnerability scanners often generate large numbers of `404 Not Found` responses while probing for known exploits. This filter bans IPs that trigger too many 404s.

Create `/etc/fail2ban/filter.d/ferron-404.conf`:

```ini
[Definition]

failregex = ^<HOST> -.*"(GET|POST|HEAD|PUT|DELETE|PATCH) .* HTTP/\d\.\d" 404
ignoreregex =
```

Create `/etc/fail2ban/jail.d/ferron-404.conf`:

```ini
[ferron-404]
enabled  = true
filter   = ferron-404
logpath  = /var/log/ferron/access.log
maxretry = 20
findtime = 600
bantime  = 3600
action   = iptables-multiport[name=ferron-404, port="http,https"]
```

This bans IPs for **1 hour** after 20 `404` responses within 10 minutes.

## Ban repeated authentication failures

Detect brute-force login attempts by matching `401 Unauthorized` responses.

Create `/etc/fail2ban/filter.d/ferron-auth.conf`:

```ini
[Definition]

failregex = ^<HOST> -.*"(GET|POST|HEAD|PUT|DELETE|PATCH) .* HTTP/\d\.\d" 401
ignoreregex =
```

Create `/etc/fail2ban/jail.d/ferron-auth.conf`:

```ini
[ferron-auth]
enabled  = true
filter   = ferron-auth
logpath  = /var/log/ferron/access.log
maxretry = 5
findtime = 300
bantime  = 7200
action   = iptables-multiport[name=ferron-auth, port="http,https"]
```

This bans IPs for **2 hours** after 5 failed authentication attempts within 5 minutes.

## Ban rate-limited clients

When Ferron returns `429 Too Many Requests`, it means a client has exceeded rate limits. Ban IPs that repeatedly trigger rate limiting.

Create `/etc/fail2ban/filter.d/ferron-ratelimit.conf`:

```ini
[Definition]

failregex = ^<HOST> -.*"(GET|POST|HEAD|PUT|DELETE|PATCH) .* HTTP/\d\.\d" 429
ignoreregex =
```

Create `/etc/fail2ban/jail.d/ferron-ratelimit.conf`:

```ini
[ferron-ratelimit]
enabled  = true
filter   = ferron-ratelimit
logpath  = /var/log/ferron/access.log
maxretry = 10
findtime = 600
bantime  = 1800
action   = iptables-multiport[name=ferron-ratelimit, port="http,https"]
```

This bans IPs for **30 minutes** after 10 rate limit rejections within 10 minutes.

## Ban clients generating server errors

Detect clients that cause an unusual number of `5xx` responses (e.g., by sending malformed requests that crash backend applications).

Create `/etc/fail2ban/filter.d/ferron-5xx.conf`:

```ini
[Definition]

failregex = ^<HOST> -.*"(GET|POST|HEAD|PUT|DELETE|PATCH) .* HTTP/\d\.\d" 5\d\d
ignoreregex =
```

Create `/etc/fail2ban/jail.d/ferron-5xx.conf`:

```ini
[ferron-5xx]
enabled  = true
filter   = ferron-5xx
logpath  = /var/log/ferron/access.log
maxretry = 15
findtime = 600
bantime  = 3600
action   = iptables-multiport[name=ferron-5xx, port="http,https"]
```

## Ban scanners targeting specific paths

Detect automated scanners probing for common vulnerable paths (e.g., WordPress admin panels, phpMyAdmin, `.env` files).

Create `/etc/fail2ban/filter.d/ferron-scanner.conf`:

```ini
[Definition]

failregex = ^<HOST> -.*"(GET|POST|HEAD) /(wp-admin|wp-login|phpmyadmin|\.env|\.git|config\.php|admin\.php|xmlrpc\.php) HTTP/\d\.\d" (403|404)
ignoreregex =
```

Create `/etc/fail2ban/jail.d/ferron-scanner.conf`:

```ini
[ferron-scanner]
enabled  = true
filter   = ferron-scanner
logpath  = /var/log/ferron/access.log
maxretry = 5
findtime = 600
bantime  = 86400
action   = iptables-multiport[name=ferron-scanner, port="http,https"]
```

This bans IPs for **24 hours** after just 5 probe attempts within 10 minutes — scanner traffic is almost always malicious.

## Using nftables instead of iptables

If your system uses `nftables`, replace the `action` with the built-in nftables action:

```ini
action = nftables-multiport[name=ferron-404, port="http,https"]
```

Or use a custom nftables command:

```ini
action = nftables chain=filter name=ferron-404 protocol=tcp multiport=80,444
```

## Allowlisting trusted IPs

To prevent Fail2ban from banning trusted IPs (internal networks, monitoring systems), create `/etc/fail2ban/jail.d/ferron-ignore.conf`:

```ini
[DEFAULT]

# Trusted networks
ignoreip = 127.0.0.1/8 ::1 10.0.0.0/8 172.16.0.0/12 192.168.0.0/16

# Monitoring system
ignoreip = 203.0.113.50
```

> [!important]
> The `ignoreip` in `[DEFAULT]` applies to all jails. You can also set per-jail `ignoreip` values.

## Checking Fail2ban status

```bash
# Check all Ferron jails
sudo fail2ban-client status

# Check a specific jail
sudo fail2ban-client status ferron-404

# Manually unban an IP
sudo fail2ban-client set ferron-404 unbanip 203.0.113.50

# View banned IPs
sudo fail2ban-client banned
```

## Combining with Ferron's built-in abuse protection

For defense in depth, use both Ferron's `abuse_protection` and Fail2ban together:

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

    log "/var/log/ferron/access.log"

    root /var/www/html
}
```

The flow works as follows:

1. Ferron's `abuse_protection` provides fast, in-process banning for repeat offenders (bans expire after 15 minutes).
2. Fail2ban monitors the same access log and applies firewall-level bans for persistent abusers (bans can last hours or days).
3. An IP that triggers both systems gets banned twice — once by Ferron (HTTP 403 responses) and once by Fail2ban (dropped at the firewall).

> [!tip]
> Keep `abuse_protection` thresholds tighter (lower `events`, shorter `window`) and Fail2ban thresholds looser (higher `maxretry`, longer `bantime`). This way, Ferron handles quick reactions while Fail2ban handles sustained abuse.

## Custom access log format for better parsing

If you prefer a simpler log format that is easier for Fail2ban to parse, use the `access_pattern` directive to output standard Combined Log Format (without the extra Host and trace ID fields):

```ferron
example.com {
    log "/var/log/ferron/access.log" {
        format text
        access_pattern "%client_ip - %auth_user [%{%d/%b/%Y:%H:%M:%S %z}t] \"%method %path_and_query %version\" %status %content_length \"%{Referer}i\" \"%{User-Agent}i\""
    }

    root /var/www/html
}
```

This produces standard CLF output that all default Fail2ban NGINX/Apache filters can parse without modification.

## See also

- [Abuse protection](/docs/v3/use-cases/security/abuse-protection) — built-in in-memory IP banning
- [Rate limiting](/docs/v3/use-cases/security/rate-limiting) — token bucket rate limiting
- [Logging & observability](/docs/v3/use-cases/operations/logging-observability) — log configuration and formats
- [Configuration: abuse protection](/docs/v3/configuration/content/abuse-ban) — directive reference
