//! Pure binning math for the Base2 exponential histogram.
//!
//! This module owns the OTel bucket-mapping math (metrics data model,
//! "Exponential Histogram" section), so it can be tested property-style
//! against the specification's range rule, independently of the histogram
//! state machine in [`super::histogram`].
//!
//! ## The mapping
//!
//! A histogram at `scale s` partitions the positive reals into buckets of
//! base `2^(2^-s)`: bucket `bin` holds the values in the range
//!
//! ```text
//! (2^(bin / 2^s), 2^((bin + 1) / 2^s)]
//! ```
//!
//! which is equivalent to `bin = ceil(log2(v) * 2^s) - 1`.
//!
//! The implementation derives the bucket from `v = frac * 2^exp` (the
//! [`frexp`] decomposition, `frac in [0.5, 1)`) instead of calling
//! `log2`:
//!
//! ```text
//! for s >= 0:  bin = exp * 2^s + trunc(log2(frac) * 2^s) - 1,  trunc toward zero
//! ```
//!
//! because `log2(frac) * 2^s` lies in `(-2^s, 0]`, truncation toward zero
//! equals `ceil` of the full sum up to the one-ulp ambiguity of the
//! floating `frac.ln() * LOG2_E * 2^s` product. When `frac == 0.5`
//! (exact powers of two) that product is `-2^s`, and a one-ulp swing
//! either way would flip the bucket by one; those values take a dedicated
//! integer path instead (in [`get_bin`]) where `log2(v) == exp - 1` is
//! exact.
//!
//! For `s < 0` the mapping is purely integer (`frac` only disambiguates
//! the exact power-of-two case):
//!
//! ```text
//! bin = (exp - correction) >> -s,  correction = 2 for frac == 0.5 else 1
//! ```
//!
//! ## Numeric policy
//!
//! [`get_bin`] is called only with finite, non-negative values; the caller
//! routes zeros to the zero bucket and drops NaN and infinities (see
//! [`super::histogram::ExpoHistogram::record`]). The `debug_assert` in
//! [`get_bin`] backs that contract in debug builds.
//!
//! Values within ~1 ulp of a bucket edge are inherently ambiguous in f64;
//! the property tests therefore accept a tolerance of one tenth of the
//! bucket width, so only real mapping errors are rejected.

use std::sync::OnceLock;

/// Maximum scale of the Base2 exponential histogram (parity with the SDK
/// view: `max_scale 20`).
pub const EXPO_MAX_SCALE: i8 = 20;
/// Minimum scale before a measurement is dropped (parity with the SDK's
/// `EXPO_MIN_SCALE`).
pub const EXPO_MIN_SCALE: i8 = -10;
/// Maximum number of buckets in the exponential histogram (parity with the
/// SDK view: `max_size 160`).
pub const EXPO_MAX_SIZE: i32 = 160;

/// The bucket index that holds `value` at `scale`, following the OTel
/// exponential histogram mapping described at the top of this module.
#[inline]
pub fn get_bin(value: f64, scale: i8) -> i32 {
    debug_assert!(value >= 0.0 && value.is_finite(), "invalid histogram value");
    let (frac, exp) = frexp(value);
    if scale <= 0 {
        let correction = if frac == 0.5 { 2 } else { 1 };
        return (exp - correction) >> -scale;
    }
    if frac == 0.5 {
        // Exact power of two: v = 2^(exp-1), so bin = (exp-1)*2^s - 1. All
        // integer; no floating rounding.
        return ((exp - 1) << scale) - 1;
    }
    (exp << scale) + (frac.ln() * scale_factors()[scale as usize]) as i32 - 1
}

/// The number of scale reductions needed to fit `bin` within the window
/// `[start_bin, start_bin + length)` of at most `max_size` buckets.
/// Returns 0 when no reduction is needed.
///
/// Each reduction halves the distance from `bin` to the far edge of the
/// window (`low >>= 1`, `high >>= 1`, exact for signed right shift), so the
/// result is the smallest `delta` with `(high >> delta) - (low >> delta) <
/// max_size`. The loop caps at the scale depth reachable from the maximum
/// scale; reaching the cap means the bin cannot be represented even at the
/// minimum scale.
#[inline]
pub fn scale_delta(max_size: i32, bin: i32, start_bin: i32, length: i32) -> u32 {
    if length == 0 {
        return 0;
    }
    let mut low = start_bin;
    let mut high = bin;
    if start_bin >= bin {
        low = bin;
        high = start_bin + length - 1;
    }
    let mut count = 0u32;
    while high - low >= max_size {
        low >>= 1;
        high >>= 1;
        count += 1;
        if count > (EXPO_MAX_SCALE - EXPO_MIN_SCALE) as u32 {
            return count;
        }
    }
    count
}

