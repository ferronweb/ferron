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
            provider cloudflare
            api_key "{{env.CF_API_TOKEN}}"
            # provider-specific directives...
        }
    }
}
```

All DNS provider implementations are currently part of the `dns-stalwart` module.

## Providers

### Alibaba Cloud DNS

**Provider name:** `alidns`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `access_key_id` | `<string>` | Alibaba Cloud AccessKey ID. | — (required) |
| `access_key_secret` | `<string>` | Alibaba Cloud AccessKey secret. | — (required) |
| `region` | `<string>` | Alibaba Cloud region. | — (optional) |
| `security_token` | `<string>` | STS security token for temporary credentials. | — (optional) |
| `line` | `<string>` | DNS line/zone identifier. | — (optional) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider alidns
            access_key_id "YOUR_ALIBABA_ACCESS_KEY_ID"
            access_key_secret "YOUR_ALIBABA_ACCESS_KEY_SECRET"
            region "cn-hangzhou"
        }
    }
}
```

---

### ArvanCloud

**Provider name:** `arvancloud`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | ArvanCloud API key. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider arvancloud
            api_key "YOUR_ARVANCLOUD_API_KEY"
        }
    }
}
```

---

### AutoDNS

**Provider name:** `autodns`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `username` | `<string>` | AutoDNS username. | — (required) |
| `password` | `<string>` | AutoDNS password. | — (required) |
| `context` | `<number>` | AutoDNS context ID. | — (optional) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider autodns
            username "YOUR_AUTODNS_USERNAME"
            password "YOUR_AUTODNS_PASSWORD"
        }
    }
}
```

---

### Azure DNS

**Provider name:** `azuredns`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `tenant_id` | `<string>` | Azure tenant ID. | — (required) |
| `client_id` | `<string>` | Azure client/application ID. | — (required) |
| `client_secret` | `<string>` | Azure client secret. | — (required) |
| `subscription_id` | `<string>` | Azure subscription ID. | — (required) |
| `resource_group` | `<string>` | Azure resource group name. | — (required) |
| `endpoint` | `AzurePublicCloud`, `AzureChinaCloud`, `AzureUSGovernment` | Azure environment. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider azuredns
            tenant_id "YOUR_TENANT_ID"
            client_id "YOUR_CLIENT_ID"
            client_secret "YOUR_CLIENT_SECRET"
            subscription_id "YOUR_SUBSCRIPTION_ID"
            resource_group "example-rg"
            endpoint "AzurePublicCloud"
        }
    }
}
```

---

### Baidu Cloud DNS

**Provider name:** `baiducloud`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `access_key_id` | `<string>` | Baidu Cloud AccessKey ID. | — (required) |
| `access_key_secret` | `<string>` | Baidu Cloud AccessKey secret. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider baiducloud
            access_key_id "YOUR_BAIDU_ACCESS_KEY_ID"
            access_key_secret "YOUR_BAIDU_ACCESS_KEY_SECRET"
        }
    }
}
```

---

### BlueCat Address Manager v2

**Provider name:** `bluecatv2`

Updates DNS records on BlueCat Address Manager v2 via its REST API.

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `server_url` | `<string>` | BlueCat gateway URL. | — (required) |
| `username` | `<string>` | BlueCat username. | — (required) |
| `password` | `<string>` | BlueCat password. | — (required) |
| `config_name` | `<string>` | BlueCat configuration name. | — (required) |
| `view_name` | `<string>` | DNS view name. | — (required) |
| `skip_deploy` | `<bool>` | Skip the deployment step after updating records. | `false` |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider bluecatv2
            server_url "https://bluecat.example.com:8443/gateway-api"
            username "YOUR_BLUECAT_USERNAME"
            password "YOUR_BLUECAT_PASSWORD"
            config_name "production"
            view_name "external"
        }
    }
}
```

---

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

### ClouDNS

**Provider name:** `cloudns`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `auth_id` | `<string>` | ClouDNS Auth ID. | — (optional) |
| `sub_auth_id` | `<string>` | ClouDNS sub-auth ID. | — (optional) |
| `password` | `<string>` | ClouDNS password (required if `auth_id`/`sub_auth_id` not used). | — (optional) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider cloudns
            auth_id "YOUR_CLOUDNS_AUTH_ID"
            password "YOUR_CLOUDNS_PASSWORD"
        }
    }
}
```

