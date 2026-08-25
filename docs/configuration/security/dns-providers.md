---
title: "Configuration: DNS providers"
description: "Reference for all built-in DNS providers used with the ACME DNS-01 challenge."
---

The `tls-acme` module uses DNS providers to solve the DNS-01 ACME challenge. This is the only challenge type that supports wildcard certificates. You configure a provider inside the `dns { }` block nested within a `tls { }` block. Select it by name with the `provider` directive.

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01
        contact "admin@example.com"
        dns {
            provider cloudflare
            api_key "{{env.CF_API_TOKEN}}"
            # provider-specific directives...
        }
    }
}
```

All DNS provider implementations are currently part of the `dns-stalwartlite` module, except the `command` provider, which ships in the separate `dns-command` module.

## Providers

### Bunny.net

**Provider name:** `bunny`

| Directive | Arguments  | Description        | Default         |
| --------- | ---------- | ------------------ | --------------- |
| `api_key` | `<string>` | Bunny DNS API key. | none (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider bunny
            api_key "YOUR_BUNNY_API_KEY"
        }
    }
}
```

---

### Cloudflare

**Provider name:** `cloudflare`

| Directive | Arguments  | Description                          | Default         |
| --------- | ---------- | ------------------------------------ | --------------- |
| `api_key` | `<string>` | Cloudflare API token (scoped token). | none (required) |

**Configuration example:**

```ferron
*.example.com {
    # Scoped API token (recommended)
    tls {
        provider acme
        challenge dns-01

        dns {
            provider cloudflare
            api_key "YOUR_CLOUDFLARE_API_TOKEN"
        }
    }
}
```

---

### deSEC

**Provider name:** `desec`

| Directive    | Arguments  | Description      | Default         |
| ------------ | ---------- | ---------------- | --------------- |
| `auth_token` | `<string>` | deSEC API token. | none (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider desec
            auth_token "YOUR_DESEC_API_TOKEN"
        }
    }
}
```

---

### DigitalOcean

**Provider name:** `digitalocean`

| Directive    | Arguments  | Description                                       | Default         |
| ------------ | ---------- | ------------------------------------------------- | --------------- |
| `auth_token` | `<string>` | DigitalOcean personal access token (OAuth token). | none (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider digitalocean
            auth_token "YOUR_DO_OAUTH_TOKEN"
        }
    }
}
```

---

### DNSimple

**Provider name:** `dnsimple`

| Directive     | Arguments  | Description           | Default         |
| ------------- | ---------- | --------------------- | --------------- |
| `oauth_token` | `<string>` | DNSimple OAuth token. | none (required) |
| `account_id`  | `<string>` | DNSimple account ID.  | none (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider dnsimple
            oauth_token "YOUR_DNSIMPLE_TOKEN"
            account_id "12345"
        }
    }
}
```

---

### Google Cloud DNS

**Provider name:** `googlecloud`

| Directive                     | Arguments  | Description                                                                            | Default         |
| ----------------------------- | ---------- | -------------------------------------------------------------------------------------- | --------------- |
| `service_account_json`        | `<string>` | Contents of the Google Cloud service account JSON key file.                            | none (required) |
| `project_id`                  | `<string>` | Google Cloud project ID.                                                               | none (required) |
| `managed_zone`                | `<string>` | Name of the Cloud DNS managed zone. Ferron resolves the zone automatically if omitted. | none (optional) |
| `private_zone`                | `<bool>`   | Set to `true` to target a private zone.                                                | `false`         |
| `impersonate_service_account` | `<string>` | Service account email to impersonate.                                                  | none (optional) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider googlecloud
            service_account_json "{\"type\":\"service_account\", ...}"
            project_id "my-gcp-project"
            managed_zone "example-com"
        }
    }
}
```

---

### OVH

**Provider name:** `ovh`

