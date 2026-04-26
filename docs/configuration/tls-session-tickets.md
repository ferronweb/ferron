---
title: "Configuration: TLS session ticket keys"
description: "Stateless TLS session resumption with automatic key rotation and file-backed persistence."
---

This page documents TLS session ticket key management (`tls-manual` module). TLS session tickets enable **stateless session resumption**, allowing clients to resume previous TLS sessions without a full handshake. This improves performance and reduces latency for returning clients.

## Configuration

### Basic usage (static keys)

To enable session tickets with a pre-existing key file (works with any TLS provider):

```ferron
tls {
    provider manual
    cert "cert.pem"
    key "key.pem"
    ticket_keys {
        file "session_tickets.keys"
    }
}
```

This configuration validates the key file and enables session tickets. Keys are loaded once at startup.

### Automatic key rotation (recommended for production)

To enable automatic key rotation:

```ferron
tls {
    provider manual
    cert "cert.pem"
    key "key.pem"
    ticket_keys {
        file "session_tickets.keys"
        auto_rotate true
        rotation_interval "12h"
        max_keys 3
    }
}
```

This configuration:

- Generates initial keys if the file doesn't exist
- Automatically rotates keys every 12 hours
- Keeps up to 3 keys for seamless decryption of old tickets
- Persists new keys to disk atomically on each rotation

### Configuration parameters

| Parameter | Type | Default | Required | Description |
|-----------|------|---------|----------|-------------|
| `file` | `<string>` | — | Yes | Path to the ticket key file |
| `auto_rotate` | `<bool>` | `false` | No | Enable automatic key rotation |
| `rotation_interval` | `<duration>` | `12h` | No | How often to rotate keys |
| `max_keys` | `<int>` | `3` | No | Maximum keys to retain (2–5) |

## Key file format

The ticket key file follows a specific format:

- File size must be a multiple of **80 bytes**
- Each 80-byte record contains:
  - **16 bytes**: key name (unique identifier)
  - **32 bytes**: AES-256 key (encryption/decryption)
  - **32 bytes**: HMAC-SHA256 key (authentication)

### Generating a key file manually

If `auto_rotate` is disabled, you can generate keys externally:

```bash
# Generate a single 80-byte key
openssl rand 80 > session_tickets.keys

# Generate multiple keys (for rotation support)
openssl rand 80 > session_tickets.keys
openssl rand 80 >> session_tickets.keys
openssl rand 80 >> session_tickets.keys
```

**Important:** Keys must be generated using cryptographically secure randomness.

### File permissions

The ticket key file contains sensitive cryptographic material. Set restrictive permissions:

```bash
chmod 600 session_tickets.keys
chown ferron:ferron session_tickets.keys
```

## How rotation works

When `auto_rotate` is enabled:

1. **Initial setup**: If the key file doesn't exist, Ferron generates `max_keys` random keys
2. **Validation**: The existing file is validated (size must be multiple of 80 bytes)
3. **Runtime**: Keys are loaded and a `TicketKeyRotator` is created
4. **Rotation trigger**: When `rotation_interval` elapses
5. **Key generation**: A new cryptographically secure key is generated
6. **File update**: New key is prepended, file is trimmed to `max_keys`, atomic write
7. **Memory update**: Current → previous, new key becomes current

### Example: 12-hour rotation

With `rotation_interval = "12h"` and `max_keys = 3`:

```text
T=0h:   [Key_A, Key_B, Key_C]     ← Encrypt with Key_A
T=12h:  [Key_D, Key_A, Key_B]     ← Encrypt with Key_D, decrypt with A/B
T=24h:  [Key_E, Key_D, Key_A]     ← Encrypt with Key_E, decrypt with A/D/E
T=36h:  [Key_F, Key_E, Key_D]     ← Key_A removed (expired)
```

Tickets issued with `Key_A` at T=0h remain valid until ~T=24h (2× interval).

### Rotation duration format

The `rotation_interval` parameter accepts:

- `"12h"` or `"12H"` — hours
- `"30m"` or `"30M"` — minutes
- `"90s"` or `"90S"` — seconds
- `"1d"` or `"1D"` — days
- `"12"` — plain number (treated as hours)

## Security considerations

### Do's

- Enable `auto_rotate` for production deployments
- Set restrictive file permissions (`chmod 600`)
- Rotate keys regularly (recommended: every 12–24 hours)
- Keep 2–3 keys during rotation for smooth transition

### Don'ts

- **Never log key content** — Ferron never logs key bytes
- **Don't use predictable values** — no hardcoded or weak keys
- **Don't expose files** — avoid world-readable permissions
- **Don't rotate all keys at once** — keep old keys for overlap during rotation
- **Don't commit keys to version control** — add to `.gitignore`

## Notes and troubleshooting

### "Ticket keys file not found"

The key file doesn't exist and `auto_rotate` is disabled.

**Fix:** Either enable `auto_rotate` or create the file manually with `openssl rand 80 > session_tickets.keys`.

### "TLS ticket key file is empty"

The key file exists but has zero bytes.

**Fix:** Generate at least one 80-byte key.

### "TLS ticket key file size (X) is not a multiple of 80 bytes"

The file size is incorrect.

**Fix:** Ensure the file contains complete 80-byte records.

### Debugging

Enable debug logging to see ticket key events:

```bash
ferron run --verbose
```

You should see messages like:

```text
Generating initial ticket keys at /path/to/session_tickets.keys (3 keys)
Loaded 3 ticket keys from /path/to/session_tickets.keys (rotation interval: 12h)
TLS session ticket keys rotated successfully
```

## See also

- [Security and TLS](/docs/v3/configuration/security-tls) — cipher suites, ECDH curves, mTLS
- [ACME automatic TLS](/docs/v3/configuration/tls-acme) — session tickets with ACME-obtained certificates
- [OCSP stapling](/docs/v3/configuration/ocsp-stapling)
