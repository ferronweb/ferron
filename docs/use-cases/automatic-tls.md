---
title: Automatic TLS
description: "Set up automatic TLS in Ferron with Let's Encrypt and ACME challenges (TLS-ALPN-01, HTTP-01, DNS-01)."
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

## Using Ferron as an ACME client for other servers

If you run other servers (alongside Ferron) that support TLS, but not automatic TLS functionality, you can use Ferron 2.5.0 and newer as an ACME client to obtain TLS certificates for those servers, like this:

```kdl
// Replace "example.com" with your website's domain name
example.com {
    auto_tls_save_data "/tmp/server.crt" "/tmp/server.key" // Replace "/tmp/server.crt" and "/tmp/server.key" with actual paths to the certificate and private key files.

    // Optionally, you can also specify the command to run after saving the certificate and private key, for example to reload the server that uses the obtained TLS certificate.
    // These environment variables are supplied to the command:
    // - FERRON_ACME_DOMAIN - the domain name for which the certificate was obtained; comma-separated if multiple domain names
    // - FERRON_ACME_CERT_PATH - the path to the obtained TLS certificate
    // - FERRON_ACME_KEY_PATH - the path to the obtained private key

    //auto_tls_post_obtain_command "/etc/reload-server.sh"

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

For the reference of supported DNS providers and their configuration properties, see the [configuration reference](/docs/configuration/security-tls#dns-providers-for-acme-dns-01-challenge)
