use std::borrow::Cow;
use std::sync::OnceLock;

use crate::proto::opentelemetry::proto::common::v1::KeyValue;
use crate::proto::opentelemetry::proto::metrics::v1::{
    exponential_histogram_data_point::Buckets, ExponentialHistogramDataPoint, HistogramDataPoint,
};

/// Maximum scale of the Base2 exponential histogram (parity with the SDK
/// view: `max_scale 20`).
pub const EXPO_MAX_SCALE: i8 = 20;
/// Minimum scale before a measurement is dropped (parity with the SDK's
/// `EXPO_MIN_SCALE`).
pub const EXPO_MIN_SCALE: i8 = -10;
/// Maximum number of buckets in the exponential histogram (parity with the
/// SDK view: `max_size 160`).
pub const EXPO_MAX_SIZE: i32 = 160;

/// The default upper bounds of the buckets; the final bucket has no upper bound.
pub(super) const DEFAULT_EXPLICIT_BOUNDS: &[f64] = &[
    0.0, 5.0, 10.0, 25.0, 50.0, 75.0, 100.0, 250.0, 500.0, 750.0, 1000.0, 2500.0, 5000.0, 7500.0,
    10000.0,
];

/// A histogram aggregated into fixed, explicit buckets.
#[derive(Debug)]
pub struct ExplicitHistogram {
    count: u64,
    min: f64,
    max: f64,
    sum: f64,
    bucket_counts: Vec<u64>,
    buckets: Cow<'static, [f64]>,
}

impl ExplicitHistogram {
    #[inline]
    pub fn new() -> Self {
        Self::with_buckets(DEFAULT_EXPLICIT_BOUNDS.into())
    }

    pub fn with_buckets(buckets: Cow<'static, [f64]>) -> Self {
        Self {
            count: 0,
            min: f64::MAX,
            max: f64::MIN,
            sum: 0.0,
            bucket_counts: vec![0; buckets.len() + 1],
            buckets,
        }
    }

    /// Record one measurement into the bucket that holds it.
    pub fn record(&mut self, value: f64) {
        self.count += 1;
        if value < self.min {
            self.min = value;
        }
        if value > self.max {
            self.max = value;
        }
        self.sum += value;
        let index = self.buckets.partition_point(|bound| value > *bound);
        self.bucket_counts[index] += 1;
    }

    /// Export the histogram as an OTLP data point.
    pub fn to_proto(
        &self,
        attributes: Vec<KeyValue>,
        start_time_unix_nano: u64,
        time_unix_nano: u64,
        exemplars: Vec<crate::proto::opentelemetry::proto::metrics::v1::Exemplar>,
    ) -> HistogramDataPoint {
        HistogramDataPoint {
            attributes,
            start_time_unix_nano,
            time_unix_nano,
            count: self.count,
            sum: Some(self.sum),
            min: Some(self.min),
            max: Some(self.max),
            explicit_bounds: self.buckets.to_vec(),
            bucket_counts: self.bucket_counts.clone(),
            flags: 0,
            exemplars,
        }
    }
}

/// A measurement that cannot fit even at the minimum scale is silently
/// dropped (parity with the SDK, which logs a debug message instead).
#[derive(Debug)]
pub struct ExpoHistogram {
    max_size: i32,
    count: u64,
    min: f64,
    max: f64,
    sum: f64,
    scale: i8,
    positive: ExpoBuckets,
    negative: ExpoBuckets,
    zero_count: u64,
}

impl ExpoHistogram {
    pub fn new() -> Self {
        Self {
            max_size: EXPO_MAX_SIZE,
            count: 0,
            min: f64::MAX,
            max: f64::MIN,
            sum: 0.0,
            scale: EXPO_MAX_SCALE,
            positive: ExpoBuckets::default(),
            negative: ExpoBuckets::default(),
            zero_count: 0,
        }
    }

    /// Rescale to a smaller scale (fewer buckets; `delta` bucket rows are
    /// merged). Used when a range of bins no longer fits and to honor the
    /// "downscale" semantic of cumulative histograms.
    pub fn downscale(&mut self, delta: u32) {
        if delta == 0 {
            return;
        }
        self.scale -= delta as i8;
        self.positive.downscale(delta);
        self.negative.downscale(delta);
    }

