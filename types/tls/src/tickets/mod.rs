//! TLS session ticket key management.
//!
//! This module provides utilities for managing TLS session ticket keys,
//! enabling stateless session resumption across multiple server instances and
//! surviving configuration reloads.
//!
//! # Current Implementation
//!
//! Due to limitations in rustls 0.23's public API, this module currently provides:
//! - Validation utilities for key file formats
//! - Documentation of the expected key file format
//!
//! Custom key loading will be fully supported when rustls exposes the necessary
//! APIs or when we implement a custom `ProducesTickets` trait.
//!
//! # Key File Format (for future use)
//!
//! The key file will consist of one or more 80-byte records. Each record contains:
//! - 16 bytes: Key Name (identifier)
//! - 32 bytes: AES-256-GCM Key
//! - 32 bytes: HMAC-SHA256 Key
//!
//! The first key in the file will be used for encryption (issuing new tickets),
//! while all keys will be used for decryption (resuming existing tickets).
//!
//! # Key Rotation (for future use)
//!
//! To rotate keys:
//! 1. Generate a new 80-byte key
//! 2. Prepend it to the key file (keeping 1-2 older keys for overlap)
//! 3. Trigger a configuration reload (e.g., via SIGHUP)
//!
//! # Security Considerations
//!
//! - Key files should be readable only by the server user (e.g., `chmod 600`)
//! - Keys must be generated externally using cryptographically secure randomness
//! - Key content is never logged

use aws_lc_rs::cipher::{
    PaddedBlockDecryptingKey, PaddedBlockEncryptingKey, UnboundCipherKey, AES_256, AES_CBC_IV_LEN,
};
use aws_lc_rs::hmac::{self, Key as HmacKey};
use aws_lc_rs::iv::FixedLength;
use rustls::server::ProducesTickets;
use rustls_pki_types::UnixTime;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::RwLock;
use std::time::Duration;

use ferron_core::config::ServerConfigurationBlock;
use ferron_core::util::parse_duration;

/// Configuration for automatic ticket key rotation.
#[derive(Debug, Clone)]
pub struct TicketKeyRotationConfig {
    /// Path to the ticket key file
    pub file: String,
    /// Whether automatic rotation is enabled
    pub auto_rotate: bool,
    /// How often to rotate keys (default: 12 hours)
    pub rotation_interval: Duration,
    /// Maximum number of keys to keep (default: 3)
    pub max_keys: usize,
}

impl Default for TicketKeyRotationConfig {
    fn default() -> Self {
        Self {
            file: String::new(),
            auto_rotate: false,
            rotation_interval: Duration::from_secs(12 * 3600),
            max_keys: 3,
        }
    }
}

impl TicketKeyRotationConfig {
    /// Parse ticket key rotation configuration from a ServerConfigurationBlock.
    pub fn from_config(config: &ServerConfigurationBlock) -> Option<Self> {
        // Look for ticket_keys nested block
        let ticket_keys_directive = config.directives.get("ticket_keys")?;
        let ticket_keys_entry = ticket_keys_directive.first()?;
        let ticket_keys_block = ticket_keys_entry.children.as_ref()?;

        // Extract file path (required)
        let file = ticket_keys_block
            .get_value("file")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))?;

        // Extract auto_rotate (optional, default: false)
        let auto_rotate = ticket_keys_block
            .get_value("auto_rotate")
            .and_then(|v| v.as_boolean())
            .unwrap_or(false);

        // Extract rotation_interval (optional, default: 12h)
        let rotation_interval = ticket_keys_block
            .get_value("rotation_interval")
            .and_then(|v| {
                if let Some(si) = v.as_string_with_interpolations(&HashMap::new()) {
                    Some(parse_duration(&si).unwrap_or(Duration::from_secs(12 * 3600)))
                } else {
                    v.as_number().map(|n| Duration::from_secs(n as u64))
                }
            })
            .unwrap_or(Duration::from_secs(12 * 3600));

        // Extract max_keys (optional, default: 3, range: 2-5)
        let max_keys = ticket_keys_block
            .get_value("max_keys")
            .and_then(|v| v.as_number())
            .map(|n| {
                let n = n as usize;
                if n < 2 {
                    ferron_core::log_warn!(
                        "ticket_keys.max_keys={} is too small, using minimum of 2",
                        n
                    );
                    2
                } else if n > 5 {
                    ferron_core::log_warn!(
                        "ticket_keys.max_keys={} is too large, using maximum of 5",
                        n
                    );
                    5
                } else {
                    n
                }
            })
            .unwrap_or(3);

        Some(Self {
            file,
            auto_rotate,
            rotation_interval,
            max_keys,
        })
    }
}

