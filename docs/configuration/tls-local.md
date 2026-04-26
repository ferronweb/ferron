---
title: "Configuration: local TLS provider"
description: "Locally-trusted certificates for development and testing environments using loopback addresses."
---

This page documents the `local` TLS provider, which generates and manages locally-trusted certificates for development and testing environments. It's automatically selected for loopback addresses (`localhost`, `127.0.0.1`, `::1`) when no explicit TLS configuration is provided.

## Directives

| Directive | Type | Default | Description |
|-----------|------|---------|-------------|
| `provider` | `local` | — | Must be set to `"local"` |
| `cache` | `<string>` | platform data dir | Path for certificate caching |

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

1. No explicit `tls` directive is configured
2. The host matches loopback criteria (`localhost`, `127.0.0.1`, `::1`)
3. The `local` provider is available

This means you can use HTTPS for development without any configuration:

```ferron
localhost {
    root "/var/www/local-site"
}

127.0.0.1 {
    root "/var/www/local-site"
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

A local root CA is generated on first use. The CA certificate is cached in the data directory and is valid for 10 years. **Manual trust is required** — you must import the CA into your OS or browser trust store.

### Leaf certificates

Leaf certificates are generated for each unique set of Subject Alternative Names (SANs). They are valid for 1 year and automatically regenerated when expired or when SANs change. When any loopback address is detected, all loopback addresses (`localhost`, `127.0.0.1`, `::1`) are automatically included in the certificate.

### Cache location

By default, certificates are stored in:

- **Linux/macOS**: `~/.local/share/ferron-local-tls/`
- **Windows**: `%LOCALAPPDATA%\ferron-local-tls\`

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

The local CA is **not automatically trusted** by your system or browser. You must manually import the CA certificate:

1. Find the CA certificate path (logged at server startup)
2. Import into your OS trust store or browser

### Development use only

- **Not suitable for production** — local certificates are not publicly trusted
- **Development and testing only** — use the ACME provider for public-facing sites
- **Manual trust management** — you control which devices trust your local CA

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

For details on TLS crypto options, see [Security and TLS](/docs/v3/configuration/security-tls).

## Migration from manual certificates

If you were previously using manual certificates for localhost development, you can switch to the local provider:

```ferron
# Before: manual certificates
localhost:443 {
    tls {
        provider manual
        cert "/path/to/localhost.crt"
        key "/path/to/localhost.key"
    }
}

# After: automatic local provider
localhost {
    # No explicit tls needed — automatically uses local provider
}
```

The local provider offers the same security with less manual certificate management.

## Notes and troubleshooting

### Certificate trust requirements

The local CA certificate must be manually imported into your system or browser trust store. The server logs the CA certificate path at startup — use this file to import the certificate.

### Cache directory permissions

If Ferron cannot write to the default cache directory, either:

- Create the directory manually and set appropriate permissions
- Configure a custom cache path with write permissions using the `cache` directive
- Run Ferron with a user that has write access to the default location

### Certificate regeneration

Leaf certificates are automatically regenerated when:

- They expire (after 1 year)
- The set of Subject Alternative Names (SANs) changes
- The cached certificate files are deleted

The CA certificate is regenerated only if the cached files are missing or corrupted.

### Browser certificate warnings

If you see security warnings in your browser:

1. **Check the certificate details** — ensure it's issued by "Ferron Local Root CA"
2. **Import the CA certificate** — add the CA to your OS or browser trust store
3. **Clear browser cache** — some browsers cache certificate trust decisions
4. **Restart your browser** — changes to certificate trust may require a restart

### Development vs production

Never use the local provider in production — local certificates are not publicly trusted and will cause security warnings for all visitors. Use the ACME provider for public-facing websites.

### Multiple loopback addresses

When any loopback address is detected, the local provider automatically includes all loopback addresses (`localhost`, `127.0.0.1`, `::1`) in the generated certificate for convenience.

## See also

- [Security and TLS](/docs/v3/configuration/security-tls) — cipher suites, ECDH curves, mTLS
- [ACME automatic TLS](/docs/v3/configuration/tls-acme) — production TLS certificates
- [HTTP host directives](/docs/v3/configuration/http-host) — per-host TLS configuration
