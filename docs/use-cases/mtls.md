---
title: mTLS (mutual TLS)
description: "Require client TLS certificates in Ferron for internal/admin traffic and service-to-service access."
---

Mutual TLS (mTLS) adds client certificate verification on top of normal server TLS. This is useful for internal admin panels, partner integrations, and service-to-service traffic.

## Require client certificates

Configure client certificate validation against your internal CA:

```kdl
globals {
    // Verify client certificates using this CA file.
    tls_client_certificate "/etc/ssl/certs/internal-client-ca.pem"
}

// Replace "admin.example.com" with your domain name.
admin.example.com {
    tls "/etc/ssl/certs/admin.example.com.crt" "/etc/ssl/private/admin.example.com.key"
    proxy "http://127.0.0.1:9000/"
}
```

You can also use the system trust store:

```kdl
globals {
    tls_client_certificate #true
}
```

## Scope planning for admin/internal endpoints

`tls_client_certificate` is a global-only directive. If you need mTLS only for internal/admin traffic (but not public traffic), run a separate Ferron instance for internal endpoints.

## Notes and troubleshooting

- Ensure the client certificate chain is issued by the CA you configured in `tls_client_certificate`.
- Keep private internal CA material protected and rotate client certificates regularly.
- If requests fail during TLS handshake, verify certificate validity dates and CA chain.
- For directive details, see [Configuration: security & TLS](/docs/configuration/security-tls).
