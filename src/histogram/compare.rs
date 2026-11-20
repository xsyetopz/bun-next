//! `ksTest()`, `cohensD()`, `cliffsD()`, `welchTest()` and `mannWhitneyTest()`: how far apart the values of two
//! histograms are.
//!
//! Node.js: lines the slots of the two up by index, so the slots it compares hold different values unless both
//! histograms were created with the same `lowest`, `highest` and `figures`. The slots are lined up by their lowest
//! value here, which is the same if they were.

use crate::Histogram;
use crate::confidence::{normal_cdf, student_t_cdf, student_t_upper_quantile};

/// Welch's t-test of whether two histograms have the same mean.
#[derive(Debug, PartialEq)]
pub struct WelchTest {
    pub t: f64,
    pub degrees_of_freedom: f64,
    /// The chance of a `t` at least this far from 0 if the means are the same.
    pub p: f64,
    /// The bounds that the difference of the means lies between with the confidence asked for.
    pub lower: f64,
    pub upper: f64,
}

/// The Mann-Whitney U test of whether the values of one histogram tend to be larger than those of another.
#[derive(Debug, PartialEq)]
pub struct MannWhitneyTest {
    pub u: f64,
    pub z: f64,
    /// The chance of a `z` at least this far from 0 if neither tends to be larger, from the normal distribution.
    pub p: f64,
}

impl Histogram {
    /// The Kolmogorov-Smirnov statistic: the largest gap between the shares of the values of each that are at most
    /// one value. 0 if either is empty.
    pub fn ks_test(&self, other: &Self) -> f64 {
        if self.count == 0 || other.count == 0 {
            return 0.0;
        }
        let (mut below, mut other_below) = (0, 0);
        let mut gap: f64 = 0.0;
        for (n, other_n) in self.lined_up(other) {
            below += n;
            other_below += other_n;
            let share = below as f64 / self.count as f64;
            let other_share = other_below as f64 / other.count as f64;
            gap = gap.max((share - other_share).abs());
        }
        gap
    }

    /// Cohen's d: how far the mean of this is above that of `other`, in standard deviations of both together. 0 if
    /// either has fewer than 2 values or none of the values differ.
    pub fn cohens_d(&self, other: &Self) -> f64 {
        let (n, other_n) = (self.count as f64, other.count as f64);
        if n < 2.0 || other_n < 2.0 {
            return 0.0;
        }
        // The population variances times their counts are the sums of squares that the sample variances share.
        let squares = self.stddev().powi(2) * n + other.stddev().powi(2) * other_n;
        let pooled = (squares / (n + other_n - 2.0)).sqrt();
        if pooled == 0.0 {
            return 0.0;
        }
        (self.mean() - other.mean()) / pooled
    }

    /// Cliff's delta: the share of the pairs of a value of this and one of `other` where this is larger, less that
    /// where it is smaller. 0 if either is empty.
    pub fn cliffs_d(&self, other: &Self) -> f64 {
        if self.count == 0 || other.count == 0 {
            return 0.0;
        }
        let (mut larger, mut tied, mut other_below) = (0.0, 0.0, 0u64);
        for (n, other_n) in self.lined_up(other) {
            larger += n as f64 * other_below as f64;
            tied += n as f64 * other_n as f64;
            other_below += other_n;
        }
        let pairs = self.count as f64 * other.count as f64;
        let smaller = pairs - larger - tied;
        (larger - smaller) / pairs
    }

    /// 0 with a `p` of 1 if either has fewer than 2 values or none of the values of either differ. `confidence` is
    /// above 0 and below 1.
    pub fn welch_test(&self, other: &Self, confidence: f64) -> WelchTest {
        let none = WelchTest {
            t: 0.0,
            degrees_of_freedom: 0.0,
            p: 1.0,
            lower: 0.0,
            upper: 0.0,
        };
        let (n, other_n) = (self.count as f64, other.count as f64);
        if n < 2.0 || other_n < 2.0 {
            return none;
        }
        // The squared standard errors of the means, from the sample variances.
        let error = self.stddev().powi(2) / (n - 1.0);
        let other_error = other.stddev().powi(2) / (other_n - 1.0);
        let errors = error + other_error;
        if errors == 0.0 {
            return none;
        }
        let difference = self.mean() - other.mean();
        let t = difference / errors.sqrt();
        let degrees_of_freedom = errors * errors
            / (error * error / (n - 1.0) + other_error * other_error / (other_n - 1.0));
        let margin =
            student_t_upper_quantile((1.0 - confidence) / 2.0, degrees_of_freedom) * errors.sqrt();
        WelchTest {
            t,
            degrees_of_freedom,
            p: 2.0 * student_t_cdf(-t.abs(), degrees_of_freedom),
            lower: difference - margin,
            upper: difference + margin,
        }
    }

    /// `u` counts the pairs of a value of this and one of `other` where this is larger, and half of those where they
    /// are tied. 0 with a `p` of 1 if either is empty.
    pub fn mann_whitney_test(&self, other: &Self) -> MannWhitneyTest {
        if self.count == 0 || other.count == 0 {
            return MannWhitneyTest {
                u: 0.0,
                z: 0.0,
                p: 1.0,
            };
        }
        let (mut u, mut ties, mut other_below) = (0.0, 0.0, 0u64);
        for (n, other_n) in self.lined_up(other) {
            u += n as f64 * other_below as f64 + 0.5 * n as f64 * other_n as f64;
            let tied = (n + other_n) as f64;
            ties += tied * tied * tied - tied;
            other_below += other_n;
        }
        let (n, other_n) = (self.count as f64, other.count as f64);
        let total = n + other_n;
        let variance = n * other_n / 12.0 * (total + 1.0 - ties / (total * (total - 1.0)));
        if variance <= 0.0 {
            return MannWhitneyTest { u, z: 0.0, p: 1.0 };
        }
        let z = (u - n * other_n / 2.0) / variance.sqrt();
        MannWhitneyTest {
            u,
            z,
            p: 2.0 * normal_cdf(-z.abs()),
        }
    }

    /// The counts of the slots of this and of `other` from the lowest value, paired where their lowest values match.
    fn lined_up<'h>(&'h self, other: &'h Self) -> impl Iterator<Item = (u64, u64)> + 'h {
        let mut slots = self.slots().peekable();
        let mut other_slots = other.slots().peekable();
        core::iter::from_fn(move || {
            let lowest = slots.peek().map(|(slot, _)| slot.lowest);
            let other_lowest = other_slots.peek().map(|(slot, _)| slot.lowest);
            let first = match (lowest, other_lowest) {
                (None, None) => return None,
                (Some(a), Some(b)) => a.min(b),
                (Some(a), None) | (None, Some(a)) => a,
            };
            let n = slots
                .next_if(|(slot, _)| slot.lowest == first)
                .map_or(0, |(_, n)| n);
            let other_n = other_slots
                .next_if(|(slot, _)| slot.lowest == first)
                .map_or(0, |(_, n)| n);
            Some((n, other_n))
        })
    }
}