/// The size of a single ticket key record in bytes.
///
/// Each record contains:
/// - 16 bytes: Key Name
/// - 32 bytes: AES-256-GCM Key
/// - 32 bytes: HMAC-SHA256 Key
pub const TICKET_KEY_RECORD_SIZE: usize = 80;

/// Recommended maximum number of ticket keys to load.
///
/// Keeping 2-3 keys allows for smooth rotation while preventing
/// unbounded memory growth. Keys beyond this limit will be silently ignored
/// with a warning logged.
pub const MAX_TICKET_KEYS: usize = 3;

/// A parsed ticket key record.
pub type TicketKeyComponents = ([u8; 16], [u8; 32], [u8; 32]);

/// Generate a single cryptographically secure ticket key record.
///
/// Creates an 80-byte key suitable for use with TLS session tickets.
/// The key contains:
/// - 16 bytes: Random key name (identifier)
/// - 32 bytes: Random AES-256-GCM key
/// - 32 bytes: Random HMAC-SHA256 key
///
/// # Returns
///
/// A `[u8; 80]` array containing the generated key.
///
/// # Security Notes
///
/// - Uses system CSPRNG via `getrandom`
/// - Never log the returned key bytes
/// - Store generated keys securely with restrictive permissions
///
/// # Example
///
/// ```
/// use ferron_tls::tickets::generate_ticket_key;
///
/// let key = generate_ticket_key();
/// assert_eq!(key.len(), 80);
/// ```
pub fn generate_ticket_key() -> [u8; TICKET_KEY_RECORD_SIZE] {
    let mut key = [0u8; TICKET_KEY_RECORD_SIZE];
    getrandom::fill(&mut key).expect("failed to generate random bytes for ticket key");
    key
}

/// Generate initial ticket keys and write to a file.
///
/// Creates a new key file with `num_keys` randomly generated keys.
/// This is useful for bootstrapping automatic key rotation.
///
/// # Arguments
///
/// * `filename` - Path where the key file will be created
/// * `num_keys` - Number of keys to generate (clamped to 1..=5)
///
/// # Returns
///
/// - `Ok(())` on success
/// - `Err` if file cannot be created or written
///
/// # Security Notes
///
/// - File is created with permissions `0o600` (readable only by owner) on Unix
/// - Keys are generated using cryptographically secure randomness
/// - Key content is never logged
///
/// # Example
///
/// ```no_run
/// use ferron_tls::tickets::generate_initial_ticket_keys;
///
/// generate_initial_ticket_keys("/path/to/session_tickets.keys", 3)
///     .expect("Failed to generate ticket keys");
/// ```
pub fn generate_initial_ticket_keys(filename: &str, num_keys: usize) -> std::io::Result<()> {
    // Clamp to reasonable range
    let num_keys = num_keys.clamp(1, 5);

    let path = Path::new(filename);

    // Create parent directories if needed
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Generate keys
    let mut data = Vec::with_capacity(num_keys * TICKET_KEY_RECORD_SIZE);
    for _ in 0..num_keys {
        let key = generate_ticket_key();
        data.extend_from_slice(&key);
    }

    // Write atomically: write to temp file, then rename
    let tmp_path = path.with_extension("keys.tmp");
    {
        let mut file = fs::File::create(&tmp_path)?;

        // Set restrictive permissions on Unix before writing
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = file.metadata()?.permissions();
            perms.set_mode(0o600);
            file.set_permissions(perms)?;
        }

        file.write_all(&data)?;
        file.sync_all()?;
    }

    // Atomic rename
    fs::rename(&tmp_path, path)?;

    // Sync parent directory to ensure rename is persisted
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let parent_dir = fs::File::open(parent)?;
        parent_dir.sync_all().ok(); // Ignore errors on some filesystems
    }

    Ok(())
}

