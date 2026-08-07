//! Password hash parsing and verification for HTTP Basic Authentication.
//!
//! Supported hash formats:
//!
//! - Argon2: `$argon2id$v=<v>$m=<m>,t=<t>,p=<p>$<salt_b64>$<hash_b64>`
//!   (also `$argon2i$` and `$argon2d$`)
//! - PBKDF2: `$pbkdf2[-sha256|-sha384|-sha512]?$<iterations>$<salt_b64>$<derived_key_b64>`
//! - PBKDF2 (legacy PHC parameter form):
//!   `$pbkdf2[-sha256|-sha512]?$i=<iterations>[,l=<length>]$<salt_b64>$<derived_key_b64>`
//! - scrypt: `$scrypt$ln=<N_log>,r=<r>,p=<p>$<salt_b64>$<hash_b64>`
//!
//! Base64 salt and hash fields accept both padded and unpadded encodings.

use std::num::NonZeroU32;
use std::str::FromStr;

use base64::engine::general_purpose::STANDARD_NO_PAD;
use base64::Engine;

/// The scrypt memory limit passed to AWS-LC. A value of `0` selects AWS-LC's
/// built-in default limit of 32 MiB, which prevents hostile hashes from
/// exhausting server memory during verification.
const SCRYPT_MAX_MEM: usize = 0;

/// Verify a plaintext password against a stored password hash.
///
/// Returns `false` for unrecognized hash formats and malformed hashes.
/// This function never panics.
pub(crate) fn verify_password(plain: &str, hash: &str) -> bool {
    if hash.starts_with("$argon2") {
        verify_argon2(plain, hash)
    } else if hash.starts_with("$pbkdf2") {
        verify_pbkdf2(plain, hash)
    } else if hash.starts_with("$scrypt$") {
        verify_scrypt(plain, hash)
    } else {
        false
    }
}

/// Verify a password against an Argon2 hash string.
fn verify_argon2(plain: &str, hash: &str) -> bool {
    // `argon2-kdf` decodes base64 without padding, so strip trailing `=`
    // padding from the salt and hash segments to accept both encodings.
    let normalized = match strip_base64_padding(hash) {
        Some(normalized) => normalized,
        None => return false,
    };
    match argon2_kdf::Hash::from_str(&normalized) {
        Ok(parsed) => parsed.verify(plain.as_bytes()),
        Err(_) => false,
    }
}

/// Strip trailing `=` padding from the salt and hash segments of an Argon2
/// hash string. Returns `None` if the string does not have the expected
/// structure.
fn strip_base64_padding(hash: &str) -> Option<String> {
    let segments: Vec<&str> = hash.split('$').collect();
    if segments.len() != 6 {
        return None;
    }
    let mut normalized = String::with_capacity(hash.len());
    for (index, segment) in segments.iter().enumerate() {
        if index > 0 {
            normalized.push('$');
        }
        if index == 4 || index == 5 {
            normalized.push_str(segment.trim_end_matches('='));
        } else {
            normalized.push_str(segment);
        }
    }
    Some(normalized)
}

/// Verify a password against a PBKDF2 hash string.
fn verify_pbkdf2(plain: &str, hash: &str) -> bool {
    let segments: Vec<&str> = hash.split('$').collect();
    if segments.len() != 5 || !segments[0].is_empty() {
        return false;
    }

    let algorithm = match pbkdf2_algorithm(segments[1]) {
        Some(algorithm) => algorithm,
        None => return false,
    };

    // The params segment is either a bare iteration count (e.g. `600000`)
    // or the legacy PHC parameter form (e.g. `i=600000,l=32`).
    let (iterations, expected_length) = match parse_pbkdf2_params(segments[2]) {
        Some(params) => params,
        None => return false,
    };

    let salt = match decode_b64_ignore_padding(segments[3]) {
        Some(salt) if !salt.is_empty() => salt,
        _ => return false,
    };
    let derived_key = match decode_b64_ignore_padding(segments[4]) {
        Some(key) if !key.is_empty() => key,
        _ => return false,
    };

    if expected_length.is_some_and(|len| len != derived_key.len() as u32) {
        return false;
    }

    aws_lc_rs::pbkdf2::verify(algorithm, iterations, &salt, plain.as_bytes(), &derived_key).is_ok()
}