static SCALE_FACTORS: OnceLock<[f64; 21]> = OnceLock::new();

/// Precomputed `LOG2_E * 2^scale` factors used by the bin formula.
#[inline]
fn scale_factors() -> &'static [f64; 21] {
    SCALE_FACTORS
        .get_or_init(|| std::array::from_fn(|i| std::f64::consts::LOG2_E * 2f64.powi(i as i32)))
}

/// Break a positive float into `(frac, exp)` with `value == frac * 2^exp`
/// and `frac in [0.5, 1)` (libc `frexp`, reimplemented because Rust removed
/// it from std).
///
/// Subnormals are handled by scaling up by `2^64` and correcting the
/// exponent afterwards; the recursion is at most one level deep. NaN and
/// infinities are clamped to `(1.0, 0)`; the caller's contract (see the
/// module docs) removes them before this point.
#[inline]
pub fn frexp(value: f64) -> (f64, i32) {
    let mut bits = value.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;

    if exponent == 0 {
        if value != 0.0 {
            let two_pow_64 = f64::from_bits(0x43f0_0000_0000_0000);
            let (frac, exp) = frexp(value * two_pow_64);
            return (frac, exp - 64);
        }
        // value is ±0.0; return the zero representation as-is.
        return (value, 0);
    }
    if exponent == 0x7ff {
        // NaN / infinity; clamp the fraction to 1.0 (cannot hold any bucket).
        return (1.0, 0);
    }

    let exponent = exponent - 0x3fe;
    bits &= 0x800f_ffff_ffff_ffff;
    bits |= 0x3fe0_0000_0000_0000;
    (f64::from_bits(bits), exponent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_vectors() {
        // Hand-verified vectors: expected values follow from the OTel range
        // rule (2^(bin/2^s), 2^((bin+1)/2^s)] and, at exact powers of two,
        // from the integer bin formula.
        let vectors: &[(f64, i8, i32)] = &[
            // Scale 0: unit-width buckets in log2 space.
            (0.5, 0, -2),
            (0.75, 0, -1),
            (1.0, 0, -1),
            (1.5, 0, 0),
            (2.0, 0, 0),
            (3.0, 0, 1),
            (4.0, 0, 1),
            (7.0, 0, 2),
            (8.0, 0, 2),
            (15.0, 0, 3),
            // Scale -1: buckets span (4^b, 4^(b+1)].
            (0.25, -1, -2),
            (1.5, -1, 0),
            (4.0, -1, 0),
            (6.0, -1, 1),
            (8.0, -1, 1),
            (15.0, -1, 1),
            (16.0, -1, 1),
            (17.0, -1, 2),
            // Scale 1: buckets span half a power of two.
            (1.0, 1, -1),
            (1.41, 1, 0),
            (1.5, 1, 1),
            (2.0, 1, 1),
            (2.5, 1, 2),
            (10.0, 1, 6),
            // Scale 20: exact powers of two land one bucket under 2^m.
            (1.0, 20, -1),
            (0.5, 20, -1_048_577),
            (2.0, 20, (1 << 20) - 1),
            (4.0, 20, (2 << 20) - 1),
        ];
        for &(value, scale, expected) in vectors {
            assert_eq!(get_bin(value, scale), expected, "get_bin({value}, {scale})");
        }
    }

    #[test]
    fn exact_powers_of_two_land_on_bucket_edges() {
        // v = 2^m must map to m*2^s - 1 for s >= 0 (it is the upper edge of
        // its bucket) and to (m-1) >> -s for s < 0. Integer arithmetic both
        // ways, across the whole f64 exponent range, including subnormals.
        // Note: powi cannot produce values below 2^-1022 at runtime (it
        // computes overflows to infinity), so subnormals come from raw bit
        // patterns instead.
        for m in -1074..=1023 {
            let value = if m >= -1022 {
                2f64.powi(m)
            } else {
                f64::from_bits(1u64 << (m + 1074))
            };
            for scale in 0..=EXPO_MAX_SCALE {
                let expected = ((m << scale) - 1) as i64;
                assert_eq!(
                    get_bin(value, scale) as i64,
                    expected,
                    "2^{m} at scale {scale}"
                );
            }
            for scale in EXPO_MIN_SCALE..0 {
                let expected = (m - 1) >> -scale;
                assert_eq!(get_bin(value, scale), expected, "2^{m} at scale {scale}");
            }
        }
    }

    /// Deterministic splitmix64 so the probes are reproducible.
    #[derive(Clone, Copy)]
    struct SplitMix64(u64);
    impl SplitMix64 {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
    }

    /// The spec bucket bounds, computed with `powf` - independent of the
    /// `frexp` + `ln` path under test.
    fn spec_bounds(bin: i32, scale: i8) -> (f64, f64) {
        let factor = 2f64.powi(scale as i32);
        (
            2f64.powf(bin as f64 / factor),
            2f64.powf((bin + 1) as f64 / factor),
        )
    }

    fn assert_bin_consistent(value: f64, bin: i32, scale: i8) {
        let (lo, hi) = spec_bounds(bin, scale);
        // Within a tenth of a bucket width of a boundary the value is
        // ambiguous in f64; anything more is a real mapping error.
        let tol = (hi - lo) * 0.1;
        assert!(
            value > lo - tol && value <= hi + tol,
            "value {value:e} at scale {scale} maps to bin {bin}, outside ({lo:e}, {hi:e}]"
        );
    }

    #[test]
    fn mapping_matches_spec_boundaries_over_the_whole_f64_space() {
        // Probe many magnitudes: arbitrary bit patterns, exact powers of
        // two, one-ulp nudges across bucket edges, subnormals, and the
        // extreme magnitudes - across every scale.
        let mut rng = SplitMix64(0x853c_49e6_748f_ea9b);
        for round in 0..25_000u64 {
            let value = match rng.next() % 6 {
                // Arbitrary bit pattern (may be NaN/Inf; filtered below).
                0 => f64::from_bits(rng.next()),
                1 => 2f64.powi((rng.next() % 2048) as i32 - 1023),
                2 => {
                    let p = 2f64.powi((rng.next() % 2046) as i32 - 1021);
                    if rng.next() & 1 == 1 {
                        f64::from_bits(p.to_bits() + 1)
                    } else {
                        p
                    }
                }
                3 => (rng.next() as f64) * 2f64.powi((rng.next() % 96) as i32 - 48),
                4 => f64::from_bits(rng.next() % 0x000f_ffff_ffff_ffff),
                _ => match round % 3 {
                    0 => f64::MAX,
                    1 => f64::MIN_POSITIVE,
                    _ => 1.0,
                },
            };
            // NaN, infinities (e.g. 2^1024 from powi) and zero have no
            // bucket; they must not reach get_bin.
            if !value.is_finite() || value == 0.0 {
                continue;
            }
            let value = value.abs();
            for scale in EXPO_MIN_SCALE..=EXPO_MAX_SCALE {
                assert_bin_consistent(value, get_bin(value, scale), scale);
            }
        }
    }

    #[test]
    fn scale_delta_is_minimal_and_sufficient() {
        let mut rng = SplitMix64(0x0123_4567_89ab_cdef);
        for _ in 0..50_000 {
            let max_size = 1 + (rng.next() % 1024) as i32;
            let bin = (rng.next() % (1 << 30)) as i32 - (1 << 30);
            let start = (rng.next() % (1 << 30)) as i32 - (1 << 30);
            let length = 1 + (rng.next() % 4096) as i32;
            let (low, high) = if start >= bin {
                (bin as i64, start as i64 + length as i64 - 1)
            } else {
                (start as i64, bin as i64)
            };
            let delta = scale_delta(max_size, bin, start, length);

            if high - low < max_size as i64 {
                assert_eq!(delta, 0, "already fits: no rescale expected");
                continue;
            }
            assert!(delta > 0, "span does not fit: expected delta > 0");
            assert!(
                (high >> delta) - (low >> delta) < max_size as i64,
                "delta {delta} is not sufficient for max_size {max_size}"
            );
            assert!(
                (high >> (delta - 1)) - (low >> (delta - 1)) >= max_size as i64,
                "delta {delta} is not minimal for max_size {max_size}"
            );
        }
    }
}
