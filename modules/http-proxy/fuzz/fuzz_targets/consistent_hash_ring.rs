#![no_main]

use std::sync::Arc;

use ferron_http_proxy::types::upstream::UpstreamInner;
use ferron_http_proxy::upstream::lb::ConsistentHashRing;
use libfuzzer_sys::fuzz_target;

/// Parse backends and keys from raw bytes.
///
/// Format:
///   [backend_count: u16 LE]
///     for each backend:
///       [weight: u32 LE]
///       [name_len: u16 LE]
///       [name: name_len bytes (UTF-8)]
///   [key_count: u16 LE]
///     for each key:
///       [key_len: u16 LE]
///       [key: key_len bytes]
fn parse_backends_and_keys(input: &[u8]) -> Option<(Vec<Arc<UpstreamInner>>, Vec<Vec<u8>>)> {
    let mut pos = 0;

    if pos + 2 > input.len() {
        return None;
    }
    let backend_count = u16::from_le_bytes([input[pos], input[pos + 1]]) as usize;
    pos += 2;

    let max_backends = 50;
    if backend_count > max_backends {
        return None;
    }

    let mut backends = Vec::with_capacity(backend_count);
    for _ in 0..backend_count {
        if pos + 4 > input.len() {
            return None;
        }
        let weight =
            u32::from_le_bytes([input[pos], input[pos + 1], input[pos + 2], input[pos + 3]]);
        pos += 4;

        if pos + 2 > input.len() {
            return None;
        }
        let name_len = u16::from_le_bytes([input[pos], input[pos + 1]]) as usize;
        pos += 2;

        if pos + name_len > input.len() || name_len > 256 {
            return None;
        }
        let name_bytes = &input[pos..pos + name_len];
        pos += name_len;

        let proxy_to = String::from_utf8(name_bytes.to_vec()).ok()?;
        backends.push(Arc::new(UpstreamInner {
            proxy_to,
            proxy_unix: None,
            weight,
        }));
    }

    if pos + 2 > input.len() {
        return None;
    }
    let key_count = u16::from_le_bytes([input[pos], input[pos + 1]]) as usize;
    pos += 2;

    let max_keys = 200;
    if key_count > max_keys {
        return None;
    }

    let mut keys = Vec::with_capacity(key_count);
    for _ in 0..key_count {
        if pos + 2 > input.len() {
            return None;
        }
        let key_len = u16::from_le_bytes([input[pos], input[pos + 1]]) as usize;
        pos += 2;

        if pos + key_len > input.len() || key_len > 512 {
            return None;
        }
        keys.push(input[pos..pos + key_len].to_vec());
        pos += key_len;
    }

    Some((backends, keys))
}

fuzz_target!(|input: &[u8]| {
    let Some((backends, keys)) = parse_backends_and_keys(input) else {
        return;
    };

    // Build ring
    let ring = ConsistentHashRing::new(&backends);

    // Invariant 1: Empty ring returns None for any key
    if backends.is_empty() {
        for key in &keys {
            assert!(
                ring.get(key, &Default::default()).is_none(),
                "empty ring must return None"
            );
        }
        return;
    }

    // Invariant 2: get() always returns a valid index
    for key in &keys {
        if let Some(idx) = ring.get(key, &Default::default()) {
            assert!(
                idx < backends.len(),
                "ring.get({:?}) returned index {idx} >= {} backends",
                key,
                backends.len()
            );
        }
    }

    // Invariant 3: Deterministic — same key returns same result
    for key in &keys {
        if let (Some(a), Some(b)) = (
            ring.get(key, &Default::default()),
            ring.get(key, &Default::default()),
        ) {
            assert_eq!(a, b, "ring.get() must be deterministic for the same key");
        }
    }

    // Invariant 4: needs_rebuild returns true when backends change
    let rebuild_needed = ring.needs_rebuild(&backends);
    assert!(
        !rebuild_needed,
        "needs_rebuild must be false for same backends"
    );

    // Invariant 5: rebuild doesn't change get() results for same backends
    if !backends.is_empty() {
        let mut cloned_ring = ring.clone();
        cloned_ring.rebuild(&backends);
        for key in &keys {
            let orig = ring.get(key, &Default::default());
            let after = cloned_ring.get(key, &Default::default());
            assert_eq!(
                orig, after,
                "rebuild with same backends must not change get() results"
            );
        }
    }

    // Invariant 6: Total allocated nodes must not exceed bounded maximum
    // With MAX_EFFECTIVE_WEIGHT = 100 and VNODES_PER_BACKEND = 160,
    // each backend contributes at most 16,000 vnodes.
    // With max 50 backends, total ≤ 800,000.
    let max_expected = backends.len().saturating_mul(100).saturating_mul(160);
    assert!(
        ring.len() <= max_expected,
        "ring node count {} exceeds max expected {}",
        ring.len(),
        max_expected
    );
});
