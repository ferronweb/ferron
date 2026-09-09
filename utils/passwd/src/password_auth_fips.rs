use std::num::NonZeroU32;

use base64::{engine::general_purpose::STANDARD_NO_PAD as B64, Engine};

pub fn generate_hash(password: impl AsRef<[u8]>) -> String {
    let mut salt: [u8; 16] = Default::default();
    getrandom::fill(&mut salt).expect("failed to fill salt with random bytes");
    let mut derived = [0u8; 32];
    let iterations = 600000; // OWASP Password Storage Cheat Sheet recommendation

    aws_lc_rs::pbkdf2::derive(
        aws_lc_rs::pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(iterations).expect("zero pbkdf2 iterations"),
        &salt,
        password.as_ref(),
        &mut derived,
    );

    let hash = format!(
        "$pbkdf2-sha256${iterations}${}${}",
        B64.encode(salt),
        B64.encode(derived)
    );
    hash
}