/// Persist ticket keys to file atomically.
///
/// Writes the keys to a temporary file and then atomically renames
/// it to the target path. This ensures the file is never left in a
/// corrupted state if the process crashes during writing.
///
/// # Arguments
///
/// * `filename` - Target file path
/// * `keys` - Slice of ticket key components to persist
///
/// # Returns
///
/// - `Ok(())` on success
/// - `Err` if file cannot be written
///
/// # Security Notes
///
/// - Uses atomic write pattern (temp file + rename)
/// - Sets `0o600` permissions on Unix
/// - Syncs file and parent directory to disk
pub fn persist_ticket_keys(filename: &str, keys: &[TicketKeyComponents]) -> std::io::Result<()> {
    if keys.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Cannot persist empty ticket keys",
        ));
    }

    let path = Path::new(filename);

    // Create parent directories if needed
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Serialize keys
    let mut data = Vec::with_capacity(keys.len() * TICKET_KEY_RECORD_SIZE);
    for (key_name, aes_key, hmac_key) in keys {
        data.extend_from_slice(key_name);
        data.extend_from_slice(aes_key);
        data.extend_from_slice(hmac_key);
    }

    // Write atomically
    let tmp_path = path.with_extension("keys.tmp");
    {
        let mut file = fs::File::create(&tmp_path)?;

        // Set restrictive permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = file.metadata()?.permissions();
            perms.set_mode(0o600);
            file.set_permissions(perms)?;
        }

        file.write_all(&data)?;
        file.sync_all()?;
    }

    // Atomic rename
    fs::rename(&tmp_path, path)?;

    // Sync parent directory
    if let Some(parent) = path.parent() {
        let parent_dir = fs::File::open(parent)?;
        parent_dir.sync_all().ok();
    }

    Ok(())
}

/// Validate a TLS session ticket key file.
///
/// This function checks that the file exists, is readable, and has a valid
/// format (size is a multiple of [`TICKET_KEY_RECORD_SIZE`]).
///
/// # Arguments
///
/// * `filename` - Path to the key file to validate
///
/// # Returns
///
/// - `Ok(num_keys)` if the file is valid, where `num_keys` is the number of keys
/// - `Err` if the file is invalid or cannot be read
///
/// # Errors
///
/// Returns an error if:
/// - The file cannot be opened or read
/// - The file size is not a multiple of [`TICKET_KEY_RECORD_SIZE`]
/// - The file is empty (zero keys)
///
/// # Security Notes
///
/// - This function does NOT validate file permissions
/// - Key content is never logged, even on error
/// - Keys must be generated externally with secure randomness
///
/// # Example
///
/// ```no_run
/// use ferron_tls::tickets::validate_ticket_keys_file;
///
/// let num_keys = validate_ticket_keys_file("/path/to/session_tickets.keys")
///     .expect("Failed to validate ticket keys file");
/// println!("File contains {} valid keys", num_keys);
/// ```
pub fn validate_ticket_keys_file(filename: &str) -> std::io::Result<usize> {
    // Read the file metadata to check size
    let metadata = fs::metadata(filename)?;
    let file_size = metadata.len() as usize;

    // Validate file size
    if file_size == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "TLS ticket key file is empty",
        ));
    }

    if !file_size.is_multiple_of(TICKET_KEY_RECORD_SIZE) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "TLS ticket key file size ({}) is not a multiple of {} bytes",
                file_size, TICKET_KEY_RECORD_SIZE
            ),
        ));
    }

    let num_keys = file_size / TICKET_KEY_RECORD_SIZE;

    if num_keys == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "TLS ticket key file contains no keys",
        ));
    }

    // Warn if more than recommended maximum
    if num_keys > MAX_TICKET_KEYS {
        ferron_core::log_warn!(
            "TLS ticket key file contains {} keys, but only the first {} will be used. \
             Consider removing older keys to avoid confusion.",
            num_keys,
            MAX_TICKET_KEYS
        );
    }

    Ok(num_keys)
}

