//! The beta distribution, which the binomial and Student's t-distributions, and the weights of `qrde()`, come from.
//!
//! https://github.com/nodejs/node/blob/v26.13.1/src/histogram.cc

use core::f64::consts::{PI, SQRT_2};

/// A beta distribution with shapes `a` and `b`.
#[derive(Clone, Copy)]
pub(crate) struct Beta {
    a: f64,
    b: f64,
    log_normalization: f64,
    pub(crate) mean: f64,
    pub(crate) standard_deviation: f64,
    skewness: f64,
    excess_kurtosis: f64,
    log_scale: f64,
    /// The continued fraction converges fastest below it, so above it the other tail is computed.
    pub(crate) symmetry_point: f64,
    asymptotic_front: bool,
    pub(crate) asymptotic_cdf: bool,
}

impl Beta {
    pub(crate) fn new(a: f64, b: f64) -> Self {
        Self::with(a, b, false)
    }

    /// Takes the front from Stirling's series when both shapes are at least 8, and the whole distribution from its
    /// Edgeworth expansion when they are so large that the continued fraction would need more terms than it has.
    pub(crate) fn approximate(a: f64, b: f64) -> Self {
        Self::with(a, b, true)
    }

    fn with(a: f64, b: f64, approximate: bool) -> Self {
        let sum = a + b;
        let mean = a / sum;
        let asymptotic_front = approximate && a >= 8.0 && b >= 8.0;
        let variance = mean * (1.0 - mean) / (sum + 1.0);
        Self {
            a,
            b,
            log_normalization: if asymptotic_front {
                0.0
            } else {
                sum.ln_gamma().0 - a.ln_gamma().0 - b.ln_gamma().0
            },
            mean,
            standard_deviation: variance.sqrt(),
            skewness: 2.0 * (b - a) * (sum + 1.0).sqrt() / ((sum + 2.0) * (a * b).sqrt()),
            excess_kurtosis: 6.0 * ((a - b) * (a - b) * (sum + 1.0) - a * b * (sum + 2.0))
                / (a * b * (sum + 2.0) * (sum + 3.0)),
            log_scale: if asymptotic_front {
                0.5 * (sum * mean * (1.0 - mean) / (2.0 * PI)).ln() + stirling_correction(sum)
                    - stirling_correction(a)
                    - stirling_correction(b)
            } else {
                0.0
            },
            symmetry_point: (a + 1.0) / (sum + 2.0),
            asymptotic_front,
            asymptotic_cdf: approximate && a * b / sum >= 25_000.0,
        }
    }

    /// The distribution of `1 - x`.
    pub(crate) fn reflected(self) -> Self {
        Self {
            a: self.b,
            b: self.a,
            mean: 1.0 - self.mean,
            skewness: -self.skewness,
            symmetry_point: 1.0 - self.symmetry_point,
            ..self
        }
    }

    /// The chance of at most `x`.
    pub(crate) fn cdf(&self, x: f64) -> f64 {
        self.cdf_and_front(x).0
    }

    /// The chance of at most `x`, and `x^a (1 - x)^b / B(a, b)`, which is 0 outside of 0 to 1.
    pub(crate) fn cdf_and_front(&self, x: f64) -> (f64, f64) {
        if x <= 0.0 {
            return (0.0, 0.0);
        }
        if x >= 1.0 {
            return (1.0, 0.0);
        }
        if self.asymptotic_cdf {
            return self.edgeworth(x);
        }
        let front = self.front(x);
        let cdf = if x < self.symmetry_point {
            front * continued_fraction(self.a, self.b, x) / self.a
        } else {
            1.0 - front * continued_fraction(self.b, self.a, 1.0 - x) / self.b
        };
        (cdf, front)
    }

    fn front(&self, x: f64) -> f64 {
        let log_front = if self.asymptotic_front {
            self.a * ((x - self.mean) / self.mean).ln_1p()
                + self.b * ((self.mean - x) / (1.0 - self.mean)).ln_1p()
                + self.log_scale
        } else {
            self.log_normalization + self.a * x.ln() + self.b * (1.0 - x).ln()
        };
        log_front.exp()
    }

    fn edgeworth(&self, x: f64) -> (f64, f64) {
        let z = (x - self.mean) / self.standard_deviation;
        let normal_cdf = 0.5 * (-z / SQRT_2).erfc();
        if z.abs() >= 8.0 {
            return (normal_cdf, 0.0);
        }
        let (z2, skewness) = (z * z, self.skewness);
        let (z3, z4) = (z2 * z, z2 * z2);
        let normal_pdf = (-0.5 * z2).exp() / (2.0 * PI).sqrt();
        let density_correction = 1.0
            + skewness / 6.0 * (z3 - 3.0 * z)
            + self.excess_kurtosis / 24.0 * (z4 - 6.0 * z2 + 3.0)
            + skewness * skewness / 72.0 * (z3 * z3 - 15.0 * z4 + 45.0 * z2 - 15.0);
        let front =
            x * (1.0 - x) * normal_pdf / self.standard_deviation * density_correction.max(0.0);
        let correction = skewness / 6.0 * (1.0 - z2)
            - self.excess_kurtosis / 24.0 * (z3 - 3.0 * z)
            - skewness * skewness / 72.0 * (z3 * z2 - 10.0 * z3 + 15.0 * z);
        (clamp(normal_cdf + normal_pdf * correction, 0.0, 1.0), front)
    }
}

/// `std::clamp`, which passes NaN through, as `f64::clamp` does but without its panic for bounds out of order.
pub(crate) fn clamp(value: f64, low: f64, high: f64) -> f64 {
    if value < low {
        low
    } else if high < value {
        high
    } else {
        value
    }
}

/// What Stirling's series adds to the log of the gamma function of `x`.
fn stirling_correction(x: f64) -> f64 {
    let inverse = 1.0 / x;
    let squared = inverse * inverse;
    inverse
        * (1.0 / 12.0
            + squared
                * (-1.0 / 360.0
                    + squared * (1.0 / 1260.0 + squared * (-1.0 / 1680.0 + squared / 1188.0))))
}

/// Lentz's method, as in Numerical Recipes in C, 2nd edition, section 6.4.
fn continued_fraction(a: f64, b: f64, x: f64) -> f64 {
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