/// Map a PBKDF2 hash identifier to an AWS-LC algorithm.
fn pbkdf2_algorithm(ident: &str) -> Option<aws_lc_rs::pbkdf2::Algorithm> {
    match ident {
        "pbkdf2" => Some(aws_lc_rs::pbkdf2::PBKDF2_HMAC_SHA1),
        "pbkdf2-sha256" => Some(aws_lc_rs::pbkdf2::PBKDF2_HMAC_SHA256),
        "pbkdf2-sha384" => Some(aws_lc_rs::pbkdf2::PBKDF2_HMAC_SHA384),
        "pbkdf2-sha512" => Some(aws_lc_rs::pbkdf2::PBKDF2_HMAC_SHA512),
        _ => None,
    }
}

/// Parse a PBKDF2 params segment.
///
/// Accepts a bare iteration count (`600000`) or the legacy PHC parameter
/// form (`i=600000[,l=32]`). Returns the iteration count and, when the `l`
/// parameter is present, the expected derived key length in bytes.
fn parse_pbkdf2_params(params: &str) -> Option<(NonZeroU32, Option<u32>)> {
    if let Ok(iterations) = params.parse::<u32>() {
        return NonZeroU32::new(iterations).map(|iterations| (iterations, None));
    }

    let mut iterations = None;
    let mut length = None;
    for pair in params.split(',') {
        let (key, value) = pair.split_once('=')?;
        match key {
            "i" => iterations = Some(value.parse::<u32>().ok()?),
            "l" => length = Some(value.parse::<u32>().ok()?),
            _ => return None,
        }
    }
    let iterations = NonZeroU32::new(iterations?)?;
    Some((iterations, length))
}

/// Verify a password against a scrypt hash string.
fn verify_scrypt(plain: &str, hash: &str) -> bool {
    let segments: Vec<&str> = hash.split('$').collect();
    if segments.len() != 5 || !segments[0].is_empty() || segments[1] != "scrypt" {
        return false;
    }

    let (n_log, r, p) = match parse_scrypt_params(segments[2]) {
        Some(params) => params,
        None => return false,
    };
    // AWS-LC requires 2 <= N <= 2^32 with N a power of two, so the log2
    // exponent must fit in 1..=32 to avoid an overflowing shift.
    if !(1..=32).contains(&n_log) || r == 0 || p == 0 {
        return false;
    }

    let salt = match decode_b64_ignore_padding(segments[3]) {
        Some(salt) if !salt.is_empty() => salt,
        _ => return false,
    };
    let expected = match decode_b64_ignore_padding(segments[4]) {
        Some(hash) if !hash.is_empty() => hash,
        _ => return false,
    };

    let mut derived = vec![0u8; expected.len()];
    let ret = unsafe {
        aws_lc_sys::EVP_PBE_scrypt(
            plain.as_ptr().cast(),
            plain.len(),
            salt.as_ptr(),
            salt.len(),
            1u64 << n_log,
            r,
            p,
            SCRYPT_MAX_MEM,
            derived.as_mut_ptr(),
            derived.len(),
        )
    };
    if ret != 1 {
        return false;
    }

    aws_lc_rs::constant_time::verify_slices_are_equal(&derived, &expected).is_ok()
}

/// Parse a scrypt params segment of the form `ln=<N_log>,r=<r>,p=<p>`.
fn parse_scrypt_params(params: &str) -> Option<(u64, u64, u64)> {
    let mut n_log = None;
    let mut r = None;
    let mut p = None;
    for pair in params.split(',') {
        let (key, value) = pair.split_once('=')?;
        match key {
            "ln" => n_log = Some(value.parse::<u64>().ok()?),
            "r" => r = Some(value.parse::<u64>().ok()?),
            "p" => p = Some(value.parse::<u64>().ok()?),
            _ => return None,
        }
    }
    Some((n_log?, r?, p?))
}

