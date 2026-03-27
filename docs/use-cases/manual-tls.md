---
title: Manual TLS
description: "Configure Ferron with existing TLS certificates and private keys, without automatic ACME certificate issuance."
---

Manual TLS is useful when you already have certificates and private keys, or when certificate issuance and renewal is handled outside Ferron.

For many use cases, automatic TLS with ACME is the easiest way to get HTTPS working. However, if you have specific requirements or existing certificates, you can configure Ferron to use them directly.

To enable manual TLS for a host, configure the `tls` directive with the certificate and private key paths:

```kdl
// Replace "manual-tls.example.com" with your domain name.
manual-tls.example.com {
    tls "/etc/ssl/certs/manual-tls.example.com.crt" "/etc/ssl/private/manual-tls.example.com.key"
    root "/var/www/html" // Replace "/var/www/html" with your website root directory
}
```

The certificate and private key must match. If they do not match, TLS handshakes will fail.

## Manual TLS with multiple hosts

You can configure manual TLS per virtual host. This is useful when each domain uses different certificates.

```kdl
example.com {
    tls "/etc/ssl/certs/example.com.crt" "/etc/ssl/private/example.com.key"
    root "/var/www/example"
}

api.example.com {
    tls "/etc/ssl/certs/api.example.com.crt" "/etc/ssl/private/api.example.com.key"
    proxy "http://localhost:3000/"
}
```

## Notes and troubleshooting

- Make sure Ferron can read both files from the `tls` directive path.
- Ensure the certificate file includes any required intermediate certificates when needed by your CA.
- If you rotate certificates externally, reload or restart Ferron so updated files are used.
- If you do not want automatic TLS on a host, set `auto_tls #false` explicitly.
- For all TLS-related directives (`tls`, `tls_min_version`, `tls_max_version`, `tls_client_certificate`, `ocsp_stapling`), see [Configuration: security & TLS](/docs/configuration/security-tls).
