//! The exponentially weighted moving averages that `createHistogram({ halfLife, threshold })` keeps beside the
//! counts, in the order that the values were recorded.

/// Off unless it was created with a half-life. Then every value moves the mean towards itself by `alpha`, and with
/// a threshold too, the share of values above the threshold moves the same way.
#[derive(Clone, Copy, Default)]
pub struct Ewma {
    /// Between 0 and 1, or 0 if this is off.
    pub(crate) alpha: f64,
    /// 0 if the error rate is not tracked.
    pub(crate) threshold: i64,
    /// Whether a value was recorded since the last reset.
    started: bool,
    pub(crate) mean: f64,
    pub(crate) variance: f64,
    pub(crate) error_rate: f64,
}

impl Ewma {
    /// A value counts half as much as one recorded `half_life` values after it. Off if `half_life` is not above 0.
    pub fn new(half_life: f64, threshold: i64) -> Self {
        let alpha = if half_life > 0.0 {
            1.0 - (-core::f64::consts::LN_2 / half_life).exp()
        } else {
            0.0
        };
        Self {
            alpha,
            threshold,
            ..Self::default()
        }
    }

    pub fn record(&mut self, value: u64) {
        if self.alpha <= 0.0 {
            return;
        }
        let value = value as f64;
        let error = if value > self.threshold as f64 {
            1.0
        } else {
            0.0
        };
        if !self.started {
            *self = Self {
                started: true,
                mean: value,
                variance: 0.0,
                error_rate: if self.threshold > 0 { error } else { 0.0 },
                ..*self
            };
            return;
        }
        let diff = value - self.mean;
        self.mean += self.alpha * diff;
        self.variance = (1.0 - self.alpha) * (self.variance + self.alpha * diff * diff);
        if self.threshold > 0 {
            self.error_rate += self.alpha * (error - self.error_rate);
        }
    }

    /// Continues from the averages of values recorded before.
    pub(crate) fn restore(&mut self, mean: f64, variance: f64, error_rate: f64) {
        *self = Self {
            started: true,
            mean,
            variance,
            error_rate,
            ..*self
        };
    }

    /// Forgets the values, and keeps the half-life and the threshold.
    pub fn reset(&mut self) {
        *self = Self {
            alpha: self.alpha,
            threshold: self.threshold,
            ..Self::default()
        };
    }

    /// 0 if this is off or no value was recorded.
    pub fn mean(&self) -> f64 {
        self.mean
    }

    /// 0 if this is off or no value was recorded.
    pub fn stddev(&self) -> f64 {
        self.variance.sqrt()
    }

    /// The share of the values above the threshold. 0 if this is off, there is no threshold or no value was recorded.
    pub fn error_rate(&self) -> f64 {
        self.error_rate
    }
}
