---
title: Automatic TLS
---

Ferron supports automatic TLS via Let's Encrypt, and TLS-ALPN-01 and HTTP-01 (Ferron 1.1.0 and newer) ACME challenges. The domain names for the certificate will be extracted from the host configuration (wildcard domains are ignored, since TLS-ALPN-01 nor HTTP-01 ACME challenges doesn't support them).

The automatic TLS functionality is used to obtain TLS certificates automatically, without needing to manually import TLS certificates or use an external tool to obtain TLS certificates, like Certbot. This makes the process of obtaining TLS certificate more convenient and efficient.

Ferron supports both production and staging Let's Encrypt directories. The staging Let's Encrypt directory can be used for testing purposes and to verify that the server and automatic TLS is configured correctly.

Below is the example Ferron configuration that enables automatic TLS using production Let's Encrypt directory:

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