    /// Record one measurement into the histogram, resizing the buckets if
    /// needed.
    ///
    /// A measurement that cannot fit even at the minimum scale is silently
    /// dropped and does not affect the count, sum, or min/max.
    pub fn record(&mut self, value: f64) {
        let abs = value.abs();
        if abs == 0.0 {
            self.zero_count += 1;
        } else {
            let value_negative = value < 0.0;
            let mut bin = self.get_bin(abs);

            let bucket = if value_negative {
                &self.negative
            } else {
                &self.positive
            };
            let delta = scale_delta(
                self.max_size,
                bin,
                bucket.offset,
                bucket.counts.len() as i32,
            );
            if delta > 0 {
                if (self.scale - delta as i8) < EXPO_MIN_SCALE {
                    // The measurement cannot fit even at the minimum scale;
                    // drop it.
                    return;
                }
                self.downscale(delta);
                bin = get_bin(abs, self.scale);
            }

            if value_negative {
                self.negative.record(bin);
            } else {
                self.positive.record(bin);
            }
        }

        self.count += 1;
        if value < self.min {
            self.min = value;
        }
        if value > self.max {
            self.max = value;
        }
        self.sum += value;
    }

    /// The index of the bucket `value` belongs to at a given scale.
    pub fn get_bin(&self, value: f64) -> i32 {
        get_bin(value, self.scale)
    }

    /// Export the histogram as an OTLP data point.
    pub fn to_proto(
        &self,
        attributes: Vec<KeyValue>,
        start_time_unix_nano: u64,
        time_unix_nano: u64,
        exemplars: Vec<crate::proto::opentelemetry::proto::metrics::v1::Exemplar>,
    ) -> ExponentialHistogramDataPoint {
        ExponentialHistogramDataPoint {
            attributes,
            start_time_unix_nano,
            time_unix_nano,
            count: self.count,
            sum: Some(self.sum),
            scale: self.scale as i32,
            zero_count: self.zero_count,
            positive: Some(self.positive.to_proto()),
            negative: Some(self.negative.to_proto()),
            flags: 0,
            exemplars,
            min: Some(self.min),
            max: Some(self.max),
            zero_threshold: 0.0,
        }
    }
}

/// The bucket index that holds `value` at `scale`, following the OTel
/// exponential histogram mapping formula.
fn get_bin(value: f64, scale: i8) -> i32 {
    debug_assert!(value >= 0.0 && value.is_finite(), "invalid histogram value");
    let (frac, exp) = frexp(value);
    if scale <= 0 {
        // With a negative scale, `frac` is always one power of two higher
        // than desired.
        let correction = if frac == 0.5 { 2 } else { 1 };
        return (exp - correction) >> -scale;
    }
    (exp << scale) + (frac.ln() * scale_factors()[scale as usize]) as i32 - 1
}

/// The number of scale reductions needed to fit `bin` within `[start_bin,
/// start_bin + length)` buckets of size `max_size`. Returns 0 when no
/// reduction is needed.
fn scale_delta(max_size: i32, bin: i32, start_bin: i32, length: i32) -> u32 {
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
fn scale_factors() -> &'static [f64; 21] {
    SCALE_FACTORS
        .get_or_init(|| std::array::from_fn(|i| std::f64::consts::LOG2_E * 2f64.powi(i as i32)))
}