---

### Constellix

**Provider name:** `constellix`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | Constellix API key. | — (required) |
| `secret_key` | `<string>` | Constellix secret key. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider constellix
            api_key "YOUR_CONSTELLIX_API_KEY"
            secret_key "YOUR_CONSTELLIX_SECRET_KEY"
        }
    }
}
```

---

### cPanel

**Provider name:** `cpanel`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `base_url` | `<string>` | cPanel host URL (e.g. `https://example.com:2083`). | — (required) |
| `username` | `<string>` | cPanel account username. | — (required) |
| `token` | `<string>` | cPanel API token. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider cpanel
            base_url "https://example.com:2083"
            username "YOUR_CPNL_USERNAME"
            token "YOUR_CPNL_API_TOKEN"
        }
    }
}
```

---

### Cloudflare

**Provider name:** `cloudflare`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | Cloudflare API token (scoped token). | — (required) |

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

### DDNSS.de

**Provider name:** `ddnss`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `key` | `<string>` | DDNSS.de API key. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider ddnss
            key "YOUR_DDNSS_API_KEY"
        }
    }
}
```

---

### deSEC

**Provider name:** `desec`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `auth_token` | `<string>` | deSEC API token. | — (required) |

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

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `auth_token` | `<string>` | DigitalOcean personal access token (OAuth token). | — (required) |

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

### DNS Made Easy

**Provider name:** `dnsmadeeasy`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | DNS Made Easy API key. | — (required) |
| `api_secret` | `<string>` | DNS Made Easy API secret. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider dnsmadeeasy
            api_key "YOUR_DNSMADEEASY_API_KEY"
            api_secret "YOUR_DNSMADEEASY_API_SECRET"
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

### Domeneshop

**Provider name:** `domeneshop`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_token` | `<string>` | Domeneshop API token. | — (required) |
| `api_secret` | `<string>` | Domeneshop API secret. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider domeneshop
            api_token "YOUR_DOMENESHOP_API_TOKEN"
            api_secret "YOUR_DOMENESHOP_API_SECRET"
        }
    }
}
```

---

### DreamHost

**Provider name:** `dreamhost`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | DreamHost API key. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider dreamhost
            api_key "YOUR_DREAMHOST_API_KEY"
        }
    }
}
```

---

### DuckDNS

**Provider name:** `duckdns`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `token` | `<string>` | DuckDNS account token. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider duckdns
            token "YOUR_DUCKDNS_TOKEN"
        }
    }
}
```

---

### Dynu

**Provider name:** `dynu`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | Dynu API key. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider dynu
            api_key "YOUR_DYNU_API_KEY"
        }
    }
}
```

---

### EasyDNS

**Provider name:** `easydns`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `token` | `<string>` | EasyDNS API token. | — (required) |
| `key` | `<string>` | EasyDNS API key. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider easydns
            token "YOUR_EASYDNS_TOKEN"
            key "YOUR_EASYDNS_API_KEY"
        }
    }
}
```

---

### Akamai Edge DNS

**Provider name:** `edgedns`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `host` | `<string>` | Edge DNS server hostname. | — (required) |
| `client_token` | `<string>` | Edge DNS client token. | — (required) |
| `client_secret` | `<string>` | Edge DNS client secret. | — (required) |
| `access_token` | `<string>` | Edge DNS access token. | — (required) |
| `account_switch_key` | `<string>` | Account switch key for multi-account setups. | — (optional) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider edgedns
            host "edgeapi.akamai.com"
            client_token "YOUR_CLIENT_TOKEN"
            client_secret "YOUR_CLIENT_SECRET"
            access_token "YOUR_ACCESS_TOKEN"
        }
    }
}
```

---

### Exoscale

**Provider name:** `exoscale`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | Exoscale API key. | — (required) |
| `api_secret` | `<string>` | Exoscale API secret. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider exoscale
            api_key "YOUR_EXOSCALE_API_KEY"
            api_secret "YOUR_EXOSCALE_API_SECRET"
        }
    }
}
```

---

### FreeMyIP

