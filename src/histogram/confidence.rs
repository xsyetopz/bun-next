//! `meanCI()` and `percentileCI()`, and the distributions that they and the tests in `compare.rs` take their
//! critical values from.
//!
//! https://github.com/nodejs/node/blob/v26.13.1/src/histogram.cc

use crate::Histogram;
use crate::beta::Beta;

impl Histogram {
    /// The mean, and the bounds that it lies between with probability `confidence`, from Student's t-distribution.
    /// The bounds are NaN if fewer than 2 values were counted. `confidence` is above 0 and below 1.
    pub fn mean_ci(&self, confidence: f64) -> [f64; 3] {
        let mean = self.mean();
        if self.count < 2 {
            return [mean, f64::NAN, f64::NAN];
        }
        let n = self.count as f64;
        let sample_variance = self.stddev().powi(2) * n / (n - 1.0);
        if sample_variance == 0.0 {
            return [mean, mean, mean];
        }
        let margin = student_t_upper_quantile((1.0 - confidence) / 2.0, n - 1.0)
            * (sample_variance / n).sqrt();
        [mean, mean - margin, mean + margin]
    }

    /// The value at `percentile`, and the values at the ranks that it lies between with probability `confidence`,
    /// from the binomial distribution. All three are the value if fewer than 2 values were counted.
    pub fn percentile_ci(&self, percentile: f64, confidence: f64) -> [u64; 3] {
        let value = self.value_at_percentile(percentile);
        if self.count < 2 {
            return [value; 3];
        }
        let n = self.count;
        let share = percentile / 100.0;
        let tail = (1.0 - confidence) / 2.0;
        // The highest rank whose values below it are no more likely than the tail.
        let (mut lowest, mut highest) = (0, n);
        while lowest < highest {
            let middle = lowest + (highest - lowest).div_ceil(2);
            if binomial_cdf(middle, n, share) <= tail {
                lowest = middle;
            } else {
                highest = middle - 1;
            }
        }
        let lower = lowest;
        // The lowest rank whose values below it are at least as likely as all but the tail.
        let (mut lowest, mut highest) = (0, n);
        while lowest < highest {
            let middle = lowest + (highest - lowest) / 2;
            if binomial_cdf(middle, n, share) >= 1.0 - tail {
                highest = middle;
            } else {
                lowest = middle + 1;
            }
        }
        let at_rank = |rank: u64| self.value_at_percentile(rank as f64 / n as f64 * 100.0);
        [value, at_rank(lower), at_rank(lowest)]
    }
}

/// The chance that a standard normal variable is at most `x`.
pub(crate) fn normal_cdf(x: f64) -> f64 {
    0.5 * (-x / core::f64::consts::SQRT_2).erfc()
}

/// The chance that a variable of Student's t-distribution with `df` degrees of freedom is at most `t`.
pub(crate) fn student_t_cdf(t: f64, df: f64) -> f64 {
    let squared = t * t;
    let upper = t >= 0.0;
    if squared < df {
        let beta = Beta::new(0.5, df / 2.0).cdf(squared / (df + squared));
        return if upper {
            0.5 + 0.5 * beta
        } else {
            0.5 - 0.5 * beta
        };
    }
    let beta = Beta::new(df / 2.0, 0.5).cdf(df / (df + squared));
    if upper { 1.0 - 0.5 * beta } else { 0.5 * beta }
}

/// The positive `t` that a variable of Student's t-distribution is above with chance `p`, found by bisection.
pub(crate) fn student_t_upper_quantile(p: f64, df: f64) -> f64 {
    let (mut low, mut high) = (0.0, 1.0);
    while student_t_cdf(-high, df) > p {
        high *= 2.0;
    }
    for _ in 0..100 {
        let middle = (low + high) / 2.0;
        if student_t_cdf(-middle, df) > p {
            low = middle;
        } else {
            high = middle;
        }
    }
    (low + high) / 2.0
}

/// The chance that fewer than `rank` of `n` trials succeed if each does with chance `p`.
fn binomial_cdf(rank: u64, n: u64, p: f64) -> f64 {
    if rank == 0 {
        return 0.0;
    }
    if rank > n {
        return 1.0;
    }
    Beta::new((n - rank + 1) as f64, rank as f64).cdf(1.0 - p)
}