| Directive            | Arguments                                                                        | Description              | Default         |
| -------------------- | -------------------------------------------------------------------------------- | ------------------------ | --------------- |
| `application_key`    | `<string>`                                                                       | OVH application key.     | none (required) |
| `application_secret` | `<string>`                                                                       | OVH application secret.  | none (required) |
| `consumer_key`       | `<string>`                                                                       | OVH consumer key.        | none (required) |
| `endpoint`           | `ovh-eu`, `ovh-ca`, `kimsufi-eu`, `kimsufi-ca`, `soyoustart-eu`, `soyoustart-ca` | OVH API endpoint region. | none (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider ovh
            application_key "YOUR_APP_KEY"
            application_secret "YOUR_APP_SECRET"
            consumer_key "YOUR_CONSUMER_KEY"
            endpoint "ovh-eu"
        }
    }
}
```

---

### Porkbun

**Provider name:** `porkbun`

| Directive    | Arguments  | Description             | Default         |
| ------------ | ---------- | ----------------------- | --------------- |
| `api_key`    | `<string>` | Porkbun API key.        | none (required) |
| `secret_key` | `<string>` | Porkbun secret API key. | none (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider porkbun
            api_key "YOUR_PORKBUN_API_KEY"
            secret_key "YOUR_PORKBUN_SECRET_KEY"
        }
    }
}
```

---

### RFC 2136 (TSIG)

**Provider name:** `rfc2136`

Updates DNS records on any authoritative server that supports dynamic updates (RFC 2136) authenticated with TSIG.

| Directive       | Arguments                                                                                                                                           | Description                                                                                      | Default         |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ | --------------- |
| `server`        | `<uri>`                                                                                                                                             | DNS server address as a URI with scheme `tcp` or `udp` (for example `udp://ns1.example.com:53`). | none (required) |
| `key_name`      | `<string>`                                                                                                                                          | TSIG key name.                                                                                   | none (required) |
| `key_secret`    | `<string>`                                                                                                                                          | TSIG key secret, Base64-encoded.                                                                 | none (required) |
| `key_algorithm` | `HMAC-MD5`, `GSS`, `HMAC-SHA1`, `HMAC-SHA224`, `HMAC-SHA256`, `HMAC-SHA256-128`, `HMAC-SHA384`, `HMAC-SHA384-192`, `HMAC-SHA512`, `HMAC-SHA512-256` | TSIG algorithm.                                                                                  | none (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider rfc2136
            server "udp://ns1.example.com:53"
            key_name "ferron-acme."
            key_secret "BASE64_ENCODED_TSIG_SECRET"
            key_algorithm "HMAC-SHA256"
        }
    }
}
```

---

### Route 53

**Provider name:** `route53`

| Directive           | Arguments  | Description                                                                 | Default         |
| ------------------- | ---------- | --------------------------------------------------------------------------- | --------------- |
| `access_key_id`     | `<string>` | AWS access key ID.                                                          | none (required) |
| `secret_access_key` | `<string>` | AWS secret access key.                                                      | none (required) |
| `region`            | `<string>` | AWS region (for example `us-east-1`).                                       | none (optional) |
| `session_token`     | `<string>` | AWS session token for temporary credentials.                                | none (optional) |
| `hosted_zone_id`    | `<string>` | Route 53 hosted zone ID. Ferron resolves the zone automatically if omitted. | none (optional) |
| `private_zone_only` | `<bool>`   | Set to `true` to target a private hosted zone only.                         | `false`         |
| `endpoint`          | `<string>` | Route 53 endpoint URL.                                                      | none (optional) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider route53
            access_key_id "AKIAIOSFODNN7EXAMPLE"
            secret_access_key "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
            region "us-east-1"
            hosted_zone_id "Z1D633PJN98FT9"
        }
    }
}
```

---

### Spaceship

**Provider name:** `spaceship`

