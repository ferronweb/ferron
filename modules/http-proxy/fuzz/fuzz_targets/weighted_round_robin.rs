#![no_main]

use ferron_http_proxy::upstream::lb::WeightedRoundRobinState;
use libfuzzer_sys::fuzz_target;

/// Parse a weight vector from raw bytes.
///
/// Format:
///   [count: u16 LE]
///   [weight: u32 LE; count]
fn parse_weights(input: &[u8]) -> Option<Vec<u32>> {
    if input.len() < 2 {
        return None;
    }
    let count = u16::from_le_bytes([input[0], input[1]]) as usize;

    // Limit to prevent excessive iteration
    if count > 1000 {
        return None;
    }

    let expected_size = 2 + count * 4;
    if input.len() < expected_size {
        return None;
    }

    let weights: Vec<u32> = input[2..expected_size]
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    Some(weights)
}

fuzz_target!(|input: &[u8]| {
    let Some(weights) = parse_weights(input) else {
        return;
    };

    let state = WeightedRoundRobinState::new();

    // Invariant 1: next() never panics and returns a valid index
    for _ in 0..20.min(weights.len().saturating_mul(3)) {
        let idx = state.next(&weights);
        if weights.is_empty() {
            assert_eq!(idx, 0, "empty weights must return index 0");
        } else {
            assert!(
                idx < weights.len(),
                "next() returned index {idx} >= {}",
                weights.len()
            );
        }
    }

    // Invariant 2: With equal weights, distribution is uniform (over 1 cycle)
    if weights.len() >= 2 && weights.iter().all(|&w| w == weights[0]) && weights[0] > 0 {
        let n = weights.len();
        let mut counts = vec![0usize; n];
        for _ in 0..n {
            let idx = state.next(&weights);
            counts[idx] += 1;
        }
        // Each backend should have been selected exactly once
        for (i, &c) in counts.iter().enumerate() {
            assert_eq!(
                c, 1,
                "equal-weight round-robin: backend {i} selected {c} times, expected 1"
            );
        }
    }

    // Invariant 3: Resizing the state works without panic
    let state2 = WeightedRoundRobinState::new();
    let short = vec![1u32, 2u32];
    let _ = state2.next(&short);
    let long = vec![1u32, 2u32, 3u32, 4u32, 5u32];
    let _ = state2.next(&long);
    let _ = state2.next(&short);
});
