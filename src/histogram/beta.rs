//! The beta distribution, which the binomial and Student's t-distributions, and the weights of `qrde()`, come from.
//!
//! https://github.com/nodejs/node/blob/v26.3.0/src/histogram.cc

/// A beta distribution with shapes `a` and `b`.
#[derive(Clone, Copy)]
pub(crate) struct Beta {
    a: f64,
    b: f64,
    log_normalization: f64,
    /// The continued fraction converges fastest below it, so above it the other tail is computed.
    pub(crate) symmetry_point: f64,
}

impl Beta {
    pub(crate) fn new(a: f64, b: f64) -> Self {
        let sum = a + b;
        Self {
            a,
            b,
            log_normalization: sum.ln_gamma().0 - a.ln_gamma().0 - b.ln_gamma().0,
            symmetry_point: (a + 1.0) / (sum + 2.0),
        }
    }

    /// The distribution of `1 - x`.
    pub(crate) fn reflected(self) -> Self {
        Self {
            a: self.b,
            b: self.a,
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
        let front = self.front(x);
        let cdf = if x < self.symmetry_point {
            front * continued_fraction(self.a, self.b, x) / self.a
        } else {
            1.0 - front * continued_fraction(self.b, self.a, 1.0 - x) / self.b
        };
        (cdf, front)
    }

    fn front(&self, x: f64) -> f64 {
        (self.log_normalization + self.a * x.ln() + self.b * (1.0 - x).ln()).exp()
    }
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
