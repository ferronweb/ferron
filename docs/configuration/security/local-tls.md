---
title: "Configuration: local TLS provider"
description: "Locally trusted certificates for development and testing on loopback addresses."
---

The `local` provider generates a local root CA and leaf certificates for development and testing. Ferron selects it automatically for loopback addresses (`localhost`, `127.0.0.1`, `::1`) when no explicit TLS configuration exists.

## Options

| Directive  | Value      | Default           | Description                  |
| ---------- | ---------- | ----------------- | ---------------------------- |
| `provider` | `local`    | none              | Set to `local`.              |
| `cache`    | `<string>` | platform data dir | Path for certificate caching |

```ferron
localhost:443 {
    tls {
        provider local
        cache /custom/cache/path
    }
}
```

## Automatic selection

Ferron selects the `local` provider for a loopback address when all of these conditions hold:

1. You have not configured an explicit `tls` directive.
2. The host is `localhost`, `127.0.0.1`, or `::1`.
3. The `local` provider is available.

Development hosts then get HTTPS without TLS configuration:

```ferron
localhost {
    root /var/www/local-site
}

127.0.0.1 {
    root /var/www/local-site
}
```

## Certificates

Ferron generates a local root CA on first use and caches the CA certificate in the data directory. The CA is valid for 10 years. You must import it into your OS or browser trust store yourself.

Ferron generates a leaf certificate for each unique set of Subject Alternative Names (SANs). Leaf certificates are valid for 1 year. Ferron regenerates them when they expire, when the SANs change, or when you delete the cached files. When Ferron detects any loopback address, the certificate includes all loopback addresses (`localhost`, `127.0.0.1`, `::1`).

Ferron regenerates the CA certificate only when the cached files are missing or corrupted. The certificate issuer is `Ferron Local Root CA`.

## Cache location

| Platform    | Default path                            |
| ----------- | --------------------------------------- |
| Linux/macOS | `~/.local/share/ferron-local-tls/`      |
| Windows     | `%LOCALAPPDATA%\ferron-local-tls\`      |

If Ferron cannot use the cache directory, it uses a temporary fallback location. If it cannot write to the default directory, do one of these:

- Create the directory and set permissions.
- Set `cache` to a writable path.
- Run Ferron as a user with write access to the default location.

## Crypto options

The local provider accepts the same TLS crypto directives as other providers: `min_version`, `max_version`, `cipher_suite`, and `ecdh_curve`. See [Security and TLS](/docs/configuration/security/tls).

## Trust the local CA

Your system or browser does not trust the local CA automatically. Ferron logs the CA certificate path at startup.

1. Find the CA certificate path in the startup log.
2. Import the certificate into your OS trust store or browser.

If the browser shows a security warning:

1. Check that the certificate issuer is `Ferron Local Root CA`.
2. Import the CA certificate.
3. Clear the browser cache, because some browsers cache certificate trust decisions.
4. Restart the browser.

> [!warning]
> Never use the local provider in production. Local certificates are not publicly trusted and cause security warnings for every visitor. Use the ACME provider for public sites.

## Migration from manual certificates

Replace manual localhost certificates with the provider:

```ferron
# Before: manual certificates
localhost:443 {
    tls /path/to/localhost.crt /path/to/localhost.key
}

# After: automatic local provider
localhost {
    # No explicit tls needed - automatically uses local provider
}
```

## See also

- [Security and TLS](/docs/configuration/security/tls): cipher suites, ECDH curves, mTLS
- [ACME automatic TLS](/docs/configuration/security/acme): production TLS certificates
- [HTTP host directives](/docs/configuration/server/host): per-host TLS configuration

## Best practices

`ferron doctor` reports the following best-practice check for directives on this page.

- `provider local` on non-loopback hosts: The local TLS provider issues self-signed certificates. Use ACME or manual certificates for production hostnames.
