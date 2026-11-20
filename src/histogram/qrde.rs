//! `qrde()`: a density of the values whose bins hold equal shares of them, from Harrell-Davis quantiles.
//!
//! https://github.com/nodejs/node/blob/v26.3.0/src/histogram.cc

use core::ops::Range;

use crate::Histogram;
use crate::beta::{Beta, clamp};

/// Which slots `qrde()` spreads the values of over the range of the slot rather than taking them at its middle.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Dequantize {
    None,
    /// Those wider than 1.
    Hdr,
    All,
}

/// The estimate of a [`Histogram::qrde`].
#[derive(Debug, Default, PartialEq)]
pub struct Qrde {
    /// The value at each probability. Empty if the histogram is.
    pub quantiles: Vec<f64>,
    /// The share of the values between two quantiles over the distance between them, infinite where that is 0.
    pub densities: Vec<f64>,
    /// How many quantiles were raised to the one before them.
    pub corrections: usize,
    /// How many slots hold values.
    pub buckets: usize,
}

/// A slot in use.
struct Bucket {
    middle: f64,
    width: f64,
    count: u64,
    /// The values up to and in it.
    cumulative: u64,
}

impl Histogram {
    /// The quantiles at `probabilities`, at least 2 that go up from 0 to 1, and the densities between them.
    pub fn qrde(&self, probabilities: &[f64], dequantize: Dequantize) -> Qrde {
        let mut cumulative = 0;
        let buckets: Vec<Bucket> = self
            .slots()
            .map(|(slot, count)| {
                cumulative += count;
                Bucket {
                    middle: slot.median() as f64,
                    width: slot.width as f64,
                    count,
                    cumulative,
                }
            })
            .collect();
        let ranges = Ranges {
            buckets: &buckets,
            dequantize,
        };
        let last = buckets.len().checked_sub(1);
        let (Some(lowest), Some(highest)) = (ranges.at(0), last.and_then(|last| ranges.at(last)))
        else {
            return Qrde::default();
        };
        let (lowest, highest) = (lowest.0, highest.1);
        let mut corrections = 0;
        let mut quantiles = Vec::with_capacity(probabilities.len());
        quantiles.push(lowest);
        for &p in probabilities
            .iter()
            .skip(1)
            .take(probabilities.len().saturating_sub(2))
        {
            let previous = quantiles.last().copied().unwrap_or(lowest);
            let mut value = clamp(ranges.quantile(self.count, p, highest), lowest, highest);
            if value < previous {
                value = previous;
                corrections += 1;
            }
            quantiles.push(value);
        }
        quantiles.push(highest);
        let densities = quantiles
            .iter()
            .zip(quantiles.iter().skip(1))
            .zip(probabilities.iter().zip(probabilities.iter().skip(1)))
            .map(|((low, high), (p, next_p))| {
                let width = high - low;
                if width == 0.0 {
                    f64::INFINITY
                } else {
                    (next_p - p) / width
                }
            })
            .collect();
        Qrde {
            quantiles,
            densities,
            corrections,
            buckets: buckets.len(),
        }
    }
}

struct Ranges<'b> {
    buckets: &'b [Bucket],
    dequantize: Dequantize,
}