/// Read and parse a TLS session ticket key file into raw key components.
///
/// This function reads the file and returns the raw components that would be
/// needed to implement a custom `ProducesTickets` trait:
/// - Key names (16 bytes each)
/// - AES-256-GCM keys (32 bytes each)
/// - HMAC-SHA256 keys (32 bytes each)
///
/// # Arguments
///
/// * `filename` - Path to the key file
///
/// # Returns
///
/// A vector of tuples `(key_name, aes_key, hmac_key)` for each key in the file.
///
/// # Errors
///
/// Returns an error if:
/// - The file cannot be opened or read
/// - The file size is not a multiple of [`TICKET_KEY_RECORD_SIZE`]
/// - The file is empty (zero keys)
///
/// # Security Notes
///
/// - **NEVER log the returned values** - they contain sensitive key material
/// - Key files should have restrictive permissions (e.g., `chmod 600`)
/// - Keys must be generated externally with cryptographically secure randomness
pub fn load_ticket_keys(filename: &str) -> std::io::Result<Vec<TicketKeyComponents>> {
    // Read the entire file
    let data = fs::read(filename)?;

    // Validate file size
    if data.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "TLS ticket key file is empty",
        ));
    }

    if !data.len().is_multiple_of(TICKET_KEY_RECORD_SIZE) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "TLS ticket key file size ({}) is not a multiple of {} bytes",
                data.len(),
                TICKET_KEY_RECORD_SIZE
            ),
        ));
    }

    let num_keys = data.len() / TICKET_KEY_RECORD_SIZE;

    if num_keys == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "TLS ticket key file contains no keys",
        ));
    }

    // Warn if more than recommended maximum
    if num_keys > MAX_TICKET_KEYS {
        ferron_core::log_warn!(
            "TLS ticket key file contains {} keys, but only the first {} will be used. \
             Consider removing older keys to avoid confusion.",
            num_keys,
            MAX_TICKET_KEYS
        );
    }

    // Parse keys
    let mut keys = Vec::new();
    for (i, chunk) in data.chunks_exact(TICKET_KEY_RECORD_SIZE).enumerate() {
        // Stop at maximum
        if keys.len() >= MAX_TICKET_KEYS {
            break;
        }

        let key = parse_ticket_key_record(chunk, i)?;

        // Check if the key is all zeros (invalid) and skip it with a warning
        if key.0.iter().all(|&b| b == 0) {
            continue;
        }

        keys.push(key);
    }

    if keys.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "No valid ticket keys found in file",
        ));
    }

    Ok(keys)
}

/// Parse a single 80-byte ticket key record into its components.
///
/// # Arguments
///
/// * `data` - Exactly 80 bytes of key data
/// * `index` - Index of this key in the file (for error messages)
///
/// # Returns
///
/// A tuple of `(key_name, aes_key, hmac_key)`.
///
/// # Errors
///
/// Returns an error if the data slice is not exactly 80 bytes.
fn parse_ticket_key_record(
    data: &[u8],
    index: usize,
) -> std::io::Result<([u8; 16], [u8; 32], [u8; 32])> {
    if data.len() != TICKET_KEY_RECORD_SIZE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "Ticket key record {} is {} bytes, expected {} bytes",
                index,
                data.len(),
                TICKET_KEY_RECORD_SIZE
            ),
        ));
    }

    // Extract components according to the format:
    // - 16 bytes: Key Name
    // - 32 bytes: AES-256-GCM Key
    // - 32 bytes: HMAC-SHA256 Key
    let mut key_name = [0u8; 16];
    let mut aes_key = [0u8; 32];
    let mut hmac_key = [0u8; 32];

    key_name.copy_from_slice(&data[0..16]);
    aes_key.copy_from_slice(&data[16..48]);
    hmac_key.copy_from_slice(&data[48..80]);

    Ok((key_name, aes_key, hmac_key))
}

