---
title: "Configuration: TLS session ticket keys"
description: "Stateless TLS session resumption with automatic key rotation and file-backed persistence."
---

This page documents TLS session ticket key management. TLS session tickets enable stateless session resumption, allowing clients to resume previous TLS sessions without a full handshake. This improves performance and reduces latency for returning clients.

## Configuration

### Basic usage (static keys)

To enable session tickets with a pre-existing key file (works with any TLS provider):

```ferron
example.com {
    tls {
        provider manual
        cert cert.pem
        key key.pem
        ticket_keys {
            file session_tickets.keys
        }
    }
}
```

This configuration validates the key file and enables session tickets. Ferron loads the keys once at startup.

### Automatic key rotation (recommended for production)

To enable automatic key rotation:

```ferron
example.com {
    tls {
        provider manual
        cert cert.pem
        key key.pem
        ticket_keys {
            file session_tickets.keys
            auto_rotate
            rotation_interval "12h"
            max_keys 3
        }
    }
}
```

This configuration:

- Generates initial keys if the file does not exist
- Automatically rotates keys every 12 hours
- Keeps up to 3 keys for decryption of old tickets without interruption
- Persists new keys to disk atomically on each rotation

> [!tip]
> Automatic session ticket key rotation feature also supports keys being shared across a shared network volume.

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

When you enable `auto_rotate`, Ferron automatically generates and rotates session ticket keys. Session ticket keys must have their sizes being multiple of 80 bytes (80 bytes would be the size of one session ticket key).

The newest key would be used for encryption, while the other keys (including older ones) could only be used for decryption.

When a configured `rotation_interval` elapses, the keys are rotated (the oldest key is discarded, while the newest key is generated, added, and used for encryption). The maximum amount of keys that can be used is configured by `max_keys`.

This feature also saves session ticket keys to a file, so they can be reused across server restarts.

The process can be described like this (with `rotation_interval` being 12 hours and `max_keys` being set to 3):

- **Right after initialization**: keys A, B and C are present in session ticket key file. Since the key A is the newest, it will be used for encrypting new session tickets.
- **12 hours after initialization**: a new key D is generated, and the oldest key C is discarded. Keys D, A and B are now present in session ticket key file. Since the key D is the newest, it will be used for encrypting sesion tickets instead of the key A.
- **24 hours after initialization**: same as before, but a new key E is generated and the oldest key B is discarded.
- **36 hours after initialization**: same as before, but a new key F is generated and the oldest key A is discarded. The key A is no longer used for decryption session tickets, and old session tickets encrypted with that key cannot be used anymore.

## Security considerations

- **Enable `auto_rotate` for production deployments**: this makes sure session tickets are rotated regularly, which reduces the risk of a single key being compromised.
- **Rotate keys regularly**: this is to make sure key compromises are less effective (recommended: every 12-24 hours)
- **Do not expose keys publicly**: this would cause key compromise, which causes attackers to be able to decrypt TLS sessions using the keys. This includes logging the keys, world-readable permissions, or committing them to version control.
- **Do not use predictable values**: doing so would make it easier for attackers to compromise TLS sessions.

## See also

- [Security and TLS](/docs/configuration/security/tls): cipher suites, ECDH curves, mTLS
- [ACME automatic TLS](/docs/configuration/security/acme): session tickets with ACME-obtained certificates
- [OCSP stapling](/docs/configuration/security/ocsp)
