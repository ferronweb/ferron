---
title: "Configuration: DNS providers"
description: "Reference for all built-in DNS providers used with the ACME DNS-01 challenge."
---

DNS providers are used by the `tls-acme` module to solve the **DNS-01 ACME challenge** — the only challenge type that supports wildcard certificates. You configure a provider inside the `dns { }` block nested within a `tls { }` block, selecting it by name with the `provider` directive.

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01
        contact "admin@example.com"
        dns {
            provider "<provider-name>"
            # provider-specific directives …
        }
    }
}
```

All DNS provider implementations are part of the `dns-stalwart` module.

## Providers

### Bunny

**Provider name:** `bunny`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | Bunny DNS API key. | — (required) |

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

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | Cloudflare API token (scoped token) or global API key. | — (required) |
| `email` | `<string>` | Account email address. Required when using a global API key; omit for scoped tokens. | — (optional) |

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

    # Global API key
    tls {
        provider acme
        challenge dns-01

        dns {
            provider cloudflare
            api_key "YOUR_GLOBAL_API_KEY"
            email "admin@example.com"
        }
    }
}
```

---

### deSEC

**Provider name:** `desec`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_token` | `<string>` | deSEC API token. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider desec
            api_token "YOUR_DESEC_API_TOKEN"
        }
    }
}
```

---

### DigitalOcean

**Provider name:** `digitalocean`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `oauth_token` | `<string>` | DigitalOcean personal access token (OAuth token). | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider digitalocean
            oauth_token "YOUR_DO_OAUTH_TOKEN"
        }
    }
}
```

---

### DNSimple

**Provider name:** `dnsimple`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `oauth_token` | `<string>` | DNSimple OAuth token. | — (required) |
| `account_id` | `<string>` | DNSimple account ID. | — (required) |

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

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `service_account_json` | `<string>` | Contents of the Google Cloud service account JSON key file. | — (required) |
| `project_id` | `<string>` | Google Cloud project ID. | — (required) |
| `managed_zone` | `<string>` | Name of the Cloud DNS managed zone. Ferron resolves the zone automatically if omitted. | — (optional) |
| `private_zone` | `<bool>` | Set to `true` to target a private zone. | `false` |
| `impersonate_service_account` | `<string>` | Service account email to impersonate. | — (optional) |

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

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `application_key` | `<string>` | OVH application key. | — (required) |
| `application_secret` | `<string>` | OVH application secret. | — (required) |
| `consumer_key` | `<string>` | OVH consumer key. | — (required) |
| `endpoint` | `ovh-eu`, `ovh-ca`, `kimsufi-eu`, `kimsufi-ca`, `soyoustart-eu`, `soyoustart-ca` | OVH API endpoint region. | — (required) |

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

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | Porkbun API key. | — (required) |
| `secret_key` | `<string>` | Porkbun secret API key. | — (required) |

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

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `server` | `<uri>` | DNS server address as a URI with scheme `tcp` or `udp` (e.g. `udp://ns1.example.com:53`). | — (required) |
| `key_name` | `<string>` | TSIG key name. | — (required) |
| `key_secret` | `<string>` | TSIG key secret, Base64-encoded. | — (required) |
| `key_algorithm` | `HMAC-MD5`, `GSS`, `HMAC-SHA1`, `HMAC-SHA224`, `HMAC-SHA256`, `HMAC-SHA256-128`, `HMAC-SHA384`, `HMAC-SHA384-192`, `HMAC-SHA512`, `HMAC-SHA512-256` | TSIG algorithm. | — (required) |

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

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `access_key_id` | `<string>` | AWS access key ID. | — (required) |
| `secret_access_key` | `<string>` | AWS secret access key. | — (required) |
| `region` | `<string>` | AWS region (e.g. `us-east-1`). | — (optional) |
| `session_token` | `<string>` | AWS session token for temporary credentials. | — (optional) |
| `hosted_zone_id` | `<string>` | Route 53 hosted zone ID. Ferron resolves the zone automatically if omitted. | — (optional) |
| `private_zone_only` | `<bool>` | Set to `true` to target a private hosted zone only. | `false` |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            rovider route53
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

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | Spaceship API key. | — (required) |
| `api_secret` | `<string>` | Spaceship API secret. | — (required) |

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

## Notes and troubleshooting

### Using environment variables for credentials

All string directives support environment variable interpolation. This avoids storing secrets directly in your configuration file:

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

| Provider | Typical minimum TTL |
|----------|-------------------|
| `bunny` | 15 s |
| `rfc2136` | 1 s |
| `route53` | 1 s |
| `spaceship` | 20 min |
| `desec` | 1 h |
| `cloudflare`, `dnsimple`, `googlecloud`, `ovh` | 60 s |
| `digitalocean` | 30 s |
| `porkbun` | 10 min |

If certificate issuance fails with a DNS validation error, verify that the TXT record is resolvable from the public internet before retrying.

### RFC 2136 TSIG key format

The `key_secret` value must be the raw TSIG key bytes encoded as **standard Base64** (with padding). Most DNS management tools (BIND `tsig-keygen`, `dnssec-keygen`) output the key in this format already.

### OVH endpoint selection

Choose the `endpoint` that matches where your domain is registered:

| Value | Region |
|-------|--------|
| `ovh-eu` | OVH Europe |
| `ovh-ca` | OVH North America / Canada |
| `kimsufi-eu` | Kimsufi Europe |
| `kimsufi-ca` | Kimsufi North America |
| `soyoustart-eu` | So you Start Europe |
| `soyoustart-ca` | So you Start North America |

## See also

- [ACME automatic TLS](/docs/v3/configuration/tls-acme) — full ACME configuration reference
- [Automatic TLS use case](/docs/v3/use-cases/automatic-tls) — guided walkthrough