**Provider name:** `freemyip`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `token` | `<string>` | FreeMyIP token. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider freemyip
            token "YOUR_FREEMYIP_TOKEN"
        }
    }
}
```

---

### Gandi v5

**Provider name:** `gandiv5`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `personal_access_token` | `<string>` | Gandi v5 personal access token. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider gandiv5
            personal_access_token "YOUR_GANDI_PAT"
        }
    }
}
```

---

### Gcore

**Provider name:** `gcore`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_token` | `<string>` | Gcore API token. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider gcore
            api_token "YOUR_GCORE_API_TOKEN"
        }
    }
}
```

---

### GleSYS

**Provider name:** `glesys`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_user` | `<string>` | GleSYS API username. | — (required) |
| `api_key` | `<string>` | GleSYS API key. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider glesys
            api_user "YOUR_GLESYS_API_USER"
            api_key "YOUR_GLESYS_API_KEY"
        }
    }
}
```

---

### GoDaddy

**Provider name:** `godaddy`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | GoDaddy API key. | — (required) |
| `api_secret` | `<string>` | GoDaddy API secret. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider godaddy
            api_key "YOUR_GODADDY_API_KEY"
            api_secret "YOUR_GODADDY_API_SECRET"
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

### Hetzner DNS

**Provider name:** `hetzner`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_token` | `<string>` | Hetzner DNS API token. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider hetzner
            api_token "YOUR_HETZNER_API_TOKEN"
        }
    }
}
```

---

### hosting.de

**Provider name:** `hostingde`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | Hosting.de API key. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider hostingde
            api_key "YOUR_HOSTINGDE_API_KEY"
        }
    }
}
```

---

### Hostinger

**Provider name:** `hostinger`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_token` | `<string>` | Hostinger API token. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider hostinger
            api_token "YOUR_HOSTINGER_API_TOKEN"
        }
    }
}
```

---

### Huawei Cloud DNS

**Provider name:** `huaweicloud`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `access_key_id` | `<string>` | Huawei Cloud AccessKey ID. | — (required) |
| `access_key_secret` | `<string>` | Huawei Cloud AccessKey secret. | — (required) |
| `region` | `<string>` | Huawei Cloud region. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider huaweicloud
            access_key_id "YOUR_HUAWEI_ACCESS_KEY_ID"
            access_key_secret "YOUR_HUAWEI_ACCESS_KEY_SECRET"
            region "cn-north-1"
        }
    }
}
```

---

### Hurricane Electric

**Provider name:** `hurricane`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `credentials` | `<string>` | Comma-separated key=value pairs (e.g. `domain1=ip1,domain2=ip2`). | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider hurricane
            credentials "example.com=192.0.2.1,mail.example.com=192.0.2.2"
        }
    }
}
```

---

### IBM Cloud

**Provider name:** `ibmcloud`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `username` | `<string>` | IBM Cloud username. | — (required) |
| `api_key` | `<string>` | IBM Cloud API key. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider ibmcloud
            username "YOUR_IBM_USERNAME"
            api_key "YOUR_IBM_API_KEY"
        }
    }
}
```

---

### Infoblox NIOS

**Provider name:** `infoblox`

Updates DNS records on Infoblox NIOS via its WAPI REST API.

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `host` | `<string>` | Infoblox NIOS server hostname. | — (required) |
| `username` | `<string>` | Infoblox username. | — (required) |
| `password` | `<string>` | Infoblox password. | — (required) |
| `port` | `<string>` | Infoblox WAPI port. | — (optional) |
| `wapi_version` | `<string>` | WAPI API version. | — (optional) |
| `dns_view` | `<string>` | DNS view name. | — (optional) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider infoblox
            host "infoblox.example.com"
            username "YOUR_INFOBLOX_USERNAME"
            password "YOUR_INFOBLOX_PASSWORD"
            wapi_version "2.11"
            dns_view "default"
        }
    }
}
```

---

### Infomaniak

**Provider name:** `infomaniak`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_token` | `<string>` | Infomaniak API token. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider infomaniak
            api_token "YOUR_INFOMANIAK_API_TOKEN"
        }
    }
}
```

---

### INWX

**Provider name:** `inwx`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `username` | `<string>` | INWX username. | — (required) |
| `password` | `<string>` | INWX password. | — (required) |
| `sandbox` | `<bool>` | Use sandbox environment. | `false` |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider inwx
            username "YOUR_INWX_USERNAME"
            password "YOUR_INWX_PASSWORD"
            sandbox "false"
        }
    }
}
```

