# TLS Session Ticket Keys

## Overview

TLS session tickets enable **stateless session resumption**, allowing clients to resume previous TLS sessions without a full handshake. This improves performance and reduces latency for returning clients.

In Ferron 3, session ticket keys are managed through the `tls "manual"` provider, with full support for **automatic key rotation** and file-backed persistence. This enables:

- **Session resumption across restarts**: Survive configuration reloads and server restarts
- **Multi-instance support**: Share the same keys across multiple Ferron instances
- **Automatic rotation**: Cryptographically secure keys generated and rotated on a schedule

## Configuration

### Basic Usage (Static Keys)

To enable session tickets with a pre-existing key file:

```
tls "manual" {
    cert "cert.pem"
    key "key.pem"
    ticket_keys {
        file "session_tickets.keys"
    }
}
```

This configuration validates the key file and enables session tickets. Keys are loaded once at startup.

### Automatic Key Rotation (Recommended for Production)

To enable automatic key rotation:

```
tls "manual" {
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

### Configuration Parameters

| Parameter | Type | Default | Required | Description |
|-----------|------|---------|----------|-------------|
| `file` | string | - | Yes | Path to the ticket key file |
| `auto_rotate` | bool | `false` | No | Enable automatic key rotation |
| `rotation_interval` | duration | `12h` | No | How often to rotate keys |
| `max_keys` | int | `3` | No | Maximum keys to retain (2-5) |

## Key File Format

The ticket key file follows a specific format:

- File size must be a multiple of **80 bytes**
- Each 80-byte record contains:
  - **16 bytes**: Key Name (unique identifier)
  - **32 bytes**: AES-256 Key (encryption/decryption)
  - **32 bytes**: HMAC-SHA256 Key (authentication)

### Example: Generating a Key File Manually

If `auto_rotate` is disabled, you can generate keys externally:

```bash
# Generate a single 80-byte key
openssl rand 80 > session_tickets.keys

