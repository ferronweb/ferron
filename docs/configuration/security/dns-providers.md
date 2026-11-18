---
title: "Configuration: DNS providers"
description: "Built-in DNS providers for the ACME DNS-01 challenge."
---

DNS providers solve the DNS-01 ACME challenge. DNS-01 is the only challenge type that supports wildcard certificates. Configure a provider in a `dns { }` block inside `tls { }`, and select it with `provider`.

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01
        contact admin@example.com
        dns {
            provider cloudflare
            api_key "{{env.CF_API_TOKEN}}"
            # provider-specific directives...
        }
    }
}
```

All providers are part of the `dns-stalwartlite` module, except `command`, which ships in the `dns-command` module.

## Provider names

All directives in this table are required strings with no default. All string directives accept `{{env.VAR}}` interpolation.

| Provider         | `provider`     | Directives                                                       |
| ---------------- | -------------- | ---------------------------------------------------------------- |
| Bunny.net        | `bunny`        | `api_key` (Bunny DNS API key)                                    |
| Cloudflare       | `cloudflare`   | `api_key` (scoped API token)                                     |
| deSEC            | `desec`        | `auth_token` (deSEC API token)                                   |
| DigitalOcean     | `digitalocean` | `auth_token` (personal access token)                             |
| DNSimple         | `dnsimple`     | `oauth_token`, `account_id`                                      |
| Google Cloud DNS | `googlecloud`  | `service_account_json`, `project_id` (optional settings below)   |
| OVH              | `ovh`          | `application_key`, `application_secret`, `consumer_key`, `endpoint` |
| Porkbun          | `porkbun`      | `api_key`, `secret_key`                                          |
| RFC 2136         | `rfc2136`      | `server`, `key_name`, `key_secret`, `key_algorithm`              |
| Route 53         | `route53`      | `access_key_id`, `secret_access_key` (optional settings below)   |
| Spaceship        | `spaceship`    | `api_key`, `api_secret`                                          |
| Command          | `command`      | `command`                                                        |

For Cloudflare, use a scoped API token.

## Google Cloud DNS

| Directive                     | Arguments  | Description                                                                            | Default         |
| ----------------------------- | ---------- | -------------------------------------------------------------------------------------- | --------------- |
| `service_account_json`        | `<string>` | Contents of the Google Cloud service account JSON key file.                            | none (required) |
| `project_id`                  | `<string>` | Google Cloud project ID.                                                               | none (required) |
| `managed_zone`                | `<string>` | Cloud DNS managed zone name. Ferron resolves the zone automatically if omitted.        | none (optional) |
| `private_zone`                | `<bool>`   | Set to `true` to target a private zone.                                                | `false`         |
| `impersonate_service_account` | `<string>` | Service account email to impersonate.                                                  | none (optional) |

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

## OVH

| Directive            | Arguments                                                                        | Description              | Default         |
| -------------------- | -------------------------------------------------------------------------------- | ------------------------ | --------------- |
| `application_key`    | `<string>`                                                                       | OVH application key.     | none (required) |
| `application_secret` | `<string>`                                                                       | OVH application secret.  | none (required) |
| `consumer_key`       | `<string>`                                                                       | OVH consumer key.        | none (required) |
| `endpoint`           | `ovh-eu`, `ovh-ca`, `kimsufi-eu`, `kimsufi-ca`, `soyoustart-eu`, `soyoustart-ca` | OVH API endpoint region. | none (required) |

Choose the endpoint that matches the domain location:

| Value           | Region                     |
| --------------- | -------------------------- |
| `ovh-eu`        | OVH Europe                 |
| `ovh-ca`        | OVH North America / Canada |
| `kimsufi-eu`    | Kimsufi Europe             |
| `kimsufi-ca`    | Kimsufi North America      |
| `soyoustart-eu` | So you Start Europe        |
| `soyoustart-ca` | So you Start North America |

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

## RFC 2136 (TSIG)

Updates records on any authoritative server that supports RFC 2136 dynamic updates authenticated with TSIG.

| Directive       | Arguments                                                                                                                                           | Description                                                                                      | Default         |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ | --------------- |
| `server`        | `<uri>`                                                                                                                                             | DNS server address as a URI with scheme `tcp` or `udp` (for example `udp://ns1.example.com:53`). | none (required) |
| `key_name`      | `<string>`                                                                                                                                          | TSIG key name.                                                                                   | none (required) |
| `key_secret`    | `<string>`                                                                                                                                          | TSIG key secret, Base64-encoded.                                                                 | none (required) |
| `key_algorithm` | `HMAC-MD5`, `GSS`, `HMAC-SHA1`, `HMAC-SHA224`, `HMAC-SHA256`, `HMAC-SHA256-128`, `HMAC-SHA384`, `HMAC-SHA384-192`, `HMAC-SHA512`, `HMAC-SHA512-256` | TSIG algorithm.                                                                                  | none (required) |

