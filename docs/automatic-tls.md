---
title: Automatic TLS
description: "Set up automatic TLS in Ferron with Let's Encrypt and ACME challenges (TLS-ALPN-01, HTTP-01, DNS-01), including DNS provider options."
---

Ferron supports automatic TLS via Let's Encrypt, and TLS-ALPN-01, HTTP-01 (Ferron 1.1.0 and newer) and DNS-01 (Ferron 2.0.0 and newer) ACME challenges. The domain names for the certificate will be extracted from the host configuration (wildcard domains are ignored for TLS-ALPN-01 and HTTP-01 ACME challenges).

The automatic TLS functionality is used to obtain TLS certificates automatically, without needing to manually import TLS certificates or use an external tool to obtain TLS certificates, like Certbot. This makes the process of obtaining TLS certificate more convenient and efficient.

Ferron supports both production and staging Let's Encrypt directories. The staging Let's Encrypt directory can be used for testing purposes and to verify that the server and automatic TLS is configured correctly.

Also, Ferron 2.0.0 and newer support a default OS-specific ACME cache directory in a home directory of a user that Ferron runs as (if the home directory is available), making automatic TLS require less setup.

Below is the example Ferron configuration that configures automatic TLS with production Let's Encrypt directory:

```kdl
* {
    auto_tls
    auto_tls_contact "someone@example.com" // Replace "someone@example.com" with actual email address
    auto_tls_cache "/path/to/letsencrypt-cache" // Replace "/path/to/letsencrypt-cache" with actual cache directory. Optional property, but recommended
    auto_tls_letsencrypt_production
}

// Replace "example.com" with your website's domain name
example.com {
    root "/var/www/html"
}
```

## Note about Cloudflare proxies (and other HTTPS proxies)

Ferron uses TLS-ALPN-01 ACME challenge for automatic TLS by default, however this wouldn't work if your website is behind a proxy that terminates TLS, as TLS-ALPN-01 challenge works on TLS handshake level.

You can use HTTP-01 challenge instead, which works on HTTP level. You can add a `auto_tls_challenge "http-01"` global configuration directive, for example like this:

```kdl
* {
    auto_tls
    auto_tls_contact "someone@example.com" // Replace "someone@example.com" with actual email address
    auto_tls_cache "/path/to/letsencrypt-cache" // Replace "/path/to/letsencrypt-cache" with actual cache directory. Optional property, but recommended
    auto_tls_letsencrypt_production

    // Use HTTP-01 challenge instead of TLS-ALPN-01, because the server is behind an HTTPS proxy.
    auto_tls_challenge "http-01"
}

// Replace "example.com" with your website's domain name
example.com {
    root "/var/www/html"
}
```

## DNS providers

Ferron 2.0.0 and newer supports DNS-01 ACME challenge for automatic TLS. The DNS-01 ACME challenge requires a DNS provider to be configured in the `provider` prop in the `auto_tls_challenge` directive.

Below is the example Ferron configuration that configures automatic TLS with production Let's Encrypt directory and hypothetical `example` DNS provider:

```kdl
* {
    auto_tls
    auto_tls_contact "someone@example.com" // Replace "someone@example.com" with actual email address
    auto_tls_cache "/path/to/letsencrypt-cache" // Replace "/path/to/letsencrypt-cache" with actual cache directory. Optional property, but recommended
    auto_tls_letsencrypt_production
    auto_tls_challenge "dns-01" provider="example" some_prop="value" // The "some_prop" prop is used to configure the DNS provider
}

// Replace "example.com" with your website's domain name
example.com {
    root "/var/www/html"
}
```

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

### bunny.net (`bunny`)

This DNS provider uses [bunny.net API](https://docs.bunny.net/reference) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.4.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="bunny" api_key="your_api_key"
```

#### Additional props

- `api_key` - bunny.net API key (required)

### Cloudflare (`cloudflare`)

This DNS provider uses [Cloudflare API](https://developers.cloudflare.com/api/resources/dns/) to authenticate and authorize ACME-related DNS records. This provider was added in Ferron 2.0.0.

#### Example directive specification

```kdl
auto_tls_challenge "dns-01" provider="cloudflare" api_key="your_api_key" email="your_email@example.com"
```

#### Additional props

- `api_key` - Cloudflare API key (required)
- `email` - Cloudflare account email address (optional)

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

- `token` - DigitalOcean OAuth token (required)

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

## Additional DNS providers

If you would like to use Ferron with additional DNS providers, you can check the [compilation notes](https://github.com/ferronweb/ferron/blob/2.x/COMPILATION.md).
