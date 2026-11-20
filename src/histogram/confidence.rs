//! `meanCI()` and `percentileCI()`, and the distributions that they take their critical values from.
//!
//! https://github.com/nodejs/node/blob/v26.3.0/src/histogram.cc

use crate::Histogram;

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

/// The chance that a variable of Student's t-distribution with `df` degrees of freedom is at most `t`.
fn student_t_cdf(t: f64, df: f64) -> f64 {
    let squared = t * t;
    let upper = t >= 0.0;
    if squared < df {
        let beta = regularized_incomplete_beta(0.5, df / 2.0, squared / (df + squared));
        return if upper {
            0.5 + 0.5 * beta
        } else {
            0.5 - 0.5 * beta
        };
    }
    let beta = regularized_incomplete_beta(df / 2.0, 0.5, df / (df + squared));
    if upper { 1.0 - 0.5 * beta } else { 0.5 * beta }
}

/// The positive `t` that a variable of Student's t-distribution is above with chance `p`, found by bisection.
fn student_t_upper_quantile(p: f64, df: f64) -> f64 {
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
    regularized_incomplete_beta((n - rank + 1) as f64, rank as f64, 1.0 - p)
}

/// The chance that a variable of the beta distribution with shapes `a` and `b` is at most `x`.
fn regularized_incomplete_beta(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let log_front =
        (a + b).ln_gamma().0 - a.ln_gamma().0 - b.ln_gamma().0 + a * x.ln() + b * (1.0 - x).ln();
    let front = log_front.exp();
    // The continued fraction converges fastest below the mean, so above it the other tail is computed.
    if x < (a + 1.0) / (a + b + 2.0) {
        front * beta_continued_fraction(a, b, x) / a
    } else {
        1.0 - front * beta_continued_fraction(b, a, 1.0 - x) / b
    }
}

/// Lentz's method, as in Numerical Recipes in C, 2nd edition, section 6.4.
fn beta_continued_fraction(a: f64, b: f64, x: f64) -> f64 {
    const TINY: f64 = 1e-30;
    let away_from_0 = |value: f64| if value.abs() < TINY { TINY } else { value };
    let mut c = 1.0;
    let mut d = 1.0 / away_from_0(1.0 - (a + b) * x / (a + 1.0));
    let mut fraction = d;
    for m in 1..=200 {
        let m = f64::from(m);
        let even = m * (b - m) * x / ((a - 1.0 + 2.0 * m) * (a + 2.0 * m));
        let odd = -(a + m) * (a + b + m) * x / ((a + 2.0 * m) * (a + 1.0 + 2.0 * m));
        let mut delta = 1.0;
        for term in [even, odd] {
            d = 1.0 / away_from_0(1.0 + term * d);
            c = away_from_0(1.0 + term / c);
            delta = d * c;
            fraction *= delta;
        }
        if (delta - 1.0).abs() <= 3e-12 {
            break;
        }
    }
    fraction
}