`key_secret` must be the raw TSIG key bytes encoded as standard Base64 with padding. BIND `tsig-keygen` and `dnssec-keygen` already output this format.

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider rfc2136
            server udp://ns1.example.com:53
            key_name "ferron-acme."
            key_secret "BASE64_ENCODED_TSIG_SECRET"
            key_algorithm "HMAC-SHA256"
        }
    }
}
```

## Route 53

| Directive           | Arguments  | Description                                                                 | Default         |
| ------------------- | ---------- | --------------------------------------------------------------------------- | --------------- |
| `access_key_id`     | `<string>` | AWS access key ID.                                                          | none (required) |
| `secret_access_key` | `<string>` | AWS secret access key.                                                      | none (required) |
| `region`            | `<string>` | AWS region (for example `us-east-1`).                                       | none (optional) |
| `session_token`     | `<string>` | AWS session token for temporary credentials.                                | none (optional) |
| `hosted_zone_id`    | `<string>` | Route 53 hosted zone ID. Ferron resolves the zone automatically if omitted. | none (optional) |
| `private_zone_only` | `<bool>`   | Set to `true` to target a private hosted zone only.                         | `false`         |
| `endpoint`          | `<string>` | Route 53 endpoint URL.                                                      | none (optional) |

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

## Command (external hook)

The `command` provider runs an external program for every DNS record change. It does not call a DNS API. Use it to delegate updates to a DNS server or automation that Ferron does not support natively. The program receives the record details in environment variables and must exit with status `0` to signal success.

| Directive | Arguments  | Description                                                                             | Default         |
| --------- | ---------- | --------------------------------------------------------------------------------------- | --------------- |
| `command` | `<string>` | Absolute path to the program to run for each record change.                             | none (required) |
| `min_ttl` | `<int>`    | Minimum TTL in seconds that Ferron accepts for records created by this provider.        | `60`            |

| Variable                  | Set on     | Description                                                   |
| ------------------------- | ---------- | ------------------------------------------------------------- |
| `FERRON_DNS_ACTION`       | always     | `add` for `update_record`, `delete` for `delete_record`.      |
| `FERRON_DNS_DOMAIN`       | always     | Full record name (for example `_acme-challenge.example.com`). |
| `FERRON_DNS_RECORD_TYPE`  | always     | Record type (for example `TXT`).                              |
| `FERRON_DNS_RECORD_VALUE` | `add` only | The record value (the ACME DNS challenge token).              |
| `FERRON_DNS_RECORD_TTL`   | `add` only | The requested TTL in seconds.                                 |

The program runs directly, without a shell, so arguments after the program path are not supported. Pass dynamic data through the environment variables. Exit status `0` means the change is applied. Any other status is an error.

> [!note]
> After the program creates the `_acme-challenge` TXT record, propagation time depends on your script and DNS server. Ferron cannot know the delay. Set a short TTL, or verify that the record resolves publicly before issuance retries.

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider command
            command /usr/local/bin/ferron-dns-hook.sh
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

## DNS propagation delays

After Ferron creates the `_acme-challenge` TXT record, the ACME CA must resolve it. Typical minimum TTLs:

| Provider                                                    | Typical minimum TTL |
| ----------------------------------------------------------- | ------------------- |
| `rfc2136`, `route53`                                        | 1 s                 |
| `bunny`                                                     | 15 s                |
| `digitalocean`                                              | 30 s                |
| `cloudflare`, `dnsimple`, `googlecloud`, `ovh`, `spaceship` | 60 s (1 min)        |
| `porkbun`                                                   | 600 s (10 min)      |
| `desec`                                                     | 3600 s (1 h)        |

If issuance fails with a DNS validation error, check that the TXT record resolves from the public internet before you retry.

## See also

- [ACME automatic TLS](/docs/configuration/security/acme): full ACME configuration reference
- [Automatic TLS use case](/docs/use-cases/https/automatic-tls): guided walkthrough

## Best practices

`ferron doctor` reports the following best-practice check for DNS provider directives.

- Secrets in plain configuration: DNS provider credentials (`api_key`, `secret`, `token`, and so on) should use environment variable interpolation (`{{env.VAR}}`) instead of plain strings. This avoids leaking secrets in version control or logs.
