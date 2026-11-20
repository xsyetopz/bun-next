//! `ksTest()`, `cohensD()` and `cliffsD()`: how far apart the values of two histograms are.
//!
//! Node.js: lines the slots of the two up by index, so the slots it compares hold different values unless both
//! histograms were created with the same `lowest`, `highest` and `figures`. The slots are lined up by their lowest
//! value here, which is the same if they were.

use crate::Histogram;

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
