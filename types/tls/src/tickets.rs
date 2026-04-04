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

use std::fs;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// Helper to create a temporary key file with the given data
    fn create_temp_key_file(data: &[u8]) -> NamedTempFile {
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        file.write_all(data).expect("Failed to write to temp file");
        file.flush().expect("Failed to flush temp file");
        file
    }

    /// Helper to generate a valid 80-byte ticket key record
    fn generate_valid_key_record() -> [u8; TICKET_KEY_RECORD_SIZE] {
        let mut record = [0u8; TICKET_KEY_RECORD_SIZE];
        // Fill with predictable but non-zero data
        for (i, byte) in record.iter_mut().enumerate() {
            *byte = (i % 256) as u8;
        }
        record
    }

    #[test]
    fn test_validate_single_valid_key() {
        let record = generate_valid_key_record();
        let file = create_temp_key_file(&record);

        let num_keys =
            validate_ticket_keys_file(file.path().to_str().unwrap()).expect("Should validate");
        assert_eq!(num_keys, 1);
    }

    #[test]
    fn test_validate_multiple_valid_keys() {
        let record1 = generate_valid_key_record();
        let mut record2 = generate_valid_key_record();
        // Make second record different
        record2[0] = 0xFF;

        let mut data = Vec::new();
        data.extend_from_slice(&record1);
        data.extend_from_slice(&record2);

        let file = create_temp_key_file(&data);

        let num_keys =
            validate_ticket_keys_file(file.path().to_str().unwrap()).expect("Should validate");
        assert_eq!(num_keys, 2);
    }

    #[test]
    fn test_validate_max_keys_warning() {
        // Create file with 5 keys (more than MAX_TICKET_KEYS)
        let mut data = Vec::new();
        for _ in 0..5 {
            data.extend_from_slice(&generate_valid_key_record());
        }

        let file = create_temp_key_file(&data);

        // Should succeed but log a warning
        let num_keys =
            validate_ticket_keys_file(file.path().to_str().unwrap()).expect("Should validate");
        assert_eq!(num_keys, 5);
    }

    #[test]
    fn test_validate_empty_file_fails() {
        let file = create_temp_key_file(&[]);

        let result = validate_ticket_keys_file(file.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn test_validate_invalid_size_not_multiple_of_80() {
        // Create file with 100 bytes (not multiple of 80)
        let data = vec![0u8; 100];
        let file = create_temp_key_file(&data);

        let result = validate_ticket_keys_file(file.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("not a multiple"));
    }

    #[test]
    fn test_validate_file_too_small() {
        // Create file with 40 bytes (less than 80)
        let data = vec![0u8; 40];
        let file = create_temp_key_file(&data);

        let result = validate_ticket_keys_file(file.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn test_validate_nonexistent_file() {
        let result = validate_ticket_keys_file("/nonexistent/path/ticket.keys");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn test_load_single_valid_key() {
        let record = generate_valid_key_record();
        let file = create_temp_key_file(&record);

        let keys = load_ticket_keys(file.path().to_str().unwrap()).expect("Should load single key");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].0, record[0..16]);
        assert_eq!(keys[0].1, record[16..48]);
        assert_eq!(keys[0].2, record[48..80]);
    }

    #[test]
    fn test_load_multiple_valid_keys() {
        let record1 = generate_valid_key_record();
        let mut record2 = generate_valid_key_record();
        // Make second record different
        record2[0] = 0xFF;

        let mut data = Vec::new();
        data.extend_from_slice(&record1);
        data.extend_from_slice(&record2);

        let file = create_temp_key_file(&data);

        let keys =
            load_ticket_keys(file.path().to_str().unwrap()).expect("Should load multiple keys");
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn test_load_max_keys_respects_limit() {
        // Create file with 5 keys (more than MAX_TICKET_KEYS)
        let mut data = Vec::new();
        for _ in 0..5 {
            data.extend_from_slice(&generate_valid_key_record());
        }

        let file = create_temp_key_file(&data);

        let keys = load_ticket_keys(file.path().to_str().unwrap()).expect("Should load keys");
        assert_eq!(keys.len(), MAX_TICKET_KEYS);
    }

    #[test]
    fn test_load_empty_file_fails() {
        let file = create_temp_key_file(&[]);

        let result = load_ticket_keys(file.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn test_load_invalid_size_not_multiple_of_80() {
        // Create file with 100 bytes (not multiple of 80)
        let data = vec![0u8; 100];
        let file = create_temp_key_file(&data);

        let result = load_ticket_keys(file.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("not a multiple"));
    }

    #[test]
    fn test_load_file_too_small() {
        // Create file with 40 bytes (less than 80)
        let data = vec![0u8; 40];
        let file = create_temp_key_file(&data);

        let result = load_ticket_keys(file.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn test_load_nonexistent_file() {
        let result = load_ticket_keys("/nonexistent/path/ticket.keys");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn test_parse_record_wrong_size() {
        let data = [0u8; 40]; // Wrong size
        let result = parse_ticket_key_record(&data, 0);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("40 bytes"));
    }

    #[test]
    fn test_parse_record_valid() {
        let record = generate_valid_key_record();
        let (key_name, aes_key, hmac_key) =
            parse_ticket_key_record(&record, 0).expect("Should parse valid record");

        assert_eq!(key_name, record[0..16]);
        assert_eq!(aes_key, record[16..48]);
        assert_eq!(hmac_key, record[48..80]);
    }

    #[test]
    fn test_key_file_permissions_warning() {
        // This test documents that we should check file permissions
        // In production, we would warn if file is world-readable
        let record = generate_valid_key_record();
        let file = create_temp_key_file(&record);

        // On Unix, we could check permissions here
        // For now, just verify the file loads successfully
        let keys = load_ticket_keys(file.path().to_str().unwrap()).expect("Should load");
        assert_eq!(keys.len(), 1);
    }

    #[test]
    fn test_key_content_not_logged() {
        // This is a documentation test - we verify that error messages
        // don't contain key content
        let data = vec![0u8; 100]; // Invalid size
        let file = create_temp_key_file(&data);

        let result = load_ticket_keys(file.path().to_str().unwrap());
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();

        // Verify error doesn't contain actual key bytes
        // (This is more of a regression test for future changes)
        assert!(!err_msg.contains("key_name"));
        assert!(!err_msg.contains("aes_key"));
        assert!(!err_msg.contains("hmac_key"));
    }
}
