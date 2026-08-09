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

mod argon2_sys;

use std::num::NonZeroU32;

use base64::engine::general_purpose::STANDARD_NO_PAD;
use base64::Engine;

/// The scrypt memory limit passed to AWS-LC. A value of `0` selects AWS-LC's
/// built-in default limit of 32 MiB, which prevents hostile hashes from
/// exhausting server memory during verification.
const SCRYPT_MAX_MEM: usize = 0;

/// Upper bounds for Argon2 cost parameters accepted during verification.
/// These prevent a hostile hash from exhausting server memory or CPU.
const MAX_MEMORY_COST_KIB: u32 = 4 * 1024 * 1024;
const MAX_ITERATIONS: u32 = 32_768;
const MAX_THREADS: u32 = 512;
const MAX_SALT_LEN: usize = 1024;
const MAX_HASH_LEN: usize = 4096;

/// Verify a plaintext password against a stored password hash.
///
/// Returns `false` for unrecognized hash formats and malformed hashes.
/// This function never panics.
#[inline]
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
#[inline]
fn verify_argon2(plain: &str, hash: &str) -> bool {
    let segments: Vec<&str> = hash.split('$').collect();
    // `$argon2id$v=<v>$m=<m>,t=<t>,p=<p>$<salt_b64>$<hash_b64>`
    if segments.len() != 6 || !segments[0].is_empty() {
        return false;
    }

    let algorithm = match segments[1] {
        "argon2id" => self::argon2_sys::Argon2_type_Argon2_id,
        "argon2i" => self::argon2_sys::Argon2_type_Argon2_i,
        "argon2d" => self::argon2_sys::Argon2_type_Argon2_d,
        _ => return false,
    };
    let version = match parse_argon2_version(segments[2]) {
        Some(version) => version,
        None => return false,
    };
    let (m_cost, t_cost, p_cost) = match parse_argon2_params(segments[3]) {
        Some(params) => params,
        None => return false,
    };
    // Bound the cost parameters so a hostile hash cannot exhaust server
    // memory or CPU during verification.
    if m_cost > MAX_MEMORY_COST_KIB || t_cost > MAX_ITERATIONS || p_cost > MAX_THREADS {
        return false;
    }

    let mut salt = match decode_b64_ignore_padding(segments[4]) {
        Some(salt) if !salt.is_empty() && salt.len() <= MAX_SALT_LEN => salt,
        _ => return false,
    };
    let expected = match decode_b64_ignore_padding(segments[5]) {
        Some(hash) if !hash.is_empty() && hash.len() <= MAX_HASH_LEN => hash,
        _ => return false,
    };

    let mut derived = vec![0u8; expected.len()];
    let plain_bytes = plain.as_bytes();
    let mut ctx = self::argon2_sys::Argon2_Context {
        out: derived.as_mut_ptr(),
        outlen: derived.len() as u32,
        salt: salt.as_mut_ptr(),
        saltlen: salt.len() as u32,
        pwd: plain_bytes.as_ptr() as *mut u8,
        pwdlen: plain_bytes.len() as u32,
        secret: std::ptr::null_mut(),
        secretlen: 0,
        ad: std::ptr::null_mut(),
        adlen: 0,
        t_cost,
        m_cost,
        lanes: p_cost,
        threads: p_cost,
        version,
        allocate_cbk: None,
        free_cbk: None,
        flags: 0, // ARGON2_DEFAULT_FLAGS
    };
    // SAFETY: `ctx` is initialized with valid pointers and parameters.
    // Also, `rc` will be set to `0` on success, so we check it for equality with `0`.
    // See https://github.com/P-H-C/phc-winner-argon2 README
    let rc = unsafe { self::argon2_sys::argon2_ctx(&mut ctx, algorithm) };
    if rc != 0 {
        return false;
    }

    aws_lc_rs::constant_time::verify_slices_are_equal(&derived, &expected).is_ok()
}