// ============================================================================
// Custom ProducesTickets Implementation for Automatic Key Rotation
// ============================================================================

/// A ticket key with all its components.
#[derive(Clone)]
pub struct TicketKey {
    /// 16-byte key name/identifier
    pub key_name: [u8; 16],
    /// 32-byte AES-256 key
    pub aes_key: [u8; 32],
    /// 32-byte HMAC-SHA256 key
    pub hmac_key: [u8; 32],
}

impl TicketKey {
    /// Create a TicketKey from parsed components.
    pub fn new(key_name: [u8; 16], aes_key: [u8; 32], hmac_key: [u8; 32]) -> Self {
        Self {
            key_name,
            aes_key,
            hmac_key,
        }
    }
}

impl fmt::Debug for TicketKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TicketKey")
            .field("key_name", &self.key_name)
            .finish_non_exhaustive()
    }
}

/// A single ticket encryptor using a specific key.
///
/// Implements RFC 5077 ticket format:
/// - Encryption: AES-256-CBC with PKCS#7 padding
/// - Authentication: HMAC-SHA256
/// - Ticket structure: IV (16B) || Ciphertext || HMAC (32B)
struct CustomTicketEncryptor {
    /// AES-256 encryption key
    encrypt_key: PaddedBlockEncryptingKey,
    /// AES-256 decryption key
    decrypt_key: PaddedBlockDecryptingKey,
    /// HMAC-SHA256 key
    hmac_key: HmacKey,
    /// Key name (sent in clear with tickets)
    _key_name: [u8; 16],
}

impl CustomTicketEncryptor {
    /// Create a new encryptor from a ticket key.
    fn new(key: &TicketKey) -> Result<Self, aws_lc_rs::error::Unspecified> {
        // Create AES keys for both encryption and decryption
        let aes_unbound = UnboundCipherKey::new(&AES_256, &key.aes_key)?;
        let encrypt_key = PaddedBlockEncryptingKey::cbc_pkcs7(aes_unbound)?;

        let aes_unbound = UnboundCipherKey::new(&AES_256, &key.aes_key)?;
        let decrypt_key = PaddedBlockDecryptingKey::cbc_pkcs7(aes_unbound)?;

        // Create HMAC key
        let hmac_key = HmacKey::new(hmac::HMAC_SHA256, &key.hmac_key);

        Ok(Self {
            encrypt_key,
            decrypt_key,
            hmac_key,
            _key_name: key.key_name,
        })
    }

    /// Encrypt plaintext into a ticket.
    ///
    /// Ticket format:
    /// - 16 bytes: IV (random)
    /// - N bytes: Ciphertext (AES-256-CBC encrypted plaintext with PKCS#7 padding)
    /// - 32 bytes: HMAC-SHA256 tag (over IV || Ciphertext)
    fn encrypt(&self, plaintext: &[u8]) -> Option<Vec<u8>> {
        // Prepare buffer with plaintext (will be encrypted in-place)
        let mut in_out = plaintext.to_vec();

        // Encrypt in-place (adds PKCS#7 padding automatically, generates random IV)
        let decrypt_ctx = self.encrypt_key.encrypt(&mut in_out).ok()?;

        // Extract IV from the DecryptionContext
        let iv: &[u8] = (&decrypt_ctx).try_into().ok()?;

        // Build ticket: IV || Ciphertext
        let mut ticket = iv.to_vec();
        ticket.extend_from_slice(&in_out);

        // Calculate HMAC-SHA256 over IV || Ciphertext
        let hmac = hmac::sign(&self.hmac_key, &ticket);
        ticket.extend_from_slice(hmac.as_ref());

        Some(ticket)
    }

