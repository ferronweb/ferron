//! Fuzz target for the Base2 exponential histogram accumulator.
//!
//! Records arbitrary values (raw bit patterns, huge magnitudes, fractional
//! values, NaN and infinities) into an [`ExpoHistogram`] and checks the
//! exporter-side invariants of the resulting data point:
//!
//! - count == zero_count + positive bucket counts + negative bucket counts;
//! - min <= max, and the mean (sum / count) falls inside [min, max];
//! - scale stays within the configured bounds.
//!
//! NaN and infinite inputs are passed through to [`ExpoHistogram::record`],
//! which drops them: they are never counted, so the invariants above hold
//! regardless of how the fuzzer mixes them in.

#![no_main]

use ferron_observability_otlp::pipeline::metrics::histogram::{
    ExpoHistogram, EXPO_MAX_SCALE, EXPO_MIN_SCALE,
};
use libfuzzer_sys::fuzz_target;

/// SplitMix64 step so the value stream is a deterministic function of the
/// fuzzer input.
fn splitmix64(state: &mut u64) -> u64 {
    const PHI: u64 = 0x9E37_79B9_7F4A_7C15;
    *state = state.wrapping_add(PHI);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn seed_from(data: &[u8]) -> u64 {
    let mut seed = 0x853C_49E6_748F_EA9B;
    for (i, byte) in data.iter().enumerate() {
        seed ^= (*byte as u64).rotate_left(i as u32);
    }
    seed
}

/// Derive one measurement from the PRNG state.
fn next_value(state: &mut u64) -> f64 {
    let raw = splitmix64(state);
    match raw & 3 {
        // Arbitrary bit pattern: NaN / infinities / subnormals included.
        0 => f64::from_bits(raw),
        // Huge integer scale.
        1 => (raw as i64) as f64,
        // Fractional magnitudes: exponent range [-32, 32].
        2 => splitmix64(state) as f64 / 2f64.powi(((raw >> 8) & 0x3F) as i32 - 32),
        // Signed huge values from two draws.
        _ => (raw as i64 as f64) * (splitmix64(state) as i64 as f64),
    }
}

fuzz_target!(|data: &[u8]| {
    let mut state = seed_from(data);

    let mut hist = ExpoHistogram::new();
    let mut zeros = 0u64;
    let calls = 1 + (state % 256) as usize;
    for _ in 0..calls {
        let value = next_value(&mut state);
        if value.abs() == 0.0 {
            zeros += 1;
        }
        hist.record(value);
    }

    let point = hist.to_proto(vec![], 0, 0, vec![]);
    let positive: u64 = point
        .positive
        .as_ref()
        .expect("positive buckets are always exported")
        .bucket_counts
        .iter()
        .sum();
    let negative: u64 = point
        .negative
        .as_ref()
        .expect("negative buckets are always exported")
        .bucket_counts
        .iter()
        .sum();
    assert_eq!(
        point.count,
        zeros + positive + negative,
        "count must equal zero_count plus bucket counts"
    );
    assert!(
        (EXPO_MIN_SCALE as i32..=EXPO_MAX_SCALE as i32).contains(&point.scale),
        "scale {} out of bounds",
        point.scale
    );

    if point.count > 0 {
        let min = point.min.expect("min is set when count > 0");
        let max = point.max.expect("max is set when count > 0");
        assert!(min <= max, "min {min} > max {max}");
        let sum = point.sum.expect("sum is set when count > 0");
        let mean = sum / point.count as f64;
        if min.is_finite() && max.is_finite() && mean.is_finite() {
            let epsilon = 1e-9 * max.abs().max(1.0);
            assert!(
                mean + epsilon >= min && mean - epsilon <= max,
                "mean {mean} outside [{min}, {max}]"
            );
        }
    }
});