/// Decode a base64 value accepting both padded and unpadded encodings.
fn decode_b64_ignore_padding(input: &str) -> Option<Vec<u8>> {
    STANDARD_NO_PAD.decode(input.trim_end_matches('=')).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use base64::engine::general_purpose::STANDARD_NO_PAD as B64;

    const PASSWORD: &str = "test";

    const ARGON2ID_HASH_1: &str = "$argon2id$v=19$m=19456,t=2,p=1$\
        p4ZwOkPffNeVtgmOBgr/ZA$bPiMPdlq3NoWLe0ogU4XBTc/PjXAHAEDuYXSka2xKtU";
    const ARGON2ID_HASH_2: &str = "$argon2id$v=19$m=19456,t=2,p=1$\
        ZiPoEVmYo3b2r6Y2oZ8+JA$23gV15+t9eGAkldj1mkCEJXmwkxR9uoq65B4bG29I34";
    const SCRYPT_HASH: &str = "$scrypt$ln=14,r=8,p=1$\
        M1J2e6IxMyKOibSPlT0NKw==$K2jSRWWk89vtjk0207snEFx7Opbfi08uhqg8AZWIObw=";
    const PBKDF2_SHA256_HASH: &str = "$pbkdf2-sha256$600000$\
        q/OlsSBToMqk35bOAlik5w==$2hVHUFyEgG0urpqr2/JjQaMbLvlFUncpwoqRx0j1Kbk=";

    /// Re-encode the salt and hash segments (indices 4 and 5) with base64
    /// padding.
    fn with_padded_segments(hash: &str) -> String {
        let segments: Vec<&str> = hash.split('$').collect();
        let mut normalized = String::with_capacity(hash.len() + 4);
        for (index, segment) in segments.iter().enumerate() {
            if index > 0 {
                normalized.push('$');
            }
            if index == 4 || index == 5 {
                normalized.push_str(&STANDARD.encode(B64.decode(segment).unwrap()));
            } else {
                normalized.push_str(segment);
            }
        }
        normalized
    }

    /// Strip base64 padding from the salt and hash segments (indices 4 and 5).
    fn with_unpadded_segments(hash: &str) -> String {
        let segments: Vec<&str> = hash.split('$').collect();
        let mut normalized = String::with_capacity(hash.len());
        for (index, segment) in segments.iter().enumerate() {
            if index > 0 {
                normalized.push('$');
            }
            if index == 4 || index == 5 {
                normalized.push_str(segment.trim_end_matches('='));
            } else {
                normalized.push_str(segment);
            }
        }
        normalized
    }

    fn verify(plain: &str, hash: &str) -> bool {
        verify_password(plain, hash)
    }

    #[test]
    fn verifies_argon2id() {
        assert!(verify(PASSWORD, ARGON2ID_HASH_1));
        assert!(verify(PASSWORD, ARGON2ID_HASH_2));
        assert!(!verify("wrong", ARGON2ID_HASH_1));
    }

    #[test]
    fn verifies_argon2id_with_padded_base64() {
        let padded = with_padded_segments(ARGON2ID_HASH_1);
        assert!(padded.split('$').nth(4).unwrap().ends_with("=="));
        assert!(verify(PASSWORD, &padded));
    }

    #[test]
    fn verifies_scrypt() {
        assert!(verify(PASSWORD, SCRYPT_HASH));
        assert!(!verify("wrong", SCRYPT_HASH));
    }

    #[test]
    fn verifies_scrypt_with_unpadded_base64() {
        let unpadded = with_unpadded_segments(SCRYPT_HASH);
        assert!(!unpadded.split('$').nth(4).unwrap().contains('='));
        assert!(verify(PASSWORD, &unpadded));
    }

    #[test]
    fn verifies_pbkdf2_sha256() {
        assert!(verify(PASSWORD, PBKDF2_SHA256_HASH));
        assert!(!verify("wrong", PBKDF2_SHA256_HASH));
    }

    #[test]
    fn verifies_pbkdf2_sha256_with_unpadded_base64() {
        let unpadded = with_unpadded_segments(PBKDF2_SHA256_HASH);
        assert!(!unpadded.split('$').nth(4).unwrap().contains('='));
        assert!(verify(PASSWORD, &unpadded));
    }

    #[test]
    fn verifies_pbkdf2_legacy_phc_params() {
        let legacy = "$pbkdf2-sha256$i=600000,l=32$\
            q/OlsSBToMqk35bOAlik5w==$2hVHUFyEgG0urpqr2/JjQaMbLvlFUncpwoqRx0j1Kbk=";
        assert!(verify(PASSWORD, legacy));

        let no_length = "$pbkdf2-sha256$i=600000$\
            q/OlsSBToMqk35bOAlik5w==$2hVHUFyEgG0urpqr2/JjQaMbLvlFUncpwoqRx0j1Kbk=";
        assert!(verify(PASSWORD, no_length));
    }

    #[test]
    fn verifies_roundtripped_pbkdf2_variants() {
        let salt = b"salt for pbkdf2 roundtrip";
        for (ident, algorithm) in [
            ("pbkdf2", aws_lc_rs::pbkdf2::PBKDF2_HMAC_SHA1),
            ("pbkdf2-sha256", aws_lc_rs::pbkdf2::PBKDF2_HMAC_SHA256),
            ("pbkdf2-sha384", aws_lc_rs::pbkdf2::PBKDF2_HMAC_SHA384),
            ("pbkdf2-sha512", aws_lc_rs::pbkdf2::PBKDF2_HMAC_SHA512),
        ] {
            let mut derived = [0u8; 32];
            aws_lc_rs::pbkdf2::derive(
                algorithm,
                NonZeroU32::new(1000).unwrap(),
                salt,
                PASSWORD.as_bytes(),
                &mut derived,
            );
            let hash = format!("${ident}$1000${}${}", B64.encode(salt), B64.encode(derived));
            assert!(verify(PASSWORD, &hash), "failed for {ident}");
            assert!(!verify("wrong", &hash), "failed for {ident}");
        }
    }

    #[test]
    fn verifies_roundtripped_argon2_variants() {
        for algorithm in [
            argon2_kdf::Algorithm::Argon2id,
            argon2_kdf::Algorithm::Argon2i,
            argon2_kdf::Algorithm::Argon2d,
        ] {
            let hash = argon2_kdf::Hasher::new()
                .algorithm(algorithm)
                .iterations(2)
                .memory_cost_kib(19456)
                .hash(PASSWORD.as_bytes())
                .unwrap();
            let hash_string = hash.to_string();
            assert!(verify(PASSWORD, &hash_string));
            assert!(!verify("wrong", &hash_string));
        }
    }

    #[test]
    fn rejects_malformed_hashes() {
        let cases = [
            "plaintext",
            "$argon2id$v=19$m=19456,t=2,p=1",
            "$argon2id$v=19$m=19456,t=2,p=1$abc",
            "$argon2x$v=19$m=19456,t=2,p=1$abc$def",
            "$pbkdf2-sha256$600000$!!!$!!!",
            "$pbkdf2-sha256$0$c2FsdA$2hVHUFyEgG0urpqr2/JjQaMbLvlFUncpwoqRx0j1Kbk=",
            "$pbkdf2-sha256$99999999999$c2FsdA$2hVHUFyEgG0urpqr2/JjQaMbLvlFUncpwoqRx0j1Kbk=",
            "$pbkdf2-sha256$x=1$c2FsdA$2hVHUFyEgG0urpqr2/JjQaMbLvlFUncpwoqRx0j1Kbk=",
            "$pbkdf2-sha256$i=600000,l=16$\
                q/OlsSBToMqk35bOAlik5w==$2hVHUFyEgG0urpqr2/JjQaMbLvlFUncpwoqRx0j1Kbk=",
            "$pbkdf2-sha256$600000$$2hVHUFyEgG0urpqr2/JjQaMbLvlFUncpwoqRx0j1Kbk=",
            "$pbkdf2-sha256$600000$c2FsdA$",
            "$pbkdf2-md5$600000$c2FsdA$ZGVyaXZlZA==",
            "$scrypt$ln=0,r=8,p=1$M1J2e6IxMyKOibSPlT0NKw==$K2jSRWWk89vtjk0207snEFx7Opbfi08uhqg8AZWIObw=",
            "$scrypt$ln=100,r=8,p=1$M1J2e6IxMyKOibSPlT0NKw==$K2jSRWWk89vtjk0207snEFx7Opbfi08uhqg8AZWIObw=",
            "$scrypt$ln=14,r=0,p=1$M1J2e6IxMyKOibSPlT0NKw==$K2jSRWWk89vtjk0207snEFx7Opbfi08uhqg8AZWIObw=",
            "$scrypt$ln=14,r=8,p=0$M1J2e6IxMyKOibSPlT0NKw==$K2jSRWWk89vtjk0207snEFx7Opbfi08uhqg8AZWIObw=",
            "$scrypt$ln=14,r=8$M1J2e6IxMyKOibSPlT0NKw==$K2jSRWWk89vtjk0207snEFx7Opbfi08uhqg8AZWIObw=",
            "$scrypt$ln=14,r=8,p=1$$K2jSRWWk89vtjk0207snEFx7Opbfi08uhqg8AZWIObw=",
            "$scrypt$ln=14,r=8,p=1$M1J2e6IxMyKOibSPlT0NKw==$",
            "$scrypt$ln=14,r=8,p=1,extra=1$M1J2e6IxMyKOibSPlT0NKw==$K2jSRWWk89vtjk0207snEFx7Opbfi08uhqg8AZWIObw=",
        ];
        for hash in cases {
            assert!(!verify(PASSWORD, hash), "should reject {hash}");
        }
    }

    #[test]
    fn rejects_unknown_prefixes() {
        assert!(!verify(PASSWORD, ""));
        assert!(!verify(PASSWORD, "$unknown$v=1$m=1$c2FsdA$aGFzaA=="));
        assert!(!verify(PASSWORD, "{SHA}2jmj7l5rSw0yVb/vlWAYkK/YBwk="));
    }

    #[test]
    fn fake_hash_is_verifiable() {
        let fake = crate::stage::FAKE_HASH;
        // The stage relies on this hash to always fail for the probe
        // password without panicking.
        assert!(!verify("test", fake));
    }
}