impl Ranges<'_> {
    fn spreads(&self, bucket: &Bucket) -> bool {
        bucket.count > 1
            && match self.dequantize {
                Dequantize::None => false,
                Dequantize::Hdr => bucket.width > 1.0,
                Dequantize::All => true,
            }
    }

    /// The values that the bucket at `index` is spread over. That at either end reaches only to its middle on the
    /// outer side, unless it is the only one.
    fn at(&self, index: usize) -> Option<(f64, f64)> {
        let bucket = self.buckets.get(index)?;
        let (value, half) = (bucket.middle, bucket.width / 2.0);
        if !self.spreads(bucket) {
            return Some((value, value));
        }
        let first = index == 0;
        let last = index + 1 == self.buckets.len();
        Some(match (first, last) {
            (true, false) => (value, value + half),
            (false, true) => (value - half, value),
            _ => (value - half, value + half),
        })
    }

    /// The Harrell-Davis estimate at `p`: the mean of the values, each weighted by the chance that the beta
    /// distribution of the quantile at `p` falls between the shares of the values below and up to it.
    fn quantile(&self, total: u64, p: f64, highest: f64) -> f64 {
        let count = total as f64;
        let (a, b) = ((count + 1.0) * p, (count + 1.0) * (1.0 - p));
        let mass = Beta::approximate(a, b);
        let reflected = mass.reflected();

        let (mut previous_cdf, mut previous_survival, mut previous_front) = (0.0, 0.0, 0.0);
        let mut previous_count = 0;
        let mut support = self.support(&mass, total);
        if let Some(before) = support
            .start
            .checked_sub(1)
            .and_then(|i| self.buckets.get(i))
        {
            let (cdf, front) = mass.cdf_and_front(before.cumulative as f64 / count);
            if cdf.is_finite() && (front.is_finite() || self.dequantize == Dequantize::None) {
                (previous_cdf, previous_front) = (clamp(cdf, 0.0, 1.0), front);
                previous_count = before.cumulative;
            } else {
                support = 0..self.buckets.len();
            }
        }
        let mut quantile = self.at(0).unwrap_or_default().0 * previous_cdf;
        let mut using_survival = false;
        for (index, bucket) in self
            .buckets
            .iter()
            .enumerate()
            .take(support.end)
            .skip(support.start)
        {
            let u0 = previous_count as f64 / count;
            let u1 = bucket.cumulative as f64 / count;
            let front;
            let weight;
            // Above the symmetry point the upper tail is computed, so that small weights there are not lost to
            // subtraction from 1.
            if u1 <= mass.symmetry_point {
                let cdf;
                (cdf, front) = mass.cdf_and_front(u1);
                let cdf = clamp(cdf, previous_cdf, 1.0);
                weight = cdf - previous_cdf;
                previous_cdf = cdf;
            } else {
                let survival;
                (survival, front) =
                    reflected.cdf_and_front((total - bucket.cumulative) as f64 / count);
                let (survival, above) = if using_survival {
                    (clamp(survival, 0.0, previous_survival), previous_survival)
                } else {
                    (clamp(survival, 0.0, 1.0 - previous_cdf), 1.0 - previous_cdf)
                };
                weight = above - survival;
                using_survival = true;
                previous_survival = survival;
            }

            if self.spreads(bucket) {
                // I_x(a + 1, b) = I_x(a, b) - front / a, so the first moment of the weight within the bucket needs no
                // second beta distribution.
                let width = bucket.count as f64 / count;
                let moment = clamp(
                    (a / (a + b) - u0) * weight - (front - previous_front) / (a + b),
                    0.0,
                    width * weight,
                );
                let (low, high) = self.at(index).unwrap_or_default();
                quantile += low * weight + (high - low) * moment / width;
            } else {
                quantile += bucket.middle * weight;
            }
            previous_front = front;
            previous_count = bucket.cumulative;
        }
        let remaining = if using_survival {
            previous_survival
        } else {
            1.0 - previous_cdf
        };
        quantile + highest * clamp(remaining, 0.0, 1.0)
    }

    /// The buckets that hold all but a negligible share of the weight of `mass`, to skip the rest.
    fn support(&self, mass: &Beta, total: u64) -> Range<usize> {
        let all = 0..self.buckets.len();
        let count = total as f64;
        let first_reaching = |share: f64, from: usize| {
            from + self
                .buckets
                .get(from..)
                .unwrap_or_default()
                .partition_point(|bucket| (bucket.cumulative as f64) < share * count)
        };
        if mass.asymptotic_cdf {
            // Beyond 12 standard deviations the normal tails hold less than 2e-33.
            let spread = 12.0 * mass.standard_deviation;
            let first = first_reaching((mass.mean - spread).max(0.0), 0);
            let last = first_reaching((mass.mean + spread).min(1.0), first);
            return first..(last + 1).min(all.end);
        }
        if self.buckets.len() < 512 {
            return all;
        }
        // Dropping both tails moves a quantile by at most 2^-79 of the range of the histogram.
        const TAIL: f64 = 1.0 / (1u128 << 80) as f64;
        let reflected = mass.reflected();
        let mut finite = true;
        let first = self.buckets.partition_point(|bucket| {
            let cdf = mass.cdf(bucket.cumulative as f64 / count);
            finite &= cdf.is_finite();
            cdf < TAIL
        });
        let last = first
            + self
                .buckets
                .get(first..)
                .unwrap_or_default()
                .partition_point(|bucket| {
                    let survival = reflected.cdf((total - bucket.cumulative) as f64 / count);
                    finite &= survival.is_finite();
                    survival > TAIL
                });
        if !finite || last >= all.end {
            return all;
        }
        first..last + 1
    }
}
