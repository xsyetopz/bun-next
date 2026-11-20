//! `linearBuckets()` and `logBuckets()`: the counts gathered under a series of rising levels, the way the linear and
//! the logarithmic iterators of HdrHistogram walk the slots.
//!
//! The walk stops at the slot of each level and reports what it passed since the last stop under the lowest value
//! of that slot, then raises the level until it is past the slot. A level that would pass `i64::MAX` is `i64::MAX`,
//! and after that one the walk goes to the last slot that a value can reach and stops there for good.

use crate::Histogram;

const LAST_LEVEL: u64 = i64::MAX as u64;

impl Histogram {
    /// The counts under the levels `step`, `2 * step`, `3 * step` and so on. `step` is at least 1.
    pub fn linear_buckets(&self, step: u64) -> Vec<(u64, u64)> {
        self.buckets(Some(step), |level, past| {
            if level == LAST_LEVEL {
                return None;
            }
            let steps = (past - level) / step + 1;
            let next = steps.checked_mul(step).and_then(|up| level.checked_add(up));
            Some(
                next.filter(|&next| next <= LAST_LEVEL)
                    .unwrap_or(LAST_LEVEL),
            )
        })
    }

    /// The counts under the levels `first`, `first * base`, `first * base * base` and so on, with `base` rounded
    /// towards 0. `first` is at least 1.
    pub fn log_buckets(&self, first: u64, base: f64) -> Vec<(u64, u64)> {
        let usable = base.is_finite() && base > 1.0 && base < LAST_LEVEL as f64;
        let base = base as u64;
        self.buckets(usable.then_some(first), |level, _| {
            if level == LAST_LEVEL {
                None
            } else if base <= 1 || level > LAST_LEVEL / base {
                Some(LAST_LEVEL)
            } else {
                Some(level * base)
            }
        })
    }

    /// `raise(level, past)` is the next level, which may still be at or below `past`, or `None` after the last.
    ///
    /// Node.js: the walk stops at a slot once for every level in it, and reports 0 every time after the first. The
    /// counts that it reported go into a `Map` under the same key, so the 0 replaces them.
    fn buckets(
        &self,
        first: Option<u64>,
        raise: impl Fn(u64, u64) -> Option<u64>,
    ) -> Vec<(u64, u64)> {
        let mut buckets = Vec::new();
        if self.count == 0 {
            return buckets;
        }
        let raise_past = |mut level: Option<u64>, past: u64| {
            while let Some(below) = level.filter(|&level| level <= past) {
                level = raise(below, past);
            }
            level
        };
        let mut level = first;
        // Before the walk reaches slot 0, it stands at value 0.
        if level.is_some_and(|level| self.index_of(level) == 0) {
            buckets.push((0, 0));
            level = raise_past(level, self.slot(0).highest());
        }
        let magnitude = self.sub_bucket_half_count_magnitude;
        // The slots of the buckets up to that of the highest trackable value. Bucket 0 holds both halves.
        let slot_count =
            ((self.index_of(self.highest_trackable_value) >> magnitude).max(1) + 1) << magnitude;
        let mut counts = self.counts().peekable();
        let mut remaining = self.count;
        while remaining > 0 {
            let stop = level.map_or(slot_count, |level| self.index_of(level));
            if stop >= slot_count {
                buckets.push((self.slot(slot_count - 1).lowest, remaining));
                break;
            }
            let mut passed = 0;
            while let Some((_, n)) = counts.next_if(|&(index, _)| index <= stop) {
                passed += n;
            }
            let slot = self.slot(stop);
            buckets.push((slot.lowest, passed));
            remaining -= passed;
            level = raise_past(level, slot.highest());
        }
        buckets
    }
}
