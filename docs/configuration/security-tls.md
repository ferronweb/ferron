---
title: "Configuration: security & TLS"
description: "TLS settings, automatic certificate management, and access control directives for KDL configuration. Included supported DNS providers for DNS-01 challenge."
---

This page covers KDL directives for TLS configuration, certificate automation, and request-security controls in Ferron.

## Global-only directives

### TLS/SSL & security

- `tls_cipher_suite <tls_cipher_suite: string> [<tls_cipher_suite_2: string> ...]`
  - This directive specifies the supported TLS cipher suites. If using the HTTP/3 protocol (which is experimental in Ferron), the `TLS_AES_128_GCM_SHA256` cipher suite needs to be enabled (it's enabled by default), otherwise the HTTP/3 server wouldn’t start at all. This directive can be specified multiple times. Default: default TLS cipher suite for Rustls
- `tls_ecdh_curves <ecdh_curve: string> [<ecdh_curve: string> ...]`
  - This directive specifies the supported TLS ECDH curves. This directive can be specified multiple times. Default: default ECDH curves for Rustls
- `tls_client_certificate [tls_client_certificate: bool|string]`
  - This directive specifies whether the TLS client certificate verification is enabled. If set to `#true`, the client certificate will be verified against the system certificate store. If set to a string, the client certificate will be verified against the certificate authority in the specified path. Default: `tls_client_certificate #false`
- `tls_min_version <tls_min_version: string>`
  - This directive specifies the minimum TLS version (TLSv1.2 or TLSv1.3) that the server will accept. Default: `tls_min_version "TLSv1.2"`
- `tls_max_version <tls_max_version: string>`
  - This directive specifies the maximum TLS version (TLSv1.2 or TLSv1.3) that the server will accept. Default: `tls_max_version "TLSv1.3"`
- `ocsp_stapling [enable_ocsp_stapling: bool]`
  - This directive specifies whether OCSP stapling is enabled. Default: `ocsp_stapling #true`
- `auto_tls_on_demand_ask <auto_tls_on_demand_ask_url: string|null>`
  - This directive specifies the URL to be used for asking whether to the hostname for automatic TLS on demand is allowed. The server will append the `domain` query parameter with the domain name for the certificate to issue as a value to the URL. It's recommended to configure this option when using automatic TLS on demand to prevent abuse. Default: `auto_tls_on_demand_ask #null`
- `auto_tls_on_demand_ask_no_verification [auto_tls_on_demand_ask_no_verification: bool]`
  - This directive specifies whether the server should not verify the TLS certificate of the automatic TLS on demand asking endpoint. Default: `auto_tls_on_demand_ask_no_verification #false`

**Configuration example:**

```kdl
* {
    tls_cipher_suite "TLS_AES_256_GCM_SHA384" "TLS_AES_128_GCM_SHA256"
    tls_ecdh_curves "secp256r1" "secp384r1"
    tls_client_certificate #false
    ocsp_stapling
    auto_tls_on_demand_ask "https://auth.example.com/check"
    auto_tls_on_demand_ask_no_verification #false
}
```

## Global and virtual host directives

### TLS/SSL & security

- `tls <certificate_path: string> <private_key_path: string>`
  - This directive specifies the path to the TLS certificate and private key. Per-IP automatic TLS are supported in Ferron 2.7.0 and newer. Default: none
- `auto_tls [enable_automatic_tls: bool]`
  - This directive specifies whether automatic TLS is enabled. Per-IP automatic TLS are supported in Ferron 2.7.0 and newer. Default: `auto_tls #true` when port isn't explicitly specified and if the hostname doesn't look like a local address (`127.0.0.1`, `::1`, `localhost`), otherwise `auto_tls #false`
- `auto_tls_contact <auto_tls_contact: string|null>`
  - This directive specifies the email address used to register an ACME account for automatic TLS. Default: `auto_tls_contact #null`
- `auto_tls_cache <auto_tls_cache: string|null>`
  - This directive specifies the directory to store cached ACME data, such as cached account data and certificates. Default: OS-specific directory, for example on GNU/Linux it can be `/home/user/.local/share/ferron-acme` for the "user" user, on macOS it can be `/Users/user/Library/Application Support/ferron-acme` for the "user" user, on Windows it can be `C:\Users\user\AppData\Local\ferron-acme` for the "user" user. On Docker, it would be `/var/lib/ferron-acme`.
- `auto_tls_letsencrypt_production [enable_auto_tls_letsencrypt_production: bool]`
  - This directive specifies whether the production Let's Encrypt ACME endpoint is used. If set as `auto_tls_letsencrypt_production #false`, the staging Let's Encrypt ACME endpoint is used. Default: `auto_tls_letsencrypt_production #true`
- `auto_tls_challenge <acme_challenge_type: string> [provider=<acme_challenge_provider: string>] [...]`
  - This directive specifies the used ACME challenge type. The supported types are `"http-01"` (HTTP-01 ACME challenge), `"tls-alpn-01"` (TLS-ALPN-01 ACME challenge) and `"dns-01"` (DNS-01 ACME challenge). The `provider` prop defines the DNS provider to use for DNS-01 challenges. Additional props can be passed as parameters for the DNS provider, see automatic TLS documentation. Default: `auto_tls_challenge "tls-alpn-01"`
- `auto_tls_directory <auto_tls_directory: string>`
  - This directive specifies the ACME directory URL from which the certificates are obtained. Overrides `auto_tls_letsencrypt_production` directive. Default: none
- `auto_tls_no_verification [auto_tls_no_verification: bool]`
  - This directive specifies whether to disable the certificate verification of the ACME server. Default: `auto_tls_no_verification #false`
- `auto_tls_profile <auto_tls_profile: string|null>`
  - This directive specifies the ACME profile to use for the certificates. Default: `auto_tls_profile #null`
- `auto_tls_on_demand <auto_tls_on_demand: bool>`
  - This directive specifies whether to enable the automatic TLS on demand. The functionality obtains TLS certificates automatically when a website is accessed for the first time. It's recommended to use either HTTP-01 or TLS-ALPN-01 ACME challenges, as DNS-01 ACME challenges might be slower due to DNS propagation delays. It's also recommended to configure the `auto_tls_on_demand_ask` directive alongside this directive. Default: `auto_tls_on_demand #false`
- `auto_tls_eab (<auto_tls_eab_key_id: string> <auto_tls_eab_key_hmac: string>)|<auto_tls_eab_disabled: null>`
  - This directive specifies the EAB key ID and HMAC for the ACME External Account Binding. The HMAC key value is encoded in a URL-safe Base64 encoding. If set as `auto_tls_eab_disabled #null`, the EAB is disabled. Default: `auto_tls_eab_disabled #null`
- `auto_tls_save_data (<auto_tls_save_certificate_path: string> <auto_tls_save_private_key_path: string>)|<auto_tls_save_data_disabled: null>` (Ferron 2.5.0 or newer)
  - This directive specifies the path to save the obtained TLS certificate and private key when using automatic TLS. This can be useful for debugging purposes or for using the obtained TLS certificate and private key with other software. This directive isn't supported when using it alongside automatic TLS on demand. Default: `auto_tls_save_data #null`
- `auto_tls_post_obtain_command <auto_tls_post_obtain_command: string>|<auto_tls_post_obtain_command_disabled: null>` (Ferron 2.5.0 or newer)
  - This directive specifies the command (arguments are supported in Ferron 2.8.0 and newer) to be executed after obtaining a TLS certificate when using automatic TLS. The command will be executed with the following environment variables set: `FERRON_ACME_DOMAIN` (the domain name for which the certificate was obtained; comma-separated if multiple domain names), `FERRON_ACME_CERT_PATH` (the path to the obtained TLS certificate), `FERRON_ACME_KEY_PATH` (the path to the obtained private key). This can be useful for running custom scripts after obtaining a TLS certificate, for example for reloading other software that uses the obtained TLS certificate. This directive is effective only when `auto_tls_save_data` directive is effective. Default: `auto_tls_post_obtain_command #null`

**Configuration example:**

```kdl
example.com {
    auto_tls
    auto_tls_contact "admin@example.com"
    auto_tls_cache "/var/cache/ferron-acme"
    auto_tls_letsencrypt_production
    auto_tls_challenge "tls-alpn-01"
    auto_tls_profile "default"
    auto_tls_on_demand #false
    auto_tls_eab #null
}

manual-tls.example.com {
    tls "/etc/ssl/certs/example.com.crt" "/etc/ssl/private/example.com.key"
}
```

### Security & access control

- `trust_x_forwarded_for [trust_x_forwarded_for: bool]`
  - This directive specifies whether to trust the value of the `X-Forwarded-For` header. It's recommended to configure this directive if behind a reverse proxy. Default: `trust_x_forwarded_for #false`
- `status <status_code: integer> [url=<url: string>|regex=<regex: string>] [location=<location: string>] [realm=<realm: string>] [brute_protection=<enable_brute_protection: bool>] [users=<users: string>] [allowed=<allowed: string>] [not_allowed=<not_allowed: string>] [body=<response_body: string>]`
  - This directive specifies the custom status code. This directive can be specified multiple times. The `url` prop specifies the request path for this status code. The `regex` prop specifies the regular expression (like `^/ferron(?:$|[/#?])`) for the custom status code. The `location` prop specifies the destination for the redirect; it supports placeholders like `{path}` which will be replaced with the request path. The `realm` prop specifies the HTTP basic authentication realm. The `brute_protection` prop specifies whether the brute-force protection is enabled. The `users` prop is a comma-separated list of allowed users for HTTP authentication. The `allowed` prop is a comma-separated list of IP addresses applicable for the status code. The `not_allowed` prop is a comma-separated list of IP addresses not applicable for the status code. The `body` prop specifies the response body to be sent. Default: none
- `user <username: string> <password_hash: string>`
  - This directive specifies an user with a password hash used for the HTTP basic authentication (it can be either Argon2, PBKDF2, or `scrypt` one). It's recommended to use the `ferron-passwd` tool to generate the password hash. This directive can be specified multiple times. Default: none
- `block (<blocked_ip: string> [<blocked_ip: string> ...])|<not_specified: null>`
  - This directive specifies IP addresses and CIDR ranges to be blocked. If set as `block #null`, this directive is ignored. This directive was global-only before Ferron 2.1.0. This directive can be specified multiple times. Default: none
- `allow (<allowed_ip: string> [<allowed_ip: string> ...])|<not_specified: null>`
  - This directive specifies IP addresses and CIDR ranges to be allowed. If set as `allow #null`, this directive is ignored. This directive was global-only before Ferron 2.1.0. This directive can be specified multiple times. Default: none
- `abort [abort_request: bool]` (Ferron 2.6.0 or newer)
  - This directive specifies whether to immediately close the connection without sending any response. Default: `abort #false`

**Configuration example:**

```kdl
example.com {
    trust_x_forwarded_for

    // Basic authentication with custom status codes
    status 401 url="/admin" realm="Admin Area" users="admin,moderator"
    status 403 url="/restricted" allowed="192.168.1.0/24" body="Access denied"
    status 301 url="/old-page" location="/new-page"

    // User definitions for authentication (use `ferron-passwd` to generate password hashes)
    user "admin" "$2b$10$hashedpassword12345"
    user "moderator" "$2b$10$anotherhashedpassword"

    // Limit who can access the site
    block "192.168.1.100" "10.0.0.5"
    allow "192.168.1.0/24" "10.0.0.0/8"
}
```

## DNS providers for ACME DNS-01 challenge

When using `auto_tls_challenge "dns-01"` directive, you can specify the DNS provider to be used for the ACME DNS-01 challenge with the `provider` prop. Below is the list of supported DNS providers and their additional configuration props.

In addition to the provider-specific props, Ferron 2.9.0 and newer supports the `propagation_wait` prop for all DNS providers, which specifies the time (in seconds, as a string, for example `propagation_wait="900"`) to wait for DNS propagation after setting the ACME challenge DNS record; it defaults to 60 seconds. Increase this value if your DNS provider synchronizes its nameservers slowly and certificate issuance fails with the default wait time.

### Akamai Edge DNS (`edgedns`)

This DNS provider uses [Akamai Edge DNS API](https://techdocs.akamai.com/edge-dns/reference/edge-dns-api) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="edgedns" host="your_akamai_host" client_token="your_client_token" client_secret="your_client_secret" access_token="your_access_token"
```

#### Additional props

- `host` - Akamai Edge DNS API host (required)
- `client_token` - Akamai client token (required)
- `client_secret` - Akamai client secret (required)
- `access_token` - Akamai access token (required)
- `account_switch_key` - Akamai account switch key (optional)

### Alibaba Cloud DNS (`alidns`)

This DNS provider uses [Alibaba Cloud DNS API](https://www.alibabacloud.com/help/en/dns) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="alidns" access_key_id="your_access_key_id" access_key_secret="your_access_key_secret"
```

#### Additional props

- `access_key_id` - Alibaba Cloud DNS access key ID (required)
- `access_key_secret` - Alibaba Cloud DNS access key secret (required)
- `region` - Alibaba Cloud DNS region (optional)
- `security_token` - Alibaba Cloud DNS security token (optional)
- `line` - Alibaba Cloud DNS resolution line (optional)

### Amazon Lightsail (`lightsail`)

This DNS provider uses [Amazon Lightsail API](https://docs.aws.amazon.com/lightsail/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="lightsail" access_key_id="your_key_id" secret_access_key="your_secret_access_key"
```

#### Additional props

- `access_key_id` - AWS access key ID (required)
- `secret_access_key` - AWS secret access key (required)
- `session_token` - AWS session token (optional)
- `region` - AWS region (optional)
- `domain` - Amazon Lightsail domain name (optional)

### Amazon Route 53 (`route53`)

This DNS provider uses [Amazon Route 53 API](https://docs.aws.amazon.com/Route53/latest/APIReference/Welcome.html) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.0.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="route53" access_key_id="your_key_id" secret_access_key="your_secret_access_key" region="aws-region" hosted_zone_id="your_hosted_zone_id"
```

#### Additional props

- `access_key_id` - AWS access key ID (optional)
- `secret_access_key` - AWS secret access key (optional)
- `region` - AWS region (optional)
- `profile_name` - AWS profile name (optional)
- `hosted_zone_id` - Amazon Route 53 hosted zone ID (optional)

### ArvanCloud (`arvancloud`)

This DNS provider uses [ArvanCloud API](https://www.arvancloud.ir/en/dev/api) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="arvancloud" api_key="your_api_key"
```

#### Additional props

- `api_key` - ArvanCloud API key (required)

### AutoDNS (`autodns`)

This DNS provider uses [AutoDNS API](https://help.internetx.com/display/APIXMLEN/Welcome) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="autodns" username="your_username" password="your_password"
```

#### Additional props

- `username` - AutoDNS username (required)
- `password` - AutoDNS password (required)
- `context` - AutoDNS context number (optional)

### Azure DNS (`azuredns`)

This DNS provider uses [Azure DNS API](https://learn.microsoft.com/en-us/rest/api/dns/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="azuredns" tenant_id="your_tenant_id" client_id="your_client_id" client_secret="your_client_secret" subscription_id="your_subscription_id" resource_group="your_resource_group" endpoint="AzurePublicCloud"
```

#### Additional props

- `tenant_id` - Microsoft Entra tenant ID (required)
- `client_id` - Application (client) ID (required)
- `client_secret` - Application client secret (required)
- `subscription_id` - Azure subscription ID (required)
- `resource_group` - Azure resource group name (required)
- `endpoint` - Azure environment; either "AzurePublicCloud", "AzureChinaCloud", or "AzureUSGovernment" (required)

### Baidu Cloud DNS (`baiducloud`)

This DNS provider uses [Baidu Cloud DNS API](https://cloud.baidu.com/doc/DNS/index.html) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="baiducloud" access_key_id="your_access_key_id" access_key_secret="your_access_key_secret"
```

#### Additional props

- `access_key_id` - Baidu Cloud DNS access key ID (required)
- `access_key_secret` - Baidu Cloud DNS access key secret (required)

### BlueCat (`bluecatv2`)

This DNS provider uses [BlueCat Address Manager REST v2 API](https://docs.bluecatnetworks.com/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="bluecatv2" server_url="https://bluecat.example.com" username="your_username" password="your_password" config_name="your_config" view_name="your_view"
```

#### Additional props

- `server_url` - BlueCat Address Manager server URL (required)
- `username` - BlueCat username (required)
- `password` - BlueCat password (required)
- `config_name` - BlueCat configuration name (required)
- `view_name` - BlueCat DNS view name (required)
- `skip_deploy` - Either "true" or "false"; if "true", skips deploying the DNS changes; defaults to "false" (optional)

### bunny.net (`bunny`)

This DNS provider uses [bunny.net API](https://docs.bunny.net/reference) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.4.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="bunny" api_key="your_api_key"
```

#### Additional props

- `api_key` - bunny.net API key (required)

### Cloudflare (`cloudflare`)

This DNS provider uses [Cloudflare API](https://developers.cloudflare.com/api/resources/dns/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.0.0. To get `your_api_key` add a new token via [Cloudflare Dashboard](https://dash.cloudflare.com/profile/api-tokens), using the "Edit zone DNS" template with "**Permissions**" of "Zone"→"DNS"→"Edit" and "**Zone Resources**" "Include"→"Specific zone"→"your custom domain".

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="cloudflare" api_key="your_api_key"
```

#### Additional props

- `api_key` - Cloudflare API token (required)
- `email` - Cloudflare account email address (deprecated and ignored since Ferron 2.9.0; Cloudflare global API keys are no longer supported, use an API token instead)

### ClouDNS (`cloudns`)

This DNS provider uses [ClouDNS API](https://www.cloudns.net/wiki/article/41/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="cloudns" auth_id="your_auth_id" password="your_password"
```

#### Additional props

- `auth_id` - ClouDNS API user ID (optional)
- `sub_auth_id` - ClouDNS API sub-user ID (optional)
- `password` - ClouDNS API password (required)

### Constellix (`constellix`)

This DNS provider uses [Constellix API](https://api-docs.constellix.com/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="constellix" api_key="your_api_key" secret_key="your_secret_key"
```

#### Additional props

- `api_key` - Constellix API key (required)
- `secret_key` - Constellix secret key (required)

### cPanel (`cpanel`)

This DNS provider uses [cPanel UAPI](https://api.docs.cpanel.net/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="cpanel" base_url="https://cpanel.example.com:2083" username="your_username" token="your_api_token"
```

#### Additional props

- `base_url` - cPanel base URL (required)
- `username` - cPanel username (required)
- `token` - cPanel API token (required)

### DDNSS.de (`ddnss`)

This DNS provider uses [DDNSS.de update API](https://ddnss.de/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="ddnss" key="your_update_key"
```

#### Additional props

- `key` - DDNSS.de update key (required)

### deSEC (`desec`)

This DNS provider uses [deSEC API](https://desec.readthedocs.io/en/latest/index.html) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.0.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="desec" api_token="your_api_token"
```

#### Additional props

- `api_token` - deSEC API token (required)

### DigitalOcean (`digitalocean`)

This DNS provider uses [DigitalOcean API](https://docs.digitalocean.com/reference/api/digitalocean/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.4.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="digitalocean" oauth_token="your_token"
```

#### Additional props

- `oauth_token` - DigitalOcean OAuth token (required)

### DNS Made Easy (`dnsmadeeasy`)

This DNS provider uses [DNS Made Easy API](https://dnsmadeeasy.com/integrations/restapi) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="dnsmadeeasy" api_key="your_api_key" api_secret="your_api_secret"
```

#### Additional props

- `api_key` - DNS Made Easy API key (required)
- `api_secret` - DNS Made Easy API secret (required)

### DNSimple (`dnsimple`)

This DNS provider uses [DNSimple API](https://developer.dnsimple.com/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.7.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="dnsimple" oauth_token="your_oauth_token" account_id="your_account_id"
```

#### Additional props

- `oauth_token` - DNSimple OAuth token (required)
- `account_id` - DNSimple account ID (required)

### Domeneshop (`domeneshop`)

This DNS provider uses [Domeneshop API](https://api.domeneshop.no/docs/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="domeneshop" api_token="your_api_token" api_secret="your_api_secret"
```

#### Additional props

- `api_token` - Domeneshop API token (required)
- `api_secret` - Domeneshop API secret (required)

### DreamHost (`dreamhost`)

This DNS provider uses [DreamHost API](https://help.dreamhost.com/hc/en-us/articles/217560167-API-overview) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="dreamhost" api_key="your_api_key"
```

#### Additional props

- `api_key` - DreamHost API key (required)

### Duck DNS (`duckdns`)

This DNS provider uses [Duck DNS API](https://www.duckdns.org/spec.jsp) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="duckdns" token="your_token"
```

#### Additional props

- `token` - Duck DNS token (required)

### Dynu (`dynu`)

This DNS provider uses [Dynu API](https://www.dynu.com/en-US/Support/API) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="dynu" api_key="your_api_key"
```

#### Additional props

- `api_key` - Dynu API key (required)

### easyDNS (`easydns`)

This DNS provider uses [easyDNS API](https://docs.sandbox.rest.easydns.net/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="easydns" token="your_api_token" key="your_api_key"
```

#### Additional props

- `token` - easyDNS API token (required)
- `key` - easyDNS API key (required)

### Exoscale (`exoscale`)

This DNS provider uses [Exoscale DNS API](https://community.exoscale.com/api/dns/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="exoscale" api_key="your_api_key" api_secret="your_api_secret"
```

#### Additional props

- `api_key` - Exoscale API key (required)
- `api_secret` - Exoscale API secret (required)

### freemyip.com (`freemyip`)

This DNS provider uses [freemyip.com API](https://freemyip.com/help) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="freemyip" token="your_token"
```

#### Additional props

- `token` - freemyip.com token (required)

### Gandi (`gandiv5`)

This DNS provider uses [Gandi LiveDNS API](https://api.gandi.net/docs/livedns/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="gandiv5" personal_access_token="your_personal_access_token"
```

#### Additional props

- `personal_access_token` - Gandi personal access token (required)

### Gcore (`gcore`)

This DNS provider uses [Gcore DNS API](https://api.gcore.com/docs/dns) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="gcore" api_token="your_api_token"
```

#### Additional props

- `api_token` - Gcore permanent API token (required)

### GleSYS (`glesys`)

This DNS provider uses [GleSYS API](https://github.com/GleSYS/API/wiki/API-Documentation) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="glesys" api_user="your_api_user" api_key="your_api_key"
```

#### Additional props

- `api_user` - GleSYS API user (required)
- `api_key` - GleSYS API key (required)

### GoDaddy (`godaddy`)

This DNS provider uses [GoDaddy API](https://developer.godaddy.com/doc/endpoint/domains) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="godaddy" api_key="your_api_key" api_secret="your_api_secret"
```

#### Additional props

- `api_key` - GoDaddy API key (required)
- `api_secret` - GoDaddy API secret (required)

### Google Cloud DNS (`googlecloud`)

This DNS provider uses [Google Cloud DNS API](https://cloud.google.com/dns/docs/reference/v1) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.8.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="googlecloud" service_account_json="your_service_account_json" project_id="your_project_id"
```

#### Additional props

- `service_account_json` - contents of the Google Cloud service account JSON key file (required)
- `project_id` - Google Cloud project ID (required)
- `managed_zone` - Google Cloud DNS managed zone name (optional)
- `private_zone` - whether to target a private zone (`"true"` would enable it, `"false"` would disable it; optional)
- `impersonate_service_account` - the service account email to impersonate (optional)

### Hetzner (`hetzner`)

This DNS provider uses [Hetzner DNS API](https://dns.hetzner.com/api-docs) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="hetzner" api_token="your_api_token"
```

#### Additional props

- `api_token` - Hetzner DNS API token (required)

### hosting.de (`hostingde`)

This DNS provider uses [hosting.de API](https://www.hosting.de/api/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="hostingde" api_key="your_api_key"
```

#### Additional props

- `api_key` - hosting.de API key (required)

### Hostinger (`hostinger`)

This DNS provider uses [Hostinger API](https://developers.hostinger.com/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="hostinger" api_token="your_api_token"
```

#### Additional props

- `api_token` - Hostinger API token (required)

### Huawei Cloud DNS (`huaweicloud`)

This DNS provider uses [Huawei Cloud DNS API](https://support.huaweicloud.com/intl/en-us/api-dns/dns_api_62001.html) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="huaweicloud" access_key_id="your_access_key_id" access_key_secret="your_access_key_secret" region="your_region"
```

#### Additional props

- `access_key_id` - Huawei Cloud access key ID (required)
- `access_key_secret` - Huawei Cloud access key secret (required)
- `region` - Huawei Cloud region (required)

### Hurricane Electric (`hurricane`)

This DNS provider uses [Hurricane Electric dynamic DNS API](https://dns.he.net/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0. Create the `_acme-challenge` TXT records in the Hurricane Electric control panel beforehand, enable dynamic DNS for them, and specify their dynamic DNS keys in the `credentials` prop.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="hurricane" credentials="_acme-challenge.example.com=your_ddns_key,_acme-challenge.www.example.com=your_other_ddns_key"
```

#### Additional props

- `credentials` - comma-separated list of `record_name=dynamic_dns_key` pairs (required)

### IBM Cloud DNS (`ibmcloud`)

This DNS provider uses [IBM Cloud (SoftLayer) API](https://sldn.softlayer.com/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="ibmcloud" username="your_username" api_key="your_api_key"
```

#### Additional props

- `username` - IBM Cloud (SoftLayer) username (required)
- `api_key` - IBM Cloud (SoftLayer) API key (required)

### Infoblox (`infoblox`)

This DNS provider uses [Infoblox NIOS WAPI](https://www.infoblox.com/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="infoblox" host="infoblox.example.com" username="your_username" password="your_password"
```

#### Additional props

- `host` - Infoblox host (required)
- `port` - Infoblox port (optional)
- `username` - Infoblox username (required)
- `password` - Infoblox password (required)
- `wapi_version` - Infoblox WAPI version (optional)
- `dns_view` - Infoblox DNS view (optional)

### Infomaniak (`infomaniak`)

This DNS provider uses [Infomaniak API](https://developer.infomaniak.com/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="infomaniak" api_token="your_api_token"
```

#### Additional props

- `api_token` - Infomaniak API token (required)

### INWX (`inwx`)

This DNS provider uses [INWX API](https://www.inwx.com/en/help/apidoc) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="inwx" username="your_username" password="your_password"
```

#### Additional props

- `username` - INWX username (required)
- `password` - INWX password (required)
- `sandbox` - Either "true" or "false"; if "true", uses the INWX sandbox environment; defaults to "false" (optional)

### IONOS (`ionos`)

This DNS provider uses [IONOS DNS API](https://developer.hosting.ionos.com/docs/dns) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="ionos" api_key="your_api_key"
```

#### Additional props

- `api_key` - IONOS API key (required)

### IPv64.net (`ipv64`)

This DNS provider uses [IPv64.net API](https://ipv64.net/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="ipv64" api_key="your_api_key"
```

#### Additional props

- `api_key` - IPv64.net API key (required)

### Joker.com (`joker`)

This DNS provider uses [Joker.com DMAPI](https://joker.com/faq/category/39/22-dmapi.html) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="joker" api_key="your_api_key"
```

#### Additional props

- `api_key` - Joker.com API key; used if the username and password are not specified (optional)
- `username` - Joker.com username; used together with the password; takes precedence over the API key (optional)
- `password` - Joker.com password; used together with the username; takes precedence over the API key (optional)

### Linode (`linode`)

This DNS provider uses [Linode API](https://techdocs.akamai.com/linode-api/reference/api) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="linode" api_token="your_api_token"
```

#### Additional props

- `api_token` - Linode API token (required)

### LuaDNS (`luadns`)

This DNS provider uses [LuaDNS API](https://www.luadns.com/api.html) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="luadns" api_username="your_email@example.com" api_token="your_api_token"
```

#### Additional props

- `api_username` - LuaDNS account email address (required)
- `api_token` - LuaDNS API token (required)

### Mythic Beasts (`mythicbeasts`)

This DNS provider uses [Mythic Beasts DNS API](https://www.mythic-beasts.com/support/api/dnsv2) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="mythicbeasts" username="your_api_key_id" password="your_api_secret"
```

#### Additional props

- `username` - Mythic Beasts API key ID (required)
- `password` - Mythic Beasts API secret (required)

### Name.com (`namedotcom`)

This DNS provider uses [Name.com API](https://www.name.com/api-docs) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="namedotcom" username="your_username" api_token="your_api_token"
```

#### Additional props

- `username` - Name.com username (required)
- `api_token` - Name.com API token (required)

### Namecheap (`namecheap`)

This DNS provider uses [Namecheap API](https://www.namecheap.com/support/api/intro/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="namecheap" api_key="your_api_user" api_secret="your_api_key" client_ip="203.0.113.2"
```

#### Additional props

- `api_key` - Namecheap API user (required)
- `api_secret` - Namecheap API key (required)
- `client_ip` - Client IP address allowed to use the Namecheap API (required)
- `username` - Namecheap username; defaults to the API user (optional)

### NameSilo (`namesilo`)

This DNS provider uses [NameSilo API](https://www.namesilo.com/api-reference) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="namesilo" api_token="your_api_key"
```

#### Additional props

- `api_token` - NameSilo API key (required)

### netcup (`netcup`)

This DNS provider uses [netcup DNS API](https://www.netcup.com/en/helpcenter/documentation/domain/our-api) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0. netcup synchronizes its nameservers slowly, so it's recommended to also specify a longer DNS propagation wait time, for example `propagation_wait="900"`.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="netcup" customer_number="12345" api_key="your_api_key" api_password="your_api_password" propagation_wait="900"
```

#### Additional props

- `customer_number` - netcup customer number (required)
- `api_key` - netcup API key (required)
- `api_password` - netcup API password (required)

### Netlify (`netlify`)

This DNS provider uses [Netlify API](https://docs.netlify.com/api/get-started/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="netlify" access_token="your_access_token"
```

#### Additional props

- `access_token` - Netlify personal access token (required)

### NIFCLOUD (`nifcloud`)

This DNS provider uses [NIFCLOUD DNS API](https://pfs.nifcloud.com/api/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="nifcloud" api_key="your_api_key" api_secret="your_api_secret"
```

#### Additional props

- `api_key` - NIFCLOUD API key (required)
- `api_secret` - NIFCLOUD API secret (required)

### NS1 (`ns1`)

This DNS provider uses [NS1 API](https://ns1.com/api) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="ns1" api_key="your_api_key"
```

#### Additional props

- `api_key` - NS1 API key (required)

### Oracle Cloud (`oraclecloud`)

This DNS provider uses [Oracle Cloud Infrastructure DNS API](https://docs.oracle.com/en-us/iaas/api/#/en/dns/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="oraclecloud" tenancy_ocid="your_tenancy_ocid" user_ocid="your_user_ocid" fingerprint="your_fingerprint" private_key_pem="your_private_key_pem" region="your_region" compartment_ocid="your_compartment_ocid"
```

#### Additional props

- `tenancy_ocid` - Oracle Cloud tenancy OCID (required)
- `user_ocid` - Oracle Cloud user OCID (required)
- `fingerprint` - Oracle Cloud API signing key fingerprint (required)
- `private_key_pem` - Oracle Cloud API signing key in the PEM format (required)
- `private_key_password` - Oracle Cloud API signing key password (optional)
- `region` - Oracle Cloud region (required)
- `compartment_ocid` - Oracle Cloud compartment OCID (required)

### OVH (`ovh`)

This DNS provider uses [OVH API](https://api.ovh.com/console/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.4.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="ovh" application_key="your_application_key" application_secret="your_application_secret" consumer_key="your_consumer_key" endpoint="ovh-eu"
```

#### Additional props

- `application_key` - OVH application key (required)
- `application_secret` - OVH application secret (required)
- `consumer_key` - OVH consumer key (required)
- `endpoint` - OVH endpoint. Supported values are `ovh-eu`, `ovh-ca`, `kimsufi-eu`, `kimsufi-ca`, `soyoustart-eu` and `soyoustart-ca` (required)

### Plesk (`plesk`)

This DNS provider uses [Plesk REST API](https://docs.plesk.com/en-US/obsidian/api-rpc/about-rest-api.79359/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="plesk" base_url="https://plesk.example.com:8443" api_key="your_api_key"
```

#### Additional props

- `base_url` - Plesk base URL (required)
- `api_key` - Plesk API key (required)

### Porkbun (`porkbun`)

This DNS provider uses [Porkbun API](https://porkbun.com/api/json/v3/documentation) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.0.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="porkbun" api_key="your_api_key" secret_key="your_secret_key"
```

#### Additional props

- `api_key` - Porkbun API key (required)
- `secret_key` - Porkbun secret API key (required)

### RFC 2136 (`rfc2136`)

This DNS provider uses [RFC 2136 protocol](https://tools.ietf.org/html/rfc2136) to authenticate and authorize ACME-related DNS records. This provider can be used with servers that support RFC 2136, like Bind9. This provider was added in Ferron 2.0.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="rfc2136" server="udp://127.0.0.1:53" key_name="dnskey" key_secret="your_key_secret" key_algorithm="hmac-sha256"
```

#### Additional props

- `server` - DNS server address URL, with either "tcp" or "udp" scheme (required)
- `key_name` - DNS server key name (required)
- `key_secret` - DNS server key secret, encoded in Base64 (required)
- `key_algorithm` - DNS server key algorithm. Supported values are `hmac-md5`, `gss`, `hmac-sha1`, `hmac-sha224`, `hmac-sha256`, `hmac-sha256-128`, `hmac-sha384`, `hmac-sha384-192`, `hmac-sha512` and `hmac-sha512-256` (required)

### SafeDNS (`safedns`)

This DNS provider uses [ANS SafeDNS API](https://developers.ans.co.uk/safedns) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="safedns" auth_token="your_api_token"
```

#### Additional props

- `auth_token` - SafeDNS API token (required)

### Scaleway (`scaleway`)

This DNS provider uses [Scaleway Domains and DNS API](https://www.scaleway.com/en/developers/api/domains-and-dns/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="scaleway" api_token="your_secret_key"
```

#### Additional props

- `api_token` - Scaleway secret key (required)

### Simply.com (`simplycom`)

This DNS provider uses [Simply.com API](https://www.simply.com/en/docs/api/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="simplycom" account_name="your_account_name" api_key="your_api_key"
```

#### Additional props

- `account_name` - Simply.com account name (required)
- `api_key` - Simply.com API key (required)

### Spaceship (`spaceship`)

This DNS provider uses [Spaceship API](https://docs.spaceship.dev/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.8.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="spaceship" api_key="your_api_key" api_secret="your_api_secret"
```

#### Additional props

- `api_key` - Spaceship API key (required)
- `api_secret` - Spaceship API secret (required)

### Tencent Cloud DNS (`tencentcloud`)

This DNS provider uses [Tencent Cloud DNSPod API](https://www.tencentcloud.com/document/product/1157) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="tencentcloud" secret_id="your_secret_id" secret_key="your_secret_key"
```

#### Additional props

- `secret_id` - Tencent Cloud secret ID (required)
- `secret_key` - Tencent Cloud secret key (required)
- `region` - Tencent Cloud region (optional)
- `session_token` - Tencent Cloud session token (optional)

### TransIP (`transip`)

This DNS provider uses [TransIP API](https://api.transip.nl/rest/docs.html) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="transip" login="your_login" private_key_pem="your_private_key_pem"
```

#### Additional props

- `login` - TransIP login name (required)
- `private_key_pem` - TransIP private key in the PEM format (required)
- `global_key` - Either "true" or "false"; if "true", the private key is not restricted to whitelisted IP addresses; defaults to "false" (optional)

### UltraDNS (`ultradns`)

This DNS provider uses [UltraDNS API](https://docs.ultradns.com/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="ultradns" username="your_username" password="your_password"
```

#### Additional props

- `username` - UltraDNS username (required)
- `password` - UltraDNS password (required)
- `endpoint` - UltraDNS API endpoint URL (optional)

### Vercel (`vercel`)

This DNS provider uses [Vercel API](https://vercel.com/docs/rest-api) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="vercel" auth_token="your_access_token"
```

#### Additional props

- `auth_token` - Vercel access token (required)
- `team_id` - Vercel team ID (optional)

### Volcengine (`volcengine`)

This DNS provider uses [Volcengine DNS API](https://www.volcengine.com/docs/6758) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="volcengine" access_key="your_access_key" secret_key="your_secret_key"
```

#### Additional props

- `access_key` - Volcengine access key (required)
- `secret_key` - Volcengine secret key (required)
- `region` - Volcengine region (optional)
- `host` - Volcengine API host (optional)
- `scheme` - Volcengine API URL scheme (optional)

### Vultr (`vultr`)

This DNS provider uses [Vultr API](https://www.vultr.com/api/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="vultr" api_key="your_api_key"
```

#### Additional props

- `api_key` - Vultr API key (required)

### Websupport (`websupport`)

This DNS provider uses [Websupport API](https://rest.websupport.sk/v2/docs) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="websupport" api_key="your_api_key" secret="your_api_secret"
```

#### Additional props

- `api_key` - Websupport API key (required)
- `secret` - Websupport API secret (required)

### Yandex Cloud DNS (`yandexcloud`)

This DNS provider uses [Yandex Cloud DNS API](https://yandex.cloud/en/docs/dns/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.9.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="yandexcloud" iam_token_b64="your_base64_encoded_iam_token" folder_id="your_folder_id"
```

#### Additional props

- `iam_token_b64` - Base64-encoded Yandex Cloud IAM token (required)
- `folder_id` - Yandex Cloud folder ID (required)

## Additional DNS providers

If you would like to use Ferron with additional DNS providers, you can check the [compilation notes](https://github.com/ferronweb/ferron/blob/2.x/COMPILATION.md).
