use super::*;
use std::io::Write;
use tempfile::NamedTempFile;

fn create_temp_key_file(data: &[u8]) -> NamedTempFile {
    let mut file = NamedTempFile::new().expect("Failed to create temp file");
    file.write_all(data).expect("Failed to write to temp file");
    file.flush().expect("Failed to flush temp file");
    file
}

fn generate_valid_key_record() -> [u8; TICKET_KEY_RECORD_SIZE] {
    let mut record = [0u8; TICKET_KEY_RECORD_SIZE];
    for (i, byte) in record.iter_mut().enumerate() {
        *byte = (i % 256) as u8;
    }
    record
}

#[test]
fn test_generate_ticket_key_size() {
    let key = generate_ticket_key();
    assert_eq!(key.len(), TICKET_KEY_RECORD_SIZE);
}

#[test]
fn test_generate_ticket_key_uniqueness() {
    let key1 = generate_ticket_key();
    let key2 = generate_ticket_key();
    assert_ne!(key1, key2);
}

#[test]
fn test_generate_ticket_key_randomness() {
    let key = generate_ticket_key();
    let zero_count = key.iter().filter(|&&b| b == 0).count();
    assert!(zero_count < 5, "Key appears to have insufficient entropy");
}

#[test]
fn test_generate_initial_ticket_keys_creates_file() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let key_file = temp_dir.path().join("session_tickets.keys");

    generate_initial_ticket_keys(key_file.to_str().unwrap(), 3).expect("Failed to generate keys");

    assert!(key_file.exists());
    let data = std::fs::read(&key_file).expect("Failed to read key file");
    assert_eq!(data.len(), 3 * TICKET_KEY_RECORD_SIZE);
}

#[test]
fn test_generate_initial_ticket_keys_single_key() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let key_file = temp_dir.path().join("single.keys");

    generate_initial_ticket_keys(key_file.to_str().unwrap(), 1).expect("Failed to generate keys");

    let data = std::fs::read(&key_file).expect("Failed to read key file");
    assert_eq!(data.len(), TICKET_KEY_RECORD_SIZE);
}

#[test]
fn test_generate_initial_ticket_keys_clamps_max() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let key_file = temp_dir.path().join("max.keys");

    generate_initial_ticket_keys(key_file.to_str().unwrap(), 10).expect("Failed to generate keys");

    let data = std::fs::read(&key_file).expect("Failed to read key file");
    assert_eq!(data.len(), 5 * TICKET_KEY_RECORD_SIZE);
}

#[test]
fn test_generate_initial_ticket_keys_clamps_min() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let key_file = temp_dir.path().join("min.keys");

    generate_initial_ticket_keys(key_file.to_str().unwrap(), 0).expect("Failed to generate keys");

    let data = std::fs::read(&key_file).expect("Failed to read key file");
    assert_eq!(data.len(), TICKET_KEY_RECORD_SIZE);
}

#[test]
fn test_generate_initial_ticket_keys_creates_parent_dirs() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let key_file = temp_dir
        .path()
        .join("subdir")
        .join("nested")
        .join("keys.keys");

    generate_initial_ticket_keys(key_file.to_str().unwrap(), 1).expect("Failed to generate keys");

    assert!(key_file.exists());
}

#[test]
fn test_persist_ticket_keys_roundtrip() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let key_file = temp_dir.path().join("persist.keys");

    let original_keys = vec![
        parse_ticket_key_record(&generate_ticket_key(), 0).unwrap(),
        parse_ticket_key_record(&generate_ticket_key(), 0).unwrap(),
    ];

    persist_ticket_keys(key_file.to_str().unwrap(), &original_keys)
        .expect("Failed to persist keys");

    let loaded_keys = load_ticket_keys(key_file.to_str().unwrap()).expect("Failed to load keys");

    assert_eq!(original_keys, loaded_keys);
}

#[test]
fn test_persist_ticket_keys_atomic_write() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let key_file = temp_dir.path().join("atomic.keys");

    let initial_key = generate_valid_key_record();
    std::fs::write(&key_file, initial_key).expect("Failed to create initial file");

    let new_keys = vec![
        parse_ticket_key_record(&generate_ticket_key(), 0).unwrap(),
        parse_ticket_key_record(&generate_ticket_key(), 0).unwrap(),
    ];

    persist_ticket_keys(key_file.to_str().unwrap(), &new_keys).expect("Failed to persist keys");

    let loaded_keys = load_ticket_keys(key_file.to_str().unwrap()).expect("Failed to load keys");

    assert_eq!(loaded_keys.len(), 2);
    assert_eq!(loaded_keys, new_keys);
}