---

### IONOS

**Provider name:** `ionos`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | IONOS API key. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider ionos
            api_key "YOUR_IONOS_API_KEY"
        }
    }
}
```

---

### IPv64

**Provider name:** `ipv64`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | IPv64 API key. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider ipv64
            api_key "YOUR_IPV64_API_KEY"
        }
    }
}
```

---

### Joker

**Provider name:** `joker`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | Joker API key (alternative to username/password). | — (optional) |
| `username` | `<string>` | Joker username (alternative to api_key). | — (optional) |
| `password` | `<string>` | Joker password (alternative to api_key). | — (optional) |

> [!note]
> Either `api_key` **or** the pair `username` + `password` is required.

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider joker
            api_key "YOUR_JOKER_API_KEY"
        }
    }
}
```

---

### AWS Lightsail

**Provider name:** `lightsail`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `access_key_id` | `<string>` | AWS AccessKey ID. | — (required) |
| `secret_access_key` | `<string>` | AWS SecretAccessKey. | — (required) |
| `region` | `<string>` | AWS region. | — (optional) |
| `session_token` | `<string>` | AWS session token (STS). | — (optional) |
| `domain` | `<string>` | Domain name filter. | — (optional) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider lightsail
            access_key_id "YOUR_LIGHTSAIL_ACCESS_KEY_ID"
            secret_access_key "YOUR_LIGHTSAIL_SECRET_ACCESS_KEY"
            region "us-east-1"
            domain "example.com"
        }
    }
}
```

---

### Linode

**Provider name:** `linode`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_token` | `<string>` | Linode API v4 token. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider linode
            api_token "YOUR_LINODE_API_TOKEN"
        }
    }
}
```

---

### LuaDNS

**Provider name:** `luadns`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_username` | `<string>` | LuaDNS API username. | — (required) |
| `api_token` | `<string>` | LuaDNS API token. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider luadns
            api_username "YOUR_LUADNS_API_USERNAME"
            api_token "YOUR_LUADNS_API_TOKEN"
        }
    }
}
```

---

### Mythic Beasts

**Provider name:** `mythicbeasts`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `username` | `<string>` | Mythic Beasts username. | — (required) |
| `password` | `<string>` | Mythic Beasts password. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider mythicbeasts
            username "YOUR_MYTHIC_USERNAME"
            password "YOUR_MYTHIC_PASSWORD"
        }
    }
}
```

---

### Namecheap

**Provider name:** `namecheap`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | Namecheap API key. | — (required) |
| `api_secret` | `<string>` | Namecheap API secret. | — (required) |
| `client_ip` | `<string>` | Client IP address (used for API access control / IP restriction). | — (required) |
| `username` | `<string>` | Namecheap username. | — (optional) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider namecheap
            api_key "YOUR_NAMECHEAP_API_KEY"
            api_secret "YOUR_NAMECHEAP_API_SECRET"
            client_ip "YOUR_CLIENT_IP"
        }
    }
}
```

---

### Name.com

**Provider name:** `namedotcom`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `username` | `<string>` | Name.com username. | — (required) |
| `api_token` | `<string>` | Name.com API token. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider namedotcom
            username "YOUR_NAMEDOTCOM_USERNAME"
            api_token "YOUR_NAMEDOTCOM_API_TOKEN"
        }
    }
}
```

---

### NameSilo

**Provider name:** `namesilo`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_token` | `<string>` | NameSilo API token. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider namesilo
            api_token "YOUR_NAMESILO_API_TOKEN"
        }
    }
}
```

---

### netcup

**Provider name:** `netcup`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `customer_number` | `<string>` | Netcup customer number. | — (required) |
| `api_key` | `<string>` | Netcup API key. | — (required) |
| `api_password` | `<string>` | Netcup API password. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider netcup
            customer_number "YOUR_NETCUP_CUSTOMER_NUMBER"
            api_key "YOUR_NETCUP_API_KEY"
            api_password "YOUR_NETCUP_API_PASSWORD"
        }
    }
}
```

---

### Netlify