/// Parse an Argon2 version segment of the form `v=<version>`.
#[inline]
fn parse_argon2_version(segment: &str) -> Option<u32> {
    let value = segment.strip_prefix("v=")?;
    match value.parse::<u32>().ok()? {
        0x10 => Some(self::argon2_sys::Argon2_version_ARGON2_VERSION_10),
        0x13 => Some(self::argon2_sys::Argon2_version_ARGON2_VERSION_13),
        _ => None,
    }
}

/// Parse an Argon2 params segment of the form `m=<m>,t=<t>,p=<p>`.
#[inline]
fn parse_argon2_params(params: &str) -> Option<(u32, u32, u32)> {
    let mut m_cost = None;
    let mut t_cost = None;
    let mut p_cost = None;
    for pair in params.split(',') {
        let (key, value) = pair.split_once('=')?;
        match key {
            "m" => m_cost = Some(value.parse::<u32>().ok()?),
            "t" => t_cost = Some(value.parse::<u32>().ok()?),
            "p" => p_cost = Some(value.parse::<u32>().ok()?),
            _ => return None,
        }
    }
    Some((m_cost?, t_cost?, p_cost?))
}

/// Verify a password against a PBKDF2 hash string.
#[inline]
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
#[inline]
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
#[inline]
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
#[inline]
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
    // Had to use `aws-lc-sys` directly here, because `aws-lc-rs` doesn't expose
    // scrypt hashing functions
    //
    // SAFETY: `plain`, `salt`, and `derived` are all valid UTF-8 strings,
    // and `derived` is large enough to hold the output hash.
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
#[inline]
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
#[inline]
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
    #[inline]
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
    #[inline]
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

    #[inline]
    fn verify(plain: &str, hash: &str) -> bool {
        verify_password(plain, hash)
    }

    #[test]
    #[inline]
    fn verifies_argon2id() {
        assert!(verify(PASSWORD, ARGON2ID_HASH_1));
        assert!(verify(PASSWORD, ARGON2ID_HASH_2));
        assert!(!verify("wrong", ARGON2ID_HASH_1));
    }

    #[test]
    #[inline]
    fn verifies_argon2id_with_padded_base64() {
        let padded = with_padded_segments(ARGON2ID_HASH_1);
        assert!(padded.split('$').nth(4).unwrap().ends_with("=="));
        assert!(verify(PASSWORD, &padded));
    }

    #[test]
    #[inline]
    fn verifies_scrypt() {
        assert!(verify(PASSWORD, SCRYPT_HASH));
        assert!(!verify("wrong", SCRYPT_HASH));
    }

    #[test]
    #[inline]
    fn verifies_scrypt_with_unpadded_base64() {
        let unpadded = with_unpadded_segments(SCRYPT_HASH);
        assert!(!unpadded.split('$').nth(4).unwrap().contains('='));
        assert!(verify(PASSWORD, &unpadded));
    }

    #[test]
    #[inline]
    fn verifies_pbkdf2_sha256() {
        assert!(verify(PASSWORD, PBKDF2_SHA256_HASH));
        assert!(!verify("wrong", PBKDF2_SHA256_HASH));
    }

    #[test]
    #[inline]
    fn verifies_pbkdf2_sha256_with_unpadded_base64() {
        let unpadded = with_unpadded_segments(PBKDF2_SHA256_HASH);
        assert!(!unpadded.split('$').nth(4).unwrap().contains('='));
        assert!(verify(PASSWORD, &unpadded));
    }

    #[test]
    #[inline]
    fn verifies_pbkdf2_legacy_phc_params() {
        let legacy = "$pbkdf2-sha256$i=600000,l=32$\
            q/OlsSBToMqk35bOAlik5w==$2hVHUFyEgG0urpqr2/JjQaMbLvlFUncpwoqRx0j1Kbk=";
        assert!(verify(PASSWORD, legacy));

        let no_length = "$pbkdf2-sha256$i=600000$\
            q/OlsSBToMqk35bOAlik5w==$2hVHUFyEgG0urpqr2/JjQaMbLvlFUncpwoqRx0j1Kbk=";
        assert!(verify(PASSWORD, no_length));
    }

    #[test]
    #[inline]
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

    /// Encode an Argon2 hash as a PHC string.
    #[inline]
    fn argon2_phc_string(
        algorithm: argon2::Algorithm,
        version: argon2::Version,
        m_cost: u32,
        t_cost: u32,
        p_cost: u32,
        salt: &[u8],
        hash: &[u8],
    ) -> String {
        let ident = match algorithm {
            argon2::Algorithm::Argon2id => "argon2id",
            argon2::Algorithm::Argon2i => "argon2i",
            argon2::Algorithm::Argon2d => "argon2d",
        };
        let version = match version {
            argon2::Version::V0x10 => 16,
            argon2::Version::V0x13 => 19,
        };
        format!(
            "${ident}$v={version}$m={m_cost},t={t_cost},p={p_cost}${}${}",
            B64.encode(salt),
            B64.encode(hash)
        )
    }

    #[test]
    #[inline]
    fn verifies_roundtripped_argon2_variants() {
        let salt = b"argon2 roundtrip salt";
        let version = argon2::Version::default();
        for (algorithm, ident) in [
            (argon2::Algorithm::Argon2id, "argon2id"),
            (argon2::Algorithm::Argon2i, "argon2i"),
            (argon2::Algorithm::Argon2d, "argon2d"),
        ] {
            let argon2 = argon2::Argon2::new(
                algorithm,
                version,
                argon2::Params::new(19_456, 2, 1, Some(64)).unwrap(),
            );
            let mut derived = vec![0u8; 64];
            argon2
                .hash_password_into(PASSWORD.as_bytes(), salt, &mut derived)
                .unwrap();
            let hash = argon2_phc_string(algorithm, version, 19_456, 2, 1, salt, &derived);
            assert!(verify(PASSWORD, &hash), "failed for {ident}");
            assert!(!verify("wrong", &hash), "failed for {ident}");
        }
    }

    #[test]
    #[inline]
    fn verifies_roundtripped_argon2_v16() {
        let salt = b"argon2 v16 roundtrip";
        let algorithm = argon2::Algorithm::Argon2id;
        let version = argon2::Version::V0x10;
        let argon2 = argon2::Argon2::new(
            algorithm,
            version,
            argon2::Params::new(19_456, 2, 1, Some(64)).unwrap(),
        );
        let mut derived = vec![0u8; 64];
        argon2
            .hash_password_into(PASSWORD.as_bytes(), salt, &mut derived)
            .unwrap();
        let hash = argon2_phc_string(algorithm, version, 19_456, 2, 1, salt, &derived);
        assert!(verify(PASSWORD, &hash));
        assert!(!verify("wrong", &hash));
    }

    #[test]
    #[inline]
    fn rejects_malformed_hashes() {
        let cases = [
            "plaintext",
            "$argon2id$v=19$m=19456,t=2,p=1",
            "$argon2id$v=19$m=19456,t=2,p=1$abc",
            "$argon2x$v=19$m=19456,t=2,p=1$abc$def",
            "$argon2id$v=18$m=19456,t=2,p=1$c2FsdA$aGFzaA==",
            "$argon2id$v=19$m=19456,t=2$c2FsdA$aGFzaA==",
            "$argon2id$v=19$m=19456,t=2,p=1$x$aGFzaA==",
            "$argon2id$v=19$m=99999999999,t=2,p=1$c2FsdA$aGFzaA==",
            "$argon2id$v=19$m=8388608,t=99999999,p=1$c2FsdA$aGFzaA==",
            "$argon2id$v=19$m=8388608,t=2,p=999$c2FsdA$aGFzaA==",
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
    #[inline]
    fn rejects_unknown_prefixes() {
        assert!(!verify(PASSWORD, ""));
        assert!(!verify(PASSWORD, "$unknown$v=1$m=1$c2FsdA$aGFzaA=="));
        assert!(!verify(PASSWORD, "{SHA}2jmj7l5rSw0yVb/vlWAYkK/YBwk="));
    }

    #[test]
    #[inline]
    fn fake_hash_is_verifiable() {
        let fake = crate::stage::FAKE_HASH;
        // The stage relies on this hash to always fail for the probe
        // password without panicking.
        assert!(!verify("test", fake));
    }
}