#[test]
fn test_persist_ticket_keys_empty_fails() {
    let result = persist_ticket_keys("/tmp/test.keys", &[]);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
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
    let mut data = Vec::new();
    for _ in 0..5 {
        data.extend_from_slice(&generate_valid_key_record());
    }

    let file = create_temp_key_file(&data);

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
    record2[0] = 0xFF;

    let mut data = Vec::new();
    data.extend_from_slice(&record1);
    data.extend_from_slice(&record2);

    let file = create_temp_key_file(&data);

    let keys = load_ticket_keys(file.path().to_str().unwrap()).expect("Should load multiple keys");
    assert_eq!(keys.len(), 2);
}

#[test]
fn test_load_max_keys_respects_limit() {
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
    let data = [0u8; 40];
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
    let record = generate_valid_key_record();
    let file = create_temp_key_file(&record);

    let keys = load_ticket_keys(file.path().to_str().unwrap()).expect("Should load");
    assert_eq!(keys.len(), 1);
}

#[test]
fn test_key_content_not_logged() {
    let data = vec![0u8; 100];
    let file = create_temp_key_file(&data);

    let result = load_ticket_keys(file.path().to_str().unwrap());
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();

    assert!(!err_msg.contains("key_name"));
    assert!(!err_msg.contains("aes_key"));
    assert!(!err_msg.contains("hmac_key"));
}

fn create_test_ticket_key() -> TicketKey {
    let raw_key = generate_ticket_key();
    TicketKey {
        key_name: raw_key[0..16].try_into().unwrap(),
        aes_key: raw_key[16..48].try_into().unwrap(),
        hmac_key: raw_key[48..80].try_into().unwrap(),
    }
}

#[test]
fn test_encrypt_decrypt_roundtrip() {
    let key = create_test_ticket_key();
    let encryptor = CustomTicketEncryptor::new(&key).expect("Failed to create encryptor");

    let plaintext = b"test session data";
    let ticket = encryptor.encrypt(plaintext).expect("Failed to encrypt");

    assert!(ticket.len() > plaintext.len());

    let decrypted = encryptor.decrypt(&ticket).expect("Failed to decrypt");
    assert_eq!(decrypted, plaintext);
}

#[test]
fn test_decrypt_with_wrong_key_fails() {
    let key1 = create_test_ticket_key();
    let key2 = create_test_ticket_key();

    let encryptor1 = CustomTicketEncryptor::new(&key1).expect("Failed to create encryptor");
    let encryptor2 = CustomTicketEncryptor::new(&key2).expect("Failed to create encryptor");

    let plaintext = b"secret session data";
    let ticket = encryptor1.encrypt(plaintext).expect("Failed to encrypt");

    let result = encryptor2.decrypt(&ticket);
    assert!(result.is_none(), "Decryption with wrong key should fail");
}

#[test]
fn test_decrypt_too_short_ticket() {
    let key = create_test_ticket_key();
    let encryptor = CustomTicketEncryptor::new(&key).expect("Failed to create encryptor");

    let short_ticket = vec![0u8; 40];
    let result = encryptor.decrypt(&short_ticket);
    assert!(result.is_none(), "Decryption of short ticket should fail");
}

#[test]
fn test_decrypt_tampered_ticket() {
    let key = create_test_ticket_key();
    let encryptor = CustomTicketEncryptor::new(&key).expect("Failed to create encryptor");

    let plaintext = b"tamper test";
    let mut ticket = encryptor.encrypt(plaintext).expect("Failed to encrypt");
    ticket[20] ^= 0xFF;

    let result = encryptor.decrypt(&ticket);
    assert!(
        result.is_none(),
        "Decryption of tampered ticket should fail"
    );
}

#[test]
fn test_different_plaintexts() {
    let key = create_test_ticket_key();
    let encryptor = CustomTicketEncryptor::new(&key).expect("Failed to create encryptor");

    let test_cases = vec![
        vec![],
        vec![0x00],
        vec![0xFF; 16],
        vec![0xAB; 100],
        vec![0x12, 0x34, 0x56, 0x78],
    ];

    for plaintext in test_cases {
        let ticket = encryptor.encrypt(&plaintext).expect("Failed to encrypt");
        let decrypted = encryptor.decrypt(&ticket).expect("Failed to decrypt");
        assert_eq!(
            decrypted,
            plaintext,
            "Roundtrip failed for plaintext of length {}",
            plaintext.len()
        );
    }
}

