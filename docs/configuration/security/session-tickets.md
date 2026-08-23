---
title: "Configuration: TLS session ticket keys"
description: "Stateless TLS session resumption with automatic key rotation and file-backed persistence."
---

This page documents TLS session ticket key management. TLS session tickets enable stateless session resumption, allowing clients to resume previous TLS sessions without a full handshake. This improves performance and reduces latency for returning clients.

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

This configuration validates the key file and enables session tickets. Ferron loads the keys once at startup.

### Automatic key rotation (recommended for production)

To enable automatic key rotation:

```ferron
tls {
    provider manual
    cert "cert.pem"
    key "key.pem"
    ticket_keys {
        file "session_tickets.keys"
        auto_rotate
        rotation_interval "12h"
        max_keys 3
    }
}
```

This configuration:

- Generates initial keys if the file does not exist
- Automatically rotates keys every 12 hours
- Keeps up to 3 keys for decryption of old tickets without interruption
- Persists new keys to disk atomically on each rotation

### Configuration parameters

| Parameter           | Type         | Default | Required | Description                   |
| ------------------- | ------------ | ------- | -------- | ----------------------------- |
| `file`              | `<string>`   | none    | Yes      | Path to the ticket key file   |
| `auto_rotate`       | `[<bool>]`   | `false` | No       | Enable automatic key rotation |
| `rotation_interval` | `<duration>` | `12h`   | No       | How often to rotate keys      |
| `max_keys`          | `<int>`      | `3`     | No       | Maximum keys to retain (2–5)  |

## Key file format

The ticket key file follows a specific format:

- File size must be a multiple of 80 bytes
- Each 80-byte record contains:
  - **16 bytes**: key name (unique identifier)
  - **32 bytes**: AES-256 key (encryption/decryption)
  - **32 bytes**: HMAC-SHA256 key (authentication)

### Generating a key file manually

If you disable `auto_rotate`, you can generate keys externally:

```bash
# Generate a single 80-byte key
openssl rand 80 > session_tickets.keys

# Generate multiple keys (for rotation support)
openssl rand 80 > session_tickets.keys
openssl rand 80 >> session_tickets.keys
openssl rand 80 >> session_tickets.keys
```

> [!important]
> Use cryptographically secure randomness to generate keys.

#### Rotating key files

Rotate ticket keys manually like this:

```bash
# Rotate keys, keeping some previous keys from the old file
mv session_tickets.keys session_tickets.keys.old
openssl rand 80 > session_tickets.keys
# Append truncated old file, without some oldest keys
head -c -80 session_tickets.keys.old >> session_tickets.keys

# Reload the server
sudo kill -HUP $(pidof ferron)
```

### File permissions

The ticket key file contains sensitive cryptographic material. Set restrictive permissions:

```bash
chmod 600 session_tickets.keys
chown ferron:ferron session_tickets.keys
```

## How rotation works

When you enable `auto_rotate`:

1. **Initial setup**: If the key file does not exist, Ferron generates `max_keys` random keys
2. **Validation**: Ferron validates the existing file (size must be a multiple of 80 bytes)
3. **Runtime**: Ferron loads the keys and creates a `TicketKeyRotator`
4. **Rotation trigger**: When `rotation_interval` elapses
5. **Key generation**: Ferron generates a new cryptographically secure key
6. **File update**: Ferron prepends the new key, trims the file to `max_keys`, and writes atomically
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

## Security considerations

- **Enable `auto_rotate` for production deployments**: this makes sure session tickets are rotated regularly, which reduces the risk of a single key being compromised.
- **Rotate keys regularly**: this is to make sure key compromises are less effective (recommended: every 12-24 hours)
- **Do not expose keys publicly**: this would cause key compromise, which causes attackers to be able to decrypt TLS sessions using the keys. This includes logging the keys, world-readable permissions, or committing them to version control.
- **Do not use predictable values**: doing so would make it easier for attackers to compromise TLS sessions.

## See also

- [Security and TLS](/docs/v3/configuration/security/tls): cipher suites, ECDH curves, mTLS
- [ACME automatic TLS](/docs/v3/configuration/security/acme): session tickets with ACME-obtained certificates
- [OCSP stapling](/docs/v3/configuration/security/ocsp)