    /// Decrypt a ticket back into plaintext.
    ///
    /// Returns None if:
    /// - Ticket is too short
    /// - HMAC verification fails
    /// - Decryption fails
    fn decrypt(&self, ticket: &[u8]) -> Option<Vec<u8>> {
        // Ticket must be at least: IV (16B) + HMAC (32B)
        if ticket.len() < AES_CBC_IV_LEN + 32 {
            return None;
        }

        // Split into components
        let iv = &ticket[..AES_CBC_IV_LEN];
        let hmac_received = &ticket[ticket.len() - 32..];
        let ciphertext = &ticket[AES_CBC_IV_LEN..ticket.len() - 32];

        // Verify HMAC-SHA256 (constant-time comparison)
        hmac::verify(&self.hmac_key, &ticket[..ticket.len() - 32], hmac_received).ok()?;

        // Decrypt AES-256-CBC
        let mut plaintext = ciphertext.to_vec();
        let mut iv_array = [0u8; AES_CBC_IV_LEN];
        iv_array.copy_from_slice(iv);
        let decrypt_ctx = aws_lc_rs::cipher::DecryptionContext::Iv128(FixedLength::from(iv_array));
        let decrypted = self.decrypt_key.decrypt(&mut plaintext, decrypt_ctx).ok()?;

        Some(decrypted.to_vec())
    }
}

/// State for the ticket key rotator.
struct TicketRotatorState {
    /// Current encryptor (for issuing new tickets)
    current: CustomTicketEncryptor,
    /// Previous encryptor (for decrypting old tickets)
    previous: Option<CustomTicketEncryptor>,
    /// When to perform the next rotation
    next_switch_time: Option<u64>,
}

/// Automatic ticket key rotator.
///
/// Implements the `ProducesTickets` trait with automatic time-based key rotation.
/// Follows the same pattern as rustls's internal `TicketRotator`, but with
/// file-backed key persistence for multi-instance support.
///
/// # Rotation Behavior
///
/// - Every `rotation_interval` seconds, a new key is generated
/// - The current key becomes the previous key (still valid for decryption)
/// - The new key becomes current (used for encryption)
/// - Old tickets remain valid for 2× rotation_interval
pub struct TicketKeyRotator {
    /// Current state
    state: RwLock<TicketRotatorState>,
    /// How often to rotate (in seconds)
    rotation_interval: Option<u32>,
    /// Path to the key file (for persistence)
    key_file: String,
}

impl TicketKeyRotator {
    /// Create a new ticket key rotator.
    ///
    /// # Arguments
    ///
    /// * `keys` - Initial keys to use (first key is for encryption, all for decryption)
    /// * `rotation_interval` - How often to rotate keys, set to `None` to disable rotation
    /// * `key_file` - Path to persist keys
    ///
    /// # Returns
    ///
    /// A new `TicketKeyRotator` or an error if keys are invalid.
    pub fn new(
        keys: Vec<TicketKey>,
        rotation_interval: Option<std::time::Duration>,
        key_file: String,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        if keys.is_empty() {
            return Err("At least one ticket key is required".into());
        }

        let lifetime = rotation_interval.map(|d| d.as_secs() as u32);

        // Create encryptor from the first key
        let current = CustomTicketEncryptor::new(&keys[0])?;

        // If we have multiple keys, create previous encryptor
        let previous = if keys.len() > 1 {
            Some(CustomTicketEncryptor::new(&keys[1])?)
        } else {
            None
        };

        let state = TicketRotatorState {
            current,
            previous,
            next_switch_time: lifetime.map(|l| UnixTime::now().as_secs().saturating_add(l as u64)),
        };

        Ok(Self {
            state: RwLock::new(state),
            rotation_interval: lifetime,
            key_file,
        })
    }