| Directive    | Arguments  | Description           | Default         |
| ------------ | ---------- | --------------------- | --------------- |
| `api_key`    | `<string>` | Spaceship API key.    | none (required) |
| `api_secret` | `<string>` | Spaceship API secret. | none (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider spaceship
            api_key "YOUR_SPACESHIP_API_KEY"
            api_secret "YOUR_SPACESHIP_API_SECRET"
        }
    }
}
```

---

### Command (external hook)

**Provider name:** `command`

The `command` provider runs an external program for every DNS record change. It does not call any DNS API directly. Use it to delegate updates to any DNS server or automation that Ferron does not support natively. The program receives the record details through environment variables and must exit with status `0` to signal success.

| Directive | Arguments  | Description                                                                                     | Default |
| --------- | ---------- | ----------------------------------------------------------------------------------------------- | ------- |
| `command` | `<string>` | Absolute path to the program to run for each record change.                                     | none (required) |
| `min_ttl` | `<int>`    | Minimum TTL (in seconds) that Ferron will accept for records created by this provider.          | `60`    |

The provider passes the following environment variables to the program:

| Variable                  | Set on        | Description                                                        |
| ------------------------- | ------------- | ------------------------------------------------------------------ |
| `FERRON_DNS_ACTION`       | always        | `add` for `update_record`, `delete` for `delete_record`.           |
| `FERRON_DNS_DOMAIN`       | always        | Full record name (for example `_acme-challenge.example.com`).      |
| `FERRON_DNS_RECORD_TYPE`  | always        | Record type (for example `TXT`).                                   |
| `FERRON_DNS_RECORD_VALUE` | `add` only    | The record value (the ACME DNS challenge token).                  |
| `FERRON_DNS_RECORD_TTL`   | `add` only    | The requested TTL in seconds.                                      |

The program runs directly (no shell), so arguments beyond the program path are not supported. Pass dynamic data through the environment variables. The program's exit status determines success: `0` means the change is applied, any other status is treated as an error.

> [!note]
> After the program creates the `_acme-challenge` TXT record, propagation time depends entirely on your script and DNS server. Ferron cannot know the delay, so verify the record is publicly resolvable before issuance retries, or set a short TTL.

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider command
            command "/usr/local/bin/ferron-dns-hook.sh"
            min_ttl 30
        }
    }
}
```

A minimal hook script:

```sh
#!/bin/sh
# Example hook: log the requested change. Replace with your real DNS update.
echo "$(date) $FERRON_DNS_ACTION $FERRON_DNS_RECORD_TYPE $FERRON_DNS_DOMAIN $FERRON_DNS_RECORD_VALUE" >> /var/log/ferron-dns.log
exit 0
```

---

## Usage notes

### Using environment variables for credentials

All string directives support environment variable interpolation. This keeps secrets out of your configuration file:

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider cloudflare
            api_key "{{env.CF_API_TOKEN}}"
        }
    }
}
```

### DNS propagation delays

After Ferron creates the `_acme-challenge` TXT record, the ACME CA must be able to resolve it. Propagation time varies by provider:

| Provider                                                    | Typical minimum TTL |
| ----------------------------------------------------------- | ------------------- |
| `rfc2136`, `route53`                                        | 1 s                 |
| `bunny`                                                     | 15 s                |
| `digitalocean`                                              | 30 s                |
| `cloudflare`, `dnsimple`, `googlecloud`, `ovh`, `spaceship` | 60 s (1 min)        |
| `porkbun`                                                   | 600 s (10 min)      |
| `desec`                                                     | 3600 s (1 h)        |

If certificate issuance fails with a DNS validation error, verify that the TXT record is resolvable from the public internet before retrying.

### RFC 2136 TSIG key format

The `key_secret` value must be the raw TSIG key bytes encoded as standard Base64 (with padding). Most DNS management tools (BIND `tsig-keygen`, `dnssec-keygen`) output the key in this format already.

### OVH endpoint selection

Choose the `endpoint` that matches where your domain resides:

| Value           | Region                     |
| --------------- | -------------------------- |
| `ovh-eu`        | OVH Europe                 |
| `ovh-ca`        | OVH North America / Canada |
| `kimsufi-eu`    | Kimsufi Europe             |
| `kimsufi-ca`    | Kimsufi North America      |
| `soyoustart-eu` | So you Start Europe        |
| `soyoustart-ca` | So you Start North America |

## See also

- [ACME automatic TLS](/docs/v3/configuration/security/acme): full ACME configuration reference
- [Automatic TLS use case](/docs/v3/use-cases/security/automatic-tls): guided walkthrough

## Best practices

`ferron doctor` reports the following best-practice check for DNS provider directives.

- **Secrets in plain configuration.** DNS provider credentials (`api_key`, `secret`, `token`, etc.) should use environment variable interpolation (`{{env.VAR}}`) rather than plain strings. This avoids leaking secrets in version control or logs.