#[test]
fn test_ticket_key_rotator_creation() {
    let keys = vec![create_test_ticket_key()];
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let key_file = temp_dir.path().join("rotator.keys");

    persist_ticket_keys(
        key_file.to_str().unwrap(),
        &[(keys[0].key_name, keys[0].aes_key, keys[0].hmac_key)],
    )
    .expect("Failed to persist keys");

    let rotator = TicketKeyRotator::new(keys, None, key_file.to_str().unwrap().to_string())
        .expect("Failed to create rotator");

    assert!(rotator.enabled());
}

#[test]
fn test_ticket_key_rotator_encrypt_decrypt() {
    let keys = vec![create_test_ticket_key()];
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let key_file = temp_dir.path().join("rotator2.keys");

    persist_ticket_keys(
        key_file.to_str().unwrap(),
        &[(keys[0].key_name, keys[0].aes_key, keys[0].hmac_key)],
    )
    .expect("Failed to persist keys");

    let rotator = TicketKeyRotator::new(keys.clone(), None, key_file.to_str().unwrap().to_string())
        .expect("Failed to create rotator");

    let plaintext = b"rotator test data";
    let ticket = <TicketKeyRotator as ProducesTickets>::encrypt(&rotator, plaintext)
        .expect("Failed to encrypt");

    let decrypted = <TicketKeyRotator as ProducesTickets>::decrypt(&rotator, &ticket)
        .expect("Failed to decrypt");

    assert_eq!(decrypted, plaintext);
}

#[test]
fn test_ticket_key_rotator_multiple_keys() {
    let keys = vec![create_test_ticket_key(), create_test_ticket_key()];
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let key_file = temp_dir.path().join("rotator3.keys");

    let key_components: Vec<_> = keys
        .iter()
        .map(|k| (k.key_name, k.aes_key, k.hmac_key))
        .collect();
    persist_ticket_keys(key_file.to_str().unwrap(), &key_components)
        .expect("Failed to persist keys");

    let rotator = TicketKeyRotator::new(keys.clone(), None, key_file.to_str().unwrap().to_string())
        .expect("Failed to create rotator");

    let plaintext = b"multi-key test";
    let ticket = <TicketKeyRotator as ProducesTickets>::encrypt(&rotator, plaintext)
        .expect("Failed to encrypt");

    let decrypted = <TicketKeyRotator as ProducesTickets>::decrypt(&rotator, &ticket)
        .expect("Failed to decrypt");
    assert_eq!(decrypted, plaintext);
}

#[test]
fn test_ticket_key_rotator_rotates() {
    let keys = vec![create_test_ticket_key()];
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let key_file = temp_dir.path().join("rotator4.keys");

    persist_ticket_keys(
        key_file.to_str().unwrap(),
        &[(keys[0].key_name, keys[0].aes_key, keys[0].hmac_key)],
    )
    .expect("Failed to persist keys");

    let rotator = TicketKeyRotator::new(
        keys.clone(),
        Some(Duration::from_secs(1)),
        key_file.to_str().unwrap().to_string(),
    )
    .expect("Failed to create rotator");

    let plaintext = b"rotate test";
    let ticket_key_contents_before = std::fs::read(&key_file).expect("Failed to read key file");
    let ticket = <TicketKeyRotator as ProducesTickets>::encrypt(&rotator, plaintext)
        .expect("Failed to encrypt");
    let ticket_key_contents = std::fs::read(&key_file).expect("Failed to read key file");
    assert_eq!(
        ticket_key_contents, ticket_key_contents_before,
        "Key file should not change before rotation"
    );

    std::thread::sleep(std::time::Duration::from_secs(2));

    let _ticket = <TicketKeyRotator as ProducesTickets>::encrypt(&rotator, plaintext)
        .expect("Failed to encrypt");
    let ticket_key_contents_after = std::fs::read(&key_file).expect("Failed to read key file");
    assert_ne!(
        ticket_key_contents_after, ticket_key_contents_before,
        "Key file should change after rotation"
    );

    let decrypted = <TicketKeyRotator as ProducesTickets>::decrypt(&rotator, &ticket)
        .expect("Failed to decrypt after rotation");
    assert_eq!(decrypted, plaintext);
}

#[test]
fn test_ticket_key_rotator_empty_keys_fails() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let key_file = temp_dir.path().join("empty.keys");

    let result = TicketKeyRotator::new(vec![], None, key_file.to_str().unwrap().to_string());

    assert!(result.is_err());
}
