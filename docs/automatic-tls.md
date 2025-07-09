---
title: Automatic TLS
---

Ferron supports automatic TLS via Let's Encrypt, and TLS-ALPN-01, HTTP-01 (Ferron 1.1.0 and newer) and DNS-01 (Ferron UNRELEASED and newer) ACME challenges. The domain names for the certificate will be extracted from the host configuration (wildcard domains are ignored for TLS-ALPN-01 and HTTP-01 ACME challenges).

The automatic TLS functionality is used to obtain TLS certificates automatically, without needing to manually import TLS certificates or use an external tool to obtain TLS certificates, like Certbot. This makes the process of obtaining TLS certificate more convenient and efficient.

Ferron supports both production and staging Let's Encrypt directories. The staging Let's Encrypt directory can be used for testing purposes and to verify that the server and automatic TLS is configured correctly.

Also, Ferron 2.0.0-beta.1 and newer support a default OS-specific ACME cache directory in a home directory of a user that Ferron runs as (if the home directory is available), making automatic TLS require less setup.

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

## DNS providers

Ferron supports DNS-01 ACME challenge for automatic TLS. The DNS-01 ACME challenge requires a DNS provider to be configured in the `provider` prop in the `auto_tls_challenge` directive.

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

### Porkbun (`porkbun`)

This DNS provider uses [Porkbun API](https://porkbun.com/api/json/v3/documentation) to authenticate and authorize ACME-related DNS records.

#### Additional props

- `api_key` - Porkbun API key
- `secret_key` - Porkbun secret API key
