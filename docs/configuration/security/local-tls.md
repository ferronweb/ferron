---
title: "Configuration: local TLS provider"
description: "Locally-trusted certificates for development and testing environments using loopback addresses."
---

This page documents the `local` TLS provider, which generates and manages locally-trusted certificates for development and testing environments. Ferron selects this provider automatically for loopback addresses (`localhost`, `127.0.0.1`, `::1`) when you do not provide explicit TLS configuration.

## Directives

| Directive  | Type       | Default           | Description                  |
| ---------- | ---------- | ----------------- | ---------------------------- |
| `provider` | `local`    | —                 | Set to `"local"`           |
| `cache`    | `<string>` | platform data dir | Path for certificate caching |

**Configuration example:**

```ferron
localhost:443 {
    tls {
        provider local
        cache "/custom/cache/path"
    }
}
```

## Automatic selection

For loopback addresses, Ferron automatically selects the `local` provider when:

1. You have not configured an explicit `tls` directive
2. The host matches loopback criteria (`localhost`, `127.0.0.1`, `::1`)
3. Ferron has the `local` provider available

This means you can use HTTPS for development without any configuration:

```ferron
localhost {
    root /var/www/local-site
}

127.0.0.1 {
    root /var/www/local-site
}
```

## Explicit configuration

You can explicitly configure the local provider with a custom cache location:

```ferron
localhost:443 {
    tls {
        provider local
        cache "/custom/path/ferron-local-tls"
    }
}
```

## Certificate management

### Certificate Authority

Ferron generates a local root CA on first use. Ferron caches the CA certificate in the data directory. The certificate is valid for 10 years. You must manually trust the CA. Import it into your OS or browser trust store.

### Leaf certificates

Ferron generates leaf certificates for each unique set of Subject Alternative Names (SANs). The certificates are valid for 1 year. Ferron regenerates them automatically when they expire or when SANs change. When Ferron detects any loopback address, it includes all loopback addresses (`localhost`, `127.0.0.1`, `::1`) in the certificate.

### Cache location

By default, the server stores certificates in:

- **Linux/macOS**: `~/.local/share/ferron-local-tls/`
- **Windows**: `%LOCALAPPDATA%\ferron-local-tls\`

If Ferron cannot use the cache directory, it uses a temporary fallback location instead.

You can customize the cache location with the `cache` directive:

```ferron
example.com {
    tls {
        provider local
        cache "/path/to/custom/cache"
    }
}
```

## Security considerations

### Trust requirements

Your system or browser does **not automatically trust** the local CA. You must manually import the CA certificate:

1. Find the CA certificate path (logged at server startup)
2. Import into your OS trust store or browser

### Development use only

- **Not suitable for production.** Local certificates are not publicly trusted.
- **Development and testing only.** Use the ACME provider for public-facing sites.
- **Manual trust management.** You control which devices trust your local CA.

## Advanced configuration

The local provider supports the same TLS configuration options as other providers:

```ferron
localhost:443 {
    tls {
        provider local
        cache "/custom/cache/path"

        # Standard TLS configuration (optional)
        min_version TLSv1.3
        max_version TLSv1.3

        cipher_suite TLS_AES_128_GCM_SHA256
        cipher_suite TLS_AES_256_GCM_SHA384

        ecdh_curve x25519
    }
}
```

> [!info]
> For details on TLS crypto options, see [Security and TLS](/docs/v3/configuration/security/tls).

## Migration from manual certificates

If you were previously using manual certificates for localhost development, you can switch to the local provider:

```ferron
# Before: manual certificates
localhost:443 {
    tls /path/to/localhost.crt /path/to/localhost.key
}

# After: automatic local provider
localhost {
    # No explicit tls needed — automatically uses local provider
}
```

The local provider offers the same security with less manual certificate management.

> [!warning]
> Never use the local provider in production. Local certificates are not publicly trusted and will cause security warnings for all visitors. Use the ACME provider for public-facing websites.

### Certificate trust requirements

You must manually import the local CA certificate into your system or browser trust store. The server logs the CA certificate path at startup. Use this file to import the certificate.

### Cache directory permissions

If Ferron cannot write to the default cache directory, either:

- Create the directory manually and set appropriate permissions
- Configure a custom cache path with write permissions using the `cache` directive
- Run Ferron with a user that has write access to the default location

### Certificate regeneration

Ferron automatically regenerates leaf certificates when:

- They expire (after 1 year)
- The set of Subject Alternative Names (SANs) changes
- You delete the cached certificate files

Ferron regenerates the CA certificate only if the cached files are missing or corrupted.

### Browser certificate warnings

If you see security warnings in your browser:

1. **Check the certificate details.** Make sure the issuer is "Ferron Local Root CA"
2. **Import the CA certificate.** Add the CA to your OS or browser trust store
3. **Clear browser cache.** Some browsers cache certificate trust decisions
4. **Restart your browser.** Changes to certificate trust may require a restart

### Multiple loopback addresses

When Ferron detects any loopback address, the local provider includes all loopback addresses (`localhost`, `127.0.0.1`, `::1`) in the generated certificate for convenience.

## See also

- [Security and TLS](/docs/v3/configuration/security/tls): cipher suites, ECDH curves, mTLS
- [ACME automatic TLS](/docs/v3/configuration/security/acme): production TLS certificates
- [HTTP host directives](/docs/v3/configuration/server/host): per-host TLS configuration

## Best practices

`ferron doctor` reports the following best-practice check for directives on this page.

- **`provider local` on non-loopback hosts.** The local TLS provider issues self-signed certificates. Use ACME or manual certificates for production hostnames.