    /// Perform key rotation if necessary.
    ///
    /// This is called on every encrypt/decrypt operation but only performs
    /// rotation when the interval has elapsed.
    fn maybe_roll(
        &self,
        now: UnixTime,
    ) -> Option<std::sync::RwLockReadGuard<'_, TicketRotatorState>> {
        let now = now.as_secs();

        // Fast path: no rotation needed
        {
            let read = self.state.read().ok()?;
            if read.next_switch_time.is_none_or(|t| now <= t) {
                return Some(read);
            }
        }

        // Slow path: generate new key outside the lock
        let new_key = generate_ticket_key();
        let new_ticket_key = TicketKey {
            key_name: new_key[0..16].try_into().ok()?,
            aes_key: new_key[16..48].try_into().ok()?,
            hmac_key: new_key[48..80].try_into().ok()?,
        };

        // Create new encryptor
        let new_encryptor = CustomTicketEncryptor::new(&new_ticket_key).ok()?;

        // Persist the new key to file
        // Read existing keys, prepend new one, trim to reasonable count
        if let Ok(mut existing_keys) = load_ticket_keys(&self.key_file) {
            // Prepend new key
            existing_keys.insert(
                0,
                (
                    new_ticket_key.key_name,
                    new_ticket_key.aes_key,
                    new_ticket_key.hmac_key,
                ),
            );
            // Keep only first 3 keys
            if existing_keys.len() > 3 {
                existing_keys.truncate(3);
            }

            // Persist (ignore errors - don't fail rotation if file write fails)
            persist_ticket_keys(&self.key_file, &existing_keys).ok();
        }

        // Acquire write lock and perform rotation
        let mut write = self.state.write().ok()?;

        // Double-check time (another thread might have rotated)
        if write.next_switch_time.is_none_or(|t| now <= t) {
            drop(write);
            return self.state.read().ok();
        }

        // Rotate: current → previous, new → current
        write.previous = Some(std::mem::replace(&mut write.current, new_encryptor));
        write.next_switch_time = self
            .rotation_interval
            .map(|interval| now.saturating_add(interval as u64));

        ferron_core::log_info!("TLS session ticket keys rotated successfully");

        drop(write);
        self.state.read().ok()
    }
}

impl ProducesTickets for TicketKeyRotator {
    /// Returns true - this ticketer is always enabled.
    fn enabled(&self) -> bool {
        true
    }

    /// Returns the ticket lifetime in seconds (2× rotation interval).
    ///
    /// Tickets remain valid for 2 rotation intervals to allow the previous
    /// key to still decrypt tickets issued with the old current key.
    fn lifetime(&self) -> u32 {
        // If rotation is disabled, return a large lifetime to effectively never expire tickets
        self.rotation_interval.map_or(u32::MAX, |l| l * 2)
    }

    /// Encrypt plaintext into a ticket.
    ///
    /// This may trigger key rotation if the interval has elapsed.
    fn encrypt(&self, message: &[u8]) -> Option<Vec<u8>> {
        self.maybe_roll(UnixTime::now())?.current.encrypt(message)
    }

    /// Decrypt a ticket back into plaintext.
    ///
    /// Tries the current key first, then falls back to the previous key.
    /// This may trigger key rotation if the interval has elapsed.
    fn decrypt(&self, ciphertext: &[u8]) -> Option<Vec<u8>> {
        let state = self.maybe_roll(UnixTime::now())?;

        // Try current key first
        state.current.decrypt(ciphertext).or_else(|| {
            // Fall back to previous key
            state
                .previous
                .as_ref()
                .and_then(|previous| previous.decrypt(ciphertext))
        })
    }
}

impl fmt::Debug for TicketKeyRotator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TicketKeyRotator")
            .field("rotation_interval", &self.rotation_interval)
            .field("key_file", &self.key_file)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
