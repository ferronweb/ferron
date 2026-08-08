use std::borrow::Cow;

use crate::proto::opentelemetry::proto::common::v1::KeyValue;
use crate::proto::opentelemetry::proto::metrics::v1::{
    exponential_histogram_data_point::Buckets, ExponentialHistogramDataPoint, HistogramDataPoint,
};

use super::bin;

/// Scale limits, bucket limit, and the binning formulas live in
/// [`super::bin`]; this module re-exports the bounds it is configured with.
pub use super::bin::{EXPO_MAX_SCALE, EXPO_MAX_SIZE, EXPO_MIN_SCALE};

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

    #[inline]
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
    #[inline]
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
    #[inline]
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
    #[inline]
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
    #[inline]
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
    /// NaN and infinite measurements are dropped: they cannot be assigned to
    /// a bucket, and they would corrupt the count, sum, and min/max. A
    /// finite measurement that cannot fit even at the minimum scale is
    /// silently dropped too. Neither kind affects the count, sum, or
    /// min/max.
    #[inline]
    pub fn record(&mut self, value: f64) {
        if !value.is_finite() {
            return;
        }
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
            let delta = bin::scale_delta(
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
                bin = bin::get_bin(abs, self.scale);
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
    #[inline]
    pub fn get_bin(&self, value: f64) -> i32 {
        bin::get_bin(value, self.scale)
    }

    /// Export the histogram as an OTLP data point.
    #[inline]
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
    #[inline]
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
    #[inline]
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

    #[inline]
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
        assert_eq!(bin::get_bin(1.0, 0), -1);
        assert_eq!(bin::get_bin(2.0, 0), 0);
        assert_eq!(bin::get_bin(0.5, 0), -2);
        assert_eq!(bin::get_bin(3.0, 0), 1);
        assert_eq!(bin::get_bin(1.5, 0), 0);
        // At the maximum scale, 1 maps to bucket -1 and 2 to bucket 2^20 - 1.
        assert_eq!(bin::get_bin(1.0, EXPO_MAX_SCALE), -1);
        assert_eq!(bin::get_bin(2.0, EXPO_MAX_SCALE), (1 << 20) - 1);
        assert_eq!(bin::get_bin(0.5, EXPO_MAX_SCALE), -1_048_577);
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

    #[test]
    fn non_finite_values_are_dropped() {
        let mut histogram = ExpoHistogram::new();
        histogram.record(f64::NAN);
        histogram.record(f64::INFINITY);
        histogram.record(f64::NEG_INFINITY);
        assert_eq!(histogram.count, 0, "dropped values must not count");
        assert_eq!(histogram.sum, 0.0);
        assert_eq!(histogram.min, f64::MAX, "min must stay untouched");
        assert_eq!(histogram.max, f64::MIN, "max must stay untouched");
        let point = histogram.to_proto(vec![], 0, 0, vec![]);
        assert_eq!(point.count, 0);
        assert_eq!(point.zero_count, 0);
        let positive: u64 = point.positive.unwrap().bucket_counts.iter().sum();
        assert_eq!(positive, 0);

        // Finite measurements still record normally afterwards.
        histogram.record(1.0);
        let point = histogram.to_proto(vec![], 0, 0, vec![]);
        assert_eq!(point.count, 1);
        assert_eq!(point.sum, Some(1.0));
        assert_eq!(point.min, Some(1.0));
        assert_eq!(point.max, Some(1.0));
    }

    #[test]
    fn subnormal_and_min_normal_values_map_and_record() {
        // The smallest subnormal is 2^-1074 (bits 0x1): it must reach the
        // right bucket and be accounted for exactly once per record.
        let smallest_subnormal = f64::from_bits(1);
        assert_eq!(bin::get_bin(smallest_subnormal, 0), -1075);
        assert_eq!(
            bin::get_bin(smallest_subnormal, EXPO_MAX_SCALE),
            -((1074 << 20) + 1),
            "scale 20 bin for 2^-1074"
        );
        // The smallest normal (2^-1022) is the upper edge of bucket -1023.
        assert_eq!(bin::get_bin(f64::MIN_POSITIVE, 0), -1023);

        let mut histogram = ExpoHistogram::new();
        for _ in 0..4 {
            histogram.record(smallest_subnormal);
        }
        let point = histogram.to_proto(vec![], 0, 0, vec![]);
        let positive: u64 = point.positive.unwrap().bucket_counts.iter().sum();
        assert_eq!(point.count, 4);
        assert_eq!(positive, 4);
        assert_eq!(point.sum, Some(4.0 * smallest_subnormal));
        assert_eq!(point.min, Some(smallest_subnormal));
        assert_eq!(point.max, Some(smallest_subnormal));
    }
}