**Provider name:** `netlify`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `access_token` | `<string>` | Netlify access token. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider netlify
            access_token "YOUR_NETLIFY_ACCESS_TOKEN"
        }
    }
}
```

---

### Nifcloud

**Provider name:** `nifcloud`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | Nifty Cloud API key. | — (required) |
| `api_secret` | `<string>` | Nifty Cloud API secret. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider nifcloud
            api_key "YOUR_NIFCLOUD_API_KEY"
            api_secret "YOUR_NIFCLOUD_API_SECRET"
        }
    }
}
```

---

### NS1

**Provider name:** `ns1`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | NS1 API key. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider ns1
            api_key "YOUR_NS1_API_KEY"
        }
    }
}
```

---

### Oracle Cloud DNS

**Provider name:** `oraclecloud`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `tenancy_ocid` | `<string>` | Oracle tenancy OCID. | — (required) |
| `user_ocid` | `<string>` | Oracle user OCID. | — (required) |
| `compartment_ocid` | `<string>` | Oracle compartment OCID. | — (required) |
| `region` | `<string>` | Oracle Cloud region. | — (required) |
| `fingerprint` | `<string>` | API fingerprint. | — (required) |
| `private_key_pem` | `<string>` | Private key in PEM format. | — (required) |
| `private_key_password` | `<string>` | Private key password (if encrypted). | — (optional) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider oraclecloud
            tenancy_ocid "ocid1.tenancy.oc1..example"
            user_ocid "ocid1.user.oc1..example"
            compartment_ocid "ocid1.compartment.oc1..example"
            region "us-phoenix-1"
            fingerprint "YOUR_FINGERPRINT"
            private_key_pem "-----BEGIN PRIVATE KEY-----\n..."
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

### Plesk

**Provider name:** `plesk`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `base_url` | `<string>` | Plesk server URL. | — (required) |
| `api_key` | `<string>` | Plesk API key. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider plesk
            base_url "https://plesk.example.com:8443"
            api_key "YOUR_PLESK_API_KEY"
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

### ANS SafeDNS

**Provider name:** `safedns`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `auth_token` | `<string>` | SafeDNS authentication token. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider safedns
            auth_token "YOUR_SAFEDNS_AUTH_TOKEN"
        }
    }
}
```

---

### Scaleway

**Provider name:** `scaleway`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_token` | `<string>` | Scaleway API token. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider scaleway
            api_token "YOUR_SCALEWAY_API_TOKEN"
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

---

### Tencent Cloud DNSPod

**Provider name:** `tencentcloud`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `secret_id` | `<string>` | Tencent Cloud SecretId. | — (required) |
| `secret_key` | `<string>` | Tencent Cloud SecretKey. | — (required) |
| `region` | `<string>` | Tencent Cloud region. | — (optional) |
| `session_token` | `<string>` | Temporary session token. | — (optional) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider tencentcloud
            secret_id "YOUR_TENCENT_SECRET_ID"
            secret_key "YOUR_TENCENT_SECRET_KEY"
            region "ap-guangzhou"
        }
    }
}
```

---

### TransIP

**Provider name:** `transip`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `login` | `<string>` | TransIP account login. | — (required) |
| `private_key_pem` | `<string>` | Private key in PEM format. | — (required) |
| `global_key` | `<boolean>` | Use global key for authentication. | `false` |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider transip
            login "YOUR_TRANSIP_LOGIN"
            private_key_pem "-----BEGIN PRIVATE KEY-----\n..."
            global_key true
        }
    }
}
```

---

### UltraDNS

**Provider name:** `ultradns`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `username` | `<string>` | UltraDNS username. | — (required) |
| `password` | `<string>` | UltraDNS password. | — (required) |
| `endpoint` | `<string>` | Custom endpoint URL. | — (optional) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider ultradns
            username "YOUR_ULTRADNS_USERNAME"
            password "YOUR_ULTRADNS_PASSWORD"
        }
    }
}
```

---

### Vercel

**Provider name:** `vercel`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `auth_token` | `<string>` | Vercel auth token. | — (required) |
| `team_id` | `<string>` | Team ID for team-managed DNS. | — (optional) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider vercel
            auth_token "YOUR_VERCEL_AUTH_TOKEN"
        }
    }
}
```

---

### Vultr

**Provider name:** `vultr`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | Vultr API key. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider vultr
            api_key "YOUR_VULTR_API_KEY"
        }
    }
}
```