/// Break a positive float into a normalized fraction and base-2 exponent
/// (libc `frexp`, reimplemented because Rust removed it from std).
fn frexp(value: f64) -> (f64, i32) {
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

/// A set of buckets of an exponential histogram.
#[derive(Debug, Default)]
pub(super) struct ExpoBuckets {
    /// Index of the first bucket in `counts`.
    offset: i32,
    /// Bucket counts, bucket `i` of the histogram lives at `offset + i`.
    counts: Vec<u64>,
}

impl ExpoBuckets {
    /// Increment the count for the given bin, expanding the counts if needed.
    pub(super) fn record(&mut self, bin: i32) {
        if self.counts.is_empty() {
            self.counts = vec![1];
            self.offset = bin;
            return;
        }

        let end_bin = self.offset + self.counts.len() as i32 - 1;

        // Inside the current range.
        if bin >= self.offset && bin <= end_bin {
            self.counts[(bin - self.offset) as usize] += 1;
            return;
        }

        // Before the current start: prepend zero buckets.
        if bin < self.offset {
            let mut new_counts = vec![0; (end_bin - bin + 1) as usize];
            let shift = (self.offset - bin) as usize;
            new_counts[shift..].copy_from_slice(&self.counts);
            new_counts[0] = 1;
            self.counts = new_counts;
            self.offset = bin;
        } else if bin > end_bin {
            // After the current end: append zero buckets and set the count.
            if ((bin - self.offset) as usize) < self.counts.capacity() {
                self.counts.resize((bin - self.offset + 1) as usize, 0);
                self.counts[(bin - self.offset) as usize] = 1;
                return;
            }
            self.counts.extend(std::iter::repeat_n(
                0,
                (bin - self.offset) as usize - self.counts.len() + 1,
            ));
            self.counts[(bin - self.offset) as usize] = 1;
        }
    }

    /// Shrink the buckets by a factor of `2^delta`, summing the merged
    /// counts.
    pub(super) fn downscale(&mut self, delta: u32) {
        if self.counts.len() <= 1 || delta < 1 {
            self.offset >>= delta;
            return;
        }
        let steps = 1 << delta;
        let mut offset = self.offset % steps;
        offset = (offset + steps) % steps;
        for index in 1..self.counts.len() {
            let merged = index + offset as usize;
            if merged.is_multiple_of(steps as usize) {
                self.counts[merged / steps as usize] = self.counts[index];
            } else {
                self.counts[merged / steps as usize] += self.counts[index];
            }
        }
        let last_index = (self.counts.len() as i32 - 1 + offset) / steps;
        self.counts = self.counts[..last_index as usize + 1].to_vec();
        self.offset >>= delta;
    }

    pub(super) fn to_proto(&self) -> Buckets {
        Buckets {
            offset: self.offset,
            bucket_counts: self.counts.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_indexes_follow_the_otlp_mapping() {
        // At scale 0 the buckets are unit-width powers of two: bucket -2 =
        // (0.25, 0.5], bucket -1 = (0.5, 1], bucket 0 = (1, 2], bucket 1 =
        // (2, 4].
        assert_eq!(get_bin(1.0, 0), -1);
        assert_eq!(get_bin(2.0, 0), 0);
        assert_eq!(get_bin(0.5, 0), -2);
        assert_eq!(get_bin(3.0, 0), 1);
        assert_eq!(get_bin(1.5, 0), 0);
        // At the maximum scale, 1 maps to bucket -1 and 2 to bucket 2^20 - 1.
        assert_eq!(get_bin(1.0, EXPO_MAX_SCALE), -1);
        assert_eq!(get_bin(2.0, EXPO_MAX_SCALE), (1 << 20) - 1);
        assert_eq!(get_bin(0.5, EXPO_MAX_SCALE), -1_048_577);
    }

    #[test]
    fn buckets_prepend_append_and_downscale() {
        // Append: bucket 0 = 1, append bucket 2 -> counts [1, 0, 1].
        let mut buckets = ExpoBuckets::default();
        buckets.record(0);
        buckets.record(2);
        assert_eq!(buckets.offset, 0);
        assert_eq!(buckets.counts, vec![1, 0, 1]);
        // Prepend: bucket -1 shifts everything right.
        buckets.record(-1);
        assert_eq!(buckets.offset, -1);
        assert_eq!(buckets.counts, vec![1, 1, 0, 1]);
        // Downscale merges adjacent bins into the half as many buckets.
        let mut buckets = ExpoBuckets::default();
        for bin in 0..4 {
            buckets.record(bin);
        }
        assert_eq!(buckets.counts, vec![1, 1, 1, 1]);
        buckets.downscale(1);
        assert_eq!(buckets.offset, 0);
        assert_eq!(buckets.counts, vec![2, 2]);
    }

    #[test]
    fn recorded_values_are_accounted_for_exactly() {
        // A spread of extreme magnitudes forces several downscales; every
        // recorded value must still be accounted for exactly once.
        let mut hist = ExpoHistogram::new();
        for value in [
            1.0,
            2.0,
            0.5,
            -1.0,
            f64::MAX,
            -f64::MAX,
            f64::MIN_POSITIVE,
            0.0,
            -0.0,
        ] {
            hist.record(value);
        }
        let point = hist.to_proto(vec![], 0, 0, vec![]);
        let positive: u64 = point.positive.as_ref().unwrap().bucket_counts.iter().sum();
        let negative: u64 = point.negative.as_ref().unwrap().bucket_counts.iter().sum();
        assert_eq!(point.count, 9, "count must cover every recorded value");
        assert_eq!(point.zero_count, 2, "both zero representations count");
        assert_eq!(positive, 5);
        assert_eq!(negative, 2);
        assert_eq!(point.sum, Some(f64::MIN_POSITIVE), "sum drifts");
        assert_eq!(point.min, Some(-f64::MAX));
        assert_eq!(point.max, Some(f64::MAX));
    }
}