# Generate multiple keys (for rotation support)
openssl rand 80 > session_tickets.keys
openssl rand 80 >> session_tickets.keys
openssl rand 80 >> session_tickets.keys
```

**Important**: Keys must be generated using cryptographically secure randomness.

### File Permissions

The ticket key file contains sensitive cryptographic material. Set restrictive permissions:

```bash
chmod 600 session_tickets.keys
chown ferron:ferron session_tickets.keys
```

The file should be readable only by the user running the Ferron process.

## Automatic Key Rotation

### How Rotation Works

When `auto_rotate` is enabled:

1. **Initial Setup**: If the key file doesn't exist, Ferron generates `max_keys` random keys
2. **Validation**: The existing file is validated (size must be multiple of 80 bytes)
3. **Runtime**: Keys are loaded and a `TicketKeyRotator` is created
4. **Rotation Trigger**: When `rotation_interval` elapses
5. **Key Generation**: A new cryptographically secure key is generated
6. **File Update**: New key is prepended, file is trimmed to `max_keys`, atomic write
7. **Memory Update**: Current → previous, new key becomes current
8. **Logging**: Rotation event is logged

### Example: 12-Hour Rotation

With `rotation_interval = "12h"` and `max_keys = 3`:

```
T=0h:   [Key_A, Key_B, Key_C]     ← Encrypt with Key_A
T=12h:  [Key_D, Key_A, Key_B]     ← Encrypt with Key_D, decrypt with A/B
T=24h:  [Key_E, Key_D, Key_A]     ← Encrypt with Key_E, decrypt with A/D/E
T=36h:  [Key_F, Key_E, Key_D]     ← Key_A removed (expired)
```

Tickets issued with `Key_A` at T=0h remain valid until ~T=24h (2× interval).

### Rotation Duration Format

The `rotation_interval` parameter accepts:
- `"12h"` or `"12H"` - Hours
- `"30m"` or `"30M"` - Minutes
- `"90s"` or `"90S"` - Seconds
- `"1d"` or `"1D"` - Days
- `"12"` - Plain number (treated as hours)

### Multi-Instance Considerations

When running multiple Ferron instances:
- Use shared storage for the key file (NFS, synced directory, etc.)
- Each instance rotates independently
- File updates propagate via the shared filesystem
- Consider external coordination if instances don't share storage

## Security Considerations

### Do's ✅

- Enable `auto_rotate` for production deployments
- Generate keys using cryptographically secure randomness (handled automatically)
- Set restrictive file permissions (`chmod 600`)
- Rotate keys regularly (recommended: every 12-24 hours)
- Keep 2-3 keys during rotation for smooth transition
- Store key files in secure locations (e.g., encrypted volumes)

### Don'ts ❌

- **Never log key content** - Ferron never logs key bytes
- **Don't use predictable values** - No hardcoded or weak keys
- **Don't expose files** - Avoid world-readable permissions
- **Don't rotate all keys at once** - Keep old keys for overlap during rotation
- **Don't commit keys to version control** - Add to `.gitignore`

## Troubleshooting

### Common Errors

#### "Ticket keys file not found"

The key file doesn't exist and `auto_rotate` is disabled.

**Fix**: Either enable `auto_rotate` or create the file manually with `openssl rand 80 > session_tickets.keys`.

#### "TLS ticket key file is empty"

The key file exists but has zero bytes.

**Fix**: Generate at least one 80-byte key.

#### "TLS ticket key file size (X) is not a multiple of 80 bytes"

The file size is incorrect.

**Fix**: Ensure the file contains complete 80-byte records.

#### "No such file or directory"

The ticket key file path is incorrect or inaccessible.

**Fix**: Verify the path is correct and accessible to the Ferron process.

### Debugging

Enable debug logging to see ticket key events:

```
# In Ferron configuration
log_level "debug"
```

You should see messages like:
```
INFO Generating initial ticket keys at /path/to/session_tickets.keys (3 keys)
INFO Loaded 3 ticket keys from /path/to/session_tickets.keys (rotation interval: 12h)
INFO TLS session ticket key rotation enabled (interval: 12h, max_keys: 3)
INFO TLS session ticket keys rotated successfully
```

Or on error:
```
ERROR Failed to load TLS session ticket keys from /path/to/session_tickets.keys: <error details>
```

## Integration with Config Reload

Ferron's configuration reload system ensures safe key rotation:

1. **Config reload triggered** (via `SIGHUP` or file change)
2. **TLS provider re-executes** with new configuration
3. **Ticket keys validated** from the specified file
4. **New `TicketKeyRotator` created** (if auto_rotate enabled)
5. **Atomic swap** via `ArcSwap` - zero downtime
6. **Old connections** continue with old config
7. **New connections** use new config

If key file validation fails during reload:
- **Reload is rejected** - old config is retained
- **Error is logged** - operator is notified
- **No service disruption** - existing connections continue normally

## Architecture

### Data Flow

```
session_tickets.keys file
    ↓
generate_initial_ticket_keys() if missing + auto_rotate
    ↓
load_ticket_keys() → Vec<TicketKey>
    ↓
TicketKeyRotator::new() → Arc<dyn ProducesTickets>
    ↓
ServerConfig.ticketer = rotator
    ↓
TcpTlsManualResolver wraps ServerConfig
    ↓
TlsResolverRadixTree stores resolver
    ↓
ArcSwap atomic swap on config reload
```

### Ticket Encryption

The `TicketKeyRotator` implements RFC 5077 ticket format:
- **Encryption**: AES-256-CBC with PKCS#7 padding
- **Authentication**: HMAC-SHA256
- **Ticket structure**: IV (16B) || Ciphertext || HMAC (32B)

Keys are rotated automatically using the `maybe_roll()` pattern:
- Fast path: Read-only check, no lock contention
- Slow path: Generate keys **outside** the lock, then atomic swap
- Double-check: Another thread might have rotated first
- Graceful: Failed rotation keeps old keys working

### Module Responsibilities

- **`types/tls`**: Core ticket key generation, validation, persistence, and `TicketKeyRotator`
- **`modules/tls-manual`**: Provider that integrates configuration and creates rotator
- **`core`**: Configuration reload infrastructure (no TLS-specific logic)

## References

- [RFC 5077: Transport Layer Security (TLS) Session Resumption](https://tools.ietf.org/html/rfc5077)
- [rustls documentation](https://docs.rs/rustls/)
- [aws-lc-rs documentation](https://docs.rs/aws-lc-rs/)
- [Ferron TLS Provider Architecture](../README.md)