---

### Websupport

**Provider name:** `websupport`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `api_key` | `<string>` | WebSupport API key. | — (required) |
| `secret` | `<string>` | WebSupport secret. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider websupport
            api_key "YOUR_WEBSUPPORT_API_KEY"
            secret "YOUR_WEBSUPPORT_SECRET"
        }
    }
}
```

---

### Volcano Engine

**Provider name:** `volcengine`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `access_key` | `<string>` | Volcengine AccessKey. | — (required) |
| `secret_key` | `<string>` | Volcengine SecretKey. | — (required) |
| `region` | `<string>` | Volcengine region. | — (optional) |
| `host` | `<string>` | Custom API host. | — (optional) |
| `scheme` | `http`, `https` | HTTP scheme. | — (optional) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider volcengine
            access_key "YOUR_VOLCENGINE_ACCESS_KEY"
            secret_key "YOUR_VOLCENGINE_SECRET_KEY"
            region "cn-beijing"
        }
    }
}
```

---

### Yandex Cloud DNS

**Provider name:** `yandexcloud`

| Directive | Arguments | Description | Default |
|-----------|-----------|-------------|---------|
| `iam_token_b64` | `<string>` | IAM token (base64-encoded). | — (required) |
| `folder_id` | `<string>` | Yandex folder ID. | — (required) |

**Configuration example:**

```ferron
*.example.com {
    tls {
        provider acme
        challenge dns-01

        dns {
            provider yandexcloud
            iam_token_b64 "YOUR_IAM_TOKEN_B64"
            folder_id "YOUR_FOLDER_ID"
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
| `bluecatv2`, `exoscale`, `ns1`, `yandexcloud` | 0 s |
| `rfc2136`, `azuredns`, `gcore`, `huaweicloud`, `route53` | 1 s |
| `bunny` | 15 s |
| `constellix`, `dnsmadeeasy`, `dynu`, `digitalocean`, `infoblox`, `oraclecloud` | 30 s |
| `cloudflare`, `dnsimple`, `googlecloud`, `ovh`, `spaceship`, `arvancloud`, `cloudns`, `dreamhost`, `duckdns`, `freemyip`, `glesys`, `hetzner`, `hostingde`, `ibmcloud`, `infomaniak`, `ionos`, `ipv64`, `lightsail`, `luadns`, `mythicbeasts`, `namecheap`, `namesilo`, `netcup`, `netlify`, `nifcloud`, `scaleway`, `ultradns`, `vercel`, `vultr`, `volcengine` | 60 s (1 min) |
| `autodns`, `cpanel`, `domeneshop`, `easydns`, `gandiv5`, `hostinger`, `hurricane`, `inwx`, `joker`, `linode`, `plesk`, `safedns`, `websupport`, `baiducloud`, `transip` | 300 s (5 min) |
| `alidns`, `godaddy`, `namedotcom`, `tencentcloud`, `porkbun` | 600 s (10 min) |
| `ddnss` | 900 s (15 min) |
| `desec` | 3600 s (1 h) |

If certificate issuance fails with a DNS validation error, verify that the TXT record is resolvable from the public internet before retrying.

### RFC 2136 TSIG key format

The `key_secret` value must be the raw TSIG key bytes encoded as **standard Base64** (with padding). Most DNS management tools (BIND `tsig-keygen`, `dnssec-keygen`) output the key in this format already.

### Azure endpoint selection

Choose the `endpoint` that matches where your DNS zone is hosted:

| Value | Region |
|-------|--------|
| `AzurePublicCloud` | Azure (default) |
| `AzureChinaCloud` | Azure China |
| `AzureUSGovernment` | Azure Government |

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

- [ACME automatic TLS](/docs/v3/configuration/security/acme) — full ACME configuration reference
- [Automatic TLS use case](/docs/v3/use-cases/security/automatic-tls) — guided walkthrough

## Best practices

The following best-practice check is reported by `ferron doctor` for DNS provider directives.

- **Secrets in plain configuration** — DNS provider credentials (`api_key`, `secret`, `token`, etc.) should use environment variable interpolation (`{{env.VAR}}`) rather than plain strings to avoid leaking secrets in version control or logs.
