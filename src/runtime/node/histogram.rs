//! The histogram classes of `node:perf_hooks`, which `createHistogram()` and `monitorEventLoopDelay()` return.

use core::cell::{Cell, RefCell};
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use bun_core::fmt::{NodeNumber, OutOfRangeOptions, double, out_of_range};
use bun_core::{Timespec, TimespecMockMode};
use bun_jsc::bindgen::{self, JsHeap, JsLocal, JsPayload as _, JsScope, JsWrapper, to_js};
use bun_jsc::{ErrorCode, JSGlobalObject, JSMap, JSValue, JsResult, MAX_SAFE_INTEGER};
use bun_threading::Guarded;

use crate::generated_bindgen::histogram::{
    ConfidenceInterval, ConfidenceOptions, HistogramJSON, MannWhitneyTest, MeanCI, PercentileCI,
    WelchTest, js_ELDHistogram, js_RecordableHistogram,
};
use crate::jsc_hooks::timer_all;
use crate::node::util::validators;

/// The GC is told what JavaScriptCore tells it of a `SharedArrayBuffer`, which also grows and which a structured clone
/// also shares with other threads. Every object, a clone too, counts the whole size (`JSArrayBuffer::finishCreation`):
/// bindgen does that, from `estimated_size`. Growth counts in the thread that grew it (`ArrayBuffer::grow`).
pub(crate) struct Histogram {
    counts: RefCell<Counts>,
    size: Arc<Size>,
    /// Of an `ELDHistogram`, with whether it is enabled.
    interval: Option<(Rc<Interval>, Cell<bool>)>,
}

/// The bytes of the counts. An atomic because a marking thread reads it. It is only a hint to the GC, and nothing is
/// read in the light of it, so every access is relaxed.
struct Size(AtomicUsize);

enum Counts {
    /// Only this object has them, so `record()` takes no lock.
    Own(State),
    /// A structured clone, which can be on another thread, or a timer has them too.
    Shared(Arc<Guarded<State>>),
}

#[derive(Clone)]
struct State {
    histogram: bun_histogram::Histogram,
    /// When `record_delta` last ran, or 0. `recordDelta()` and `Interval::sample` read different
    /// clocks, and no histogram has both.
    previous_ns: u64,
    /// How many times values were removed.
    reset_count: u64,
    ewma: bun_histogram::Ewma,
}

/// An `ELDHistogram`, as the timer that samples the event loop sees it.
pub(crate) struct Interval {
    state: Arc<Guarded<State>>,
    size: Arc<Size>,
    /// Never 0.
    pub(crate) resolution_ms: u32,
}

impl State {
    /// Records the time since the last call, if there was one.
    fn record_delta(&mut self, now_ns: u64) {
        if self.previous_ns > 0 {
            self.record(now_ns.saturating_sub(self.previous_ns));
        }
        self.previous_ns = now_ns;
    }

    fn record(&mut self, value: u64) {
        if self.histogram.record(value) {
            self.ewma.record(value);
        }
    }

    fn add(&mut self, other: &State) {
        self.histogram.add(&other.histogram);
        self.previous_ns = self.previous_ns.max(other.previous_ns);
    }

    fn subtract(&mut self, other: &State) {
        self.histogram.subtract(&other.histogram);
        self.reset_count += 1;
    }
}

impl Interval {
    /// Returns the time of the sample, and the bytes that the counts have grown by.
    pub(crate) fn sample(&self) -> (Timespec, usize) {
        let now = Timespec::now(TimespecMockMode::ForceRealTime);
        let mut state = self.state.lock();
        state.record_delta(now.ns());
        (now, self.size.update(&state))
    }
}

impl Size {
    /// Returns the bytes that `state`, which the caller has exclusive access to, has grown by.
    fn update(&self, state: &State) -> usize {
        let size = state.histogram.memory_size();
        let before = self.0.load(Ordering::Relaxed);
        if size != before {
            self.0.store(size, Ordering::Relaxed);
        }
        size.saturating_sub(before)
    }
}

impl Histogram {
    pub(crate) fn estimated_size(&self) -> usize {
        self.size.0.load(Ordering::Relaxed)
    }

    /// Returns what `f` does, and the bytes that the counts have grown by. `f` does not reach another histogram.
    fn with<T>(&self, f: impl FnOnce(&mut State) -> T) -> (T, usize) {
        let with = |state: &mut State| (f(state), self.size.update(state));
        match &mut *self.counts.borrow_mut() {
            Counts::Own(state) => with(state),
            Counts::Shared(state) => with(&mut state.lock()),
        }
    }

    /// This can collect, so the caller has not borrowed the counts.
    fn report_growth<'a>(heap: &JsHeap<'a>, this_value: JsWrapper<'a, Self>, grown: usize) {
        if grown > 0 {
            heap.report_extra_memory(this_value.as_value(), grown);
        }
    }

    fn read<T>(&self, read: impl FnOnce(&bun_histogram::Histogram) -> T) -> T {
        self.read_state(|state| read(&state.histogram))
    }

    fn read_state<T>(&self, read: impl FnOnce(&State) -> T) -> T {
        match &*self.counts.borrow() {
            Counts::Own(state) => read(state),
            Counts::Shared(state) => read(&state.lock()),
        }
    }

    /// A `Histogram` that nothing else records into.
    fn detached(state: State) -> Self {
        Histogram {
            size: Arc::new(Size(AtomicUsize::new(state.histogram.memory_size()))),
            counts: RefCell::new(Counts::Own(state)),
            interval: None,
        }
    }

    fn share(&self) -> Arc<Guarded<State>> {
        let mut counts = self.counts.borrow_mut();
        let shared = match &mut *counts {
            Counts::Own(state) => Arc::new(Guarded::new(State {
                histogram: state.histogram.take(),
                ..*state
            })),
            Counts::Shared(state) => return Arc::clone(state),
        };
        *counts = Counts::Shared(Arc::clone(&shared));
        shared
    }
}

fn new_state(histogram: bun_histogram::Histogram, ewma: bun_histogram::Ewma) -> (State, Arc<Size>) {
    let size = Arc::new(Size(AtomicUsize::new(histogram.memory_size())));
    let state = State {
        histogram,
        previous_ns: 0,
        reset_count: 0,
        ewma,
    };
    (state, size)
}

pub(crate) fn create_recordable<'a>(
    heap: &JsHeap<'a>,
    lowest: u64,
    highest: u64,
    figures: u32,
    half_life: f64,
    threshold: f64,
) -> Option<JsWrapper<'a, Histogram>> {
    let histogram = bun_histogram::Histogram::new(lowest, highest, figures)?;
    // `as` saturates.
    let ewma = bun_histogram::Ewma::new(half_life, threshold as i64);
    let (state, size) = new_state(histogram, ewma);
    let histogram = Histogram {
        counts: RefCell::new(Counts::Own(state)),
        size,
        interval: None,
    };
    Some(js_RecordableHistogram::create(heap, histogram))
}

pub(crate) fn create_interval<'a>(heap: &JsHeap<'a>, resolution: f64) -> JsWrapper<'a, Histogram> {
    // https://github.com/nodejs/node/blob/v26.3.0/src/node_perf.cc#L298
    let histogram = bun_histogram::Histogram::new(1000, i64::MAX as u64, 3);
    let (state, size) = new_state(histogram.expect("a valid range"), Default::default());
    let state = Arc::new(Guarded::new(state));
    let interval = Rc::new(Interval {
        state: Arc::clone(&state),
        size: Arc::clone(&size),
        // `as` saturates.
        resolution_ms: (resolution as u32).max(1),
    });
    let histogram = Histogram {
        counts: RefCell::new(Counts::Shared(state)),
        size,
        interval: Some((interval, Cell::new(false))),
    };
    js_ELDHistogram::create(heap, histogram)
}

pub(crate) fn is_histogram(value: JsLocal<'_>) -> bool {
    value.to_js().is::<Histogram>()
}

/// `Histogram`
impl Histogram {
    pub(crate) fn get_count(&self) -> u64 {
        self.read(|histogram| histogram.count())
    }

    pub(crate) fn get_min(&self) -> u64 {
        self.read(|histogram| histogram.min())
    }

    pub(crate) fn get_max(&self) -> u64 {
        self.read(|histogram| histogram.max())
    }

    pub(crate) fn get_mean(&self) -> f64 {
        self.read(|histogram| histogram.mean())
    }

    pub(crate) fn get_exceeds(&self) -> u64 {
        self.read(|histogram| histogram.exceeds())
    }

    pub(crate) fn get_stddev(&self) -> f64 {
        self.read(|histogram| histogram.stddev())
    }

    pub(crate) fn get_skewness(&self) -> f64 {
        self.read(|histogram| histogram.skewness())
    }

    pub(crate) fn get_kurtosis(&self) -> f64 {
        self.read(|histogram| histogram.kurtosis())
    }

    pub(crate) fn get_ewma_mean(&self) -> f64 {
        self.read_state(|state| state.ewma.mean())
    }

    pub(crate) fn get_ewma_stddev(&self) -> f64 {
        self.read_state(|state| state.ewma.stddev())
    }

    pub(crate) fn get_ewma_error_rate(&self) -> f64 {
        self.read_state(|state| state.ewma.error_rate())
    }

    /// How many times faster than allowed the error budget of `slo_target`, a share of the values that are not
    /// above the threshold, is spent.
    pub(crate) fn burn_rate<'a>(
        &self,
        heap: &JsHeap<'a>,
        slo_target: f64,
    ) -> bindgen::Result<'a, f64> {
        check_range(heap, slo_target, b"sloTarget", b"> 0 && < 1", |t| {
            t > 0.0 && t < 1.0
        })?;
        Ok(self.get_ewma_error_rate() / (1.0 - slo_target))
    }

    fn percentiles_map<'a>(
        &self,
        cx: &mut JsScope<'a>,
        value_to_js: impl Fn(&JSGlobalObject, u64) -> JsResult<JSValue>,
    ) -> bindgen::Result<'a, JSValue> {
        let percentiles = self.read(|histogram| histogram.percentiles());
        number_map(cx, percentiles, value_to_js)
    }

    pub(crate) fn get_percentiles<'a>(&self, cx: &mut JsScope<'a>) -> bindgen::Result<'a, JSValue> {
        self.percentiles_map(cx, |_, value| Ok(JSValue::js_number_from_uint64(value)))
    }

    pub(crate) fn get_percentiles_big_int<'a>(
        &self,
        cx: &mut JsScope<'a>,
    ) -> bindgen::Result<'a, JSValue> {
        self.percentiles_map(cx, JSValue::from_uint64_no_truncate)
    }

    pub(crate) fn percentile<'a>(
        &self,
        heap: &JsHeap<'a>,
        percentile: f64,
    ) -> bindgen::Result<'a, u64> {
        check_range(heap, percentile, b"percentile", b"> 0 && <= 100", |p| {
            p > 0.0 && p <= 100.0
        })?;
        Ok(self.read(|histogram| histogram.value_at_percentile(percentile)))
    }

    pub(crate) fn mean_ci<'a>(
        &self,
        cx: &mut JsScope<'a>,
        options: JsLocal<'a>,
    ) -> bindgen::Result<'a, MeanCI> {
        let confidence = check_confidence(cx, options)?;
        let [mean, lower, upper] = self.read(|histogram| histogram.mean_ci(confidence));
        Ok(MeanCI { mean, lower, upper })
    }

    pub(crate) fn percentile_ci<'a>(
        &self,
        cx: &mut JsScope<'a>,
        percentile: f64,
        options: JsLocal<'a>,
    ) -> bindgen::Result<'a, PercentileCI> {
        check_range(&cx.heap, percentile, b"percentile", b"> 0 && <= 100", |p| {
            p > 0.0 && p <= 100.0
        })?;
        let confidence = check_confidence(cx, options)?;
        let [value, lower, upper] =
            self.read(|histogram| histogram.percentile_ci(percentile, confidence));
        Ok(PercentileCI {
            value,
            lower,
            upper,
        })
    }

    /// Node.js: 0 with a `p` of 1 if `other` is this histogram, unlike for a copy of it.
    pub(crate) fn welch_test<'a>(
        &self,
        cx: &mut JsScope<'a>,
        other: &Histogram,
        options: JsLocal<'a>,
    ) -> bindgen::Result<'a, WelchTest> {
        let confidence = check_confidence(cx, options)?;
        let test = self.compare(other, |this, other| this.welch_test(other, confidence));
        Ok(WelchTest {
            t_statistic: test.t,
            degrees_of_freedom: test.degrees_of_freedom,
            p_value: test.p,
            confidence_interval: ConfidenceInterval {
                lower: test.lower,
                upper: test.upper,
            },
        })
    }

    /// Node.js: 0 with a `p` of 1 if `other` is this histogram, unlike for a copy of it.
    pub(crate) fn mann_whitney_test(&self, other: &Histogram) -> MannWhitneyTest {
        let test = self.compare(other, bun_histogram::Histogram::mann_whitney_test);
        MannWhitneyTest {
            u_statistic: test.u,
            z_score: test.z,
            p_value: test.p,
        }
    }

    /// Node.js: 1 at every percentile if the histogram is empty.
    pub(crate) fn percentiles_at<'a>(
        &self,
        cx: &mut JsScope<'a>,
        mut percentiles: Vec<f64>,
    ) -> bindgen::Result<'a, JSValue> {
        for (index, &percentile) in percentiles.iter().enumerate() {
            if !(percentile > 0.0 && percentile <= 100.0) {
                let field_name = format!("percentiles[{index}]");
                let options = OutOfRangeOptions {
                    field_name: field_name.as_bytes(),
                    msg: b"> 0 && <= 100",
                    ..Default::default()
                };
                return Err(cx.heap.error_with_code(
                    ErrorCode::OUT_OF_RANGE,
                    format_args!("{}", out_of_range(NodeNumber(percentile), options)),
                ));
            }
        }
        percentiles.sort_by(f64::total_cmp);
        let values = self.read(|histogram| {
            percentiles
                .into_iter()
                .map(|percentile| (percentile, histogram.value_at_percentile(percentile)))
                .collect::<Vec<_>>()
        });
        number_map(cx, values, |_, value| {
            Ok(JSValue::js_number_from_uint64(value))
        })
    }

    pub(crate) fn linear_buckets<'a>(
        &self,
        cx: &mut JsScope<'a>,
        step_size: JsLocal<'a>,
    ) -> bindgen::Result<'a, JSValue> {
        let step = validators::validate_integer(
            cx.global(),
            step_size.to_js(),
            "stepSize",
            Some(1),
            None,
        )?;
        let buckets = self.read(|histogram| histogram.linear_buckets(step as u64));
        buckets_map(cx, buckets)
    }

    pub(crate) fn log_buckets<'a>(
        &self,
        cx: &mut JsScope<'a>,
        first_bucket: JsLocal<'a>,
        base: JsLocal<'a>,
    ) -> bindgen::Result<'a, JSValue> {
        let (first_bucket, base) = (first_bucket.to_js(), base.to_js());
        let global = cx.global();
        let first =
            validators::validate_integer(global, first_bucket, "firstBucket", Some(1), None)?;
        if !base.is_number() {
            Err(global.throw_invalid_argument_type_value("base", "number", base))?;
        }
        let base = base.as_number();
        if base <= 1.0 {
            let options = OutOfRangeOptions {
                field_name: b"base",
                msg: b"> 1",
                ..Default::default()
            };
            return Err(cx.heap.error_with_code(
                ErrorCode::OUT_OF_RANGE,
                format_args!("{}", out_of_range(NodeNumber(base), options)),
            ));
        }
        let buckets = self.read(|histogram| histogram.log_buckets(first as u64, base));
        buckets_map(cx, buckets)
    }

    /// Node.js: reads outside its counts for a negative value.
    pub(crate) fn count_at(&self, value: f64) -> f64 {
        if !(value >= 0.0) {
            return 0.0;
        }
        // `as` saturates.
        self.read(|histogram| histogram.count_at(value as u64)) as f64
    }

    /// A value below 0 is in the first slot.
    pub(crate) fn cdf(&self, value: f64) -> f64 {
        self.read(|histogram| histogram.cdf(value as u64))
    }

    pub(crate) fn ccdf(&self, value: f64) -> f64 {
        1.0 - self.cdf(value)
    }

    pub(crate) fn get_reset_count(&self) -> u64 {
        self.read_state(|state| state.reset_count)
    }

    pub(crate) fn snapshot<'a>(&self, heap: &JsHeap<'a>) -> JsWrapper<'a, Self> {
        Self::detached(self.read_state(State::clone)).into_js(heap)
    }

    pub(crate) fn ks_test(&self, other: &Histogram) -> f64 {
        self.compare(other, bun_histogram::Histogram::ks_test)
    }

    pub(crate) fn cohens_d(&self, other: &Histogram) -> f64 {
        self.compare(other, bun_histogram::Histogram::cohens_d)
    }

    pub(crate) fn cliffs_d(&self, other: &Histogram) -> f64 {
        self.compare(other, bun_histogram::Histogram::cliffs_d)
    }

    /// Compares a copy of `other`, so that this holds one lock at a time.
    fn compare<T>(
        &self,
        other: &Histogram,
        f: impl FnOnce(&bun_histogram::Histogram, &bun_histogram::Histogram) -> T,
    ) -> T {
        let other = other.read(Clone::clone);
        self.read(|this| f(this, &other))
    }

    /// What was recorded here since `other` was a snapshot of it.
    pub(crate) fn diff<'a>(
        &self,
        heap: &JsHeap<'a>,
        other: &Histogram,
    ) -> bindgen::Result<'a, JsWrapper<'a, Self>> {
        // https://github.com/nodejs/node/blob/v26.3.0/src/histogram.cc
        let this = self.read_state(State::clone);
        let diff = other.read_state(|other| {
            if !this.histogram.same_layout(&other.histogram) {
                return Err((
                    ErrorCode::INVALID_ARG_VALUE,
                    "other must have the same configuration as the histogram",
                ));
            }
            if this.reset_count != other.reset_count {
                return Err((
                    ErrorCode::INVALID_STATE,
                    "Values were removed from the histogram after other was taken",
                ));
            }
            this.histogram.diff(&other.histogram).ok_or((
                ErrorCode::INVALID_ARG_VALUE,
                "other contains values that are not in the histogram",
            ))
        });
        match diff {
            Ok(histogram) => {
                let state = State {
                    histogram,
                    previous_ns: 0,
                    reset_count: 0,
                    ewma: Default::default(),
                };
                Ok(Self::detached(state).into_js(heap))
            }
            Err((code, message)) => Err(heap.error_with_code(code, format_args!("{message}"))),
        }
    }

    pub(crate) fn reset(&self) {
        self.with(|state| {
            state.histogram.reset();
            state.ewma.reset();
            state.previous_ns = 0;
            state.reset_count += 1;
        });
    }

    pub(crate) fn to_json<'a>(&self, heap: &JsHeap<'a>) -> bindgen::Result<'a, HistogramJSON<'a>> {
        let (json, percentiles) = self.read_state(
            |State {
                 histogram, ewma, ..
             }| {
                let json = HistogramJSON {
                    count: histogram.count(),
                    min: histogram.min(),
                    max: histogram.max(),
                    mean: histogram.mean(),
                    exceeds: histogram.exceeds(),
                    stddev: histogram.stddev(),
                    skewness: histogram.skewness(),
                    kurtosis: histogram.kurtosis(),
                    ewma_mean: ewma.mean(),
                    ewma_stddev: ewma.stddev(),
                    ewma_error_rate: ewma.error_rate(),
                    percentiles: JsLocal::NULL,
                };
                (json, histogram.percentiles())
            },
        );
        let percentiles = percentiles
            .into_iter()
            .map(|(percentile, at)| (double(percentile).to_string(), at));
        Ok(HistogramJSON {
            percentiles: to_js::record(heap, percentiles, to_js::u64)?,
            ..json
        })
    }
}

/// `RecordableHistogram`
impl Histogram {
    pub(crate) fn record<'a>(
        &self,
        this_value: JsWrapper<'a, Self>,
        cx: &mut JsScope<'a>,
        val: JsLocal<'a>,
    ) -> bindgen::Result<'a, ()> {
        let val = val.to_js();
        let value = if val.is_big_int() {
            big_int_in_range(&cx.heap, val, 0, "value")?
        } else {
            validators::validate_integer(cx.global(), val, "val", Some(0), Some(MAX_SAFE_INTEGER))?
                as u64
        };
        let (_, grown) = self.with(|state| state.record(value));
        Self::report_growth(&cx.heap, this_value, grown);
        Ok(())
    }

    pub(crate) fn record_corrected<'a>(
        &self,
        this_value: JsWrapper<'a, Self>,
        cx: &mut JsScope<'a>,
        val: JsLocal<'a>,
        expected_interval: JsLocal<'a>,
    ) -> bindgen::Result<'a, ()> {
        let (val, interval) = (val.to_js(), expected_interval.to_js());
        let (value, interval) = if val.is_big_int() {
            if !interval.is_big_int() {
                let global = cx.global();
                Err(global.throw_invalid_argument_type_value(
                    "expectedInterval",
                    "bigint",
                    interval,
                ))?;
            }
            let value = big_int_in_range(&cx.heap, val, 0, "value")?;
            (
                value,
                big_int_in_range(&cx.heap, interval, 1, "expected_interval")?,
            )
        } else {
            let global = cx.global();
            let value = validators::validate_integer(global, val, "val", Some(0), None)?;
            let interval =
                validators::validate_integer(global, interval, "expectedInterval", Some(1), None)?;
            (value as u64, interval as u64)
        };
        let (_, grown) = self.with(|state| {
            if state.histogram.record_corrected(value, interval) {
                state.ewma.record(value);
            }
        });
        Self::report_growth(&cx.heap, this_value, grown);
        Ok(())
    }

    pub(crate) fn record_delta<'a>(&self, this_value: JsWrapper<'a, Self>, heap: &JsHeap<'a>) {
        unsafe extern "C" {
            /// `Timespec::now` has whole microseconds on macOS.
            safe fn uv_hrtime() -> u64;
        }
        let ((), grown) = self.with(|state| state.record_delta(uv_hrtime()));
        Self::report_growth(heap, this_value, grown);
    }

    pub(crate) fn add<'a>(
        &self,
        this_value: JsWrapper<'a, Self>,
        heap: &JsHeap<'a>,
        other: &Histogram,
    ) {
        let grown = self.with_other(other, State::add);
        Self::report_growth(heap, this_value, grown);
    }

    pub(crate) fn subtract(&self, other: &Histogram) {
        self.with_other(other, State::subtract);
    }

    /// Returns the bytes that the counts have grown by. `other` can be `self`.
    fn with_other(&self, other: &Histogram, f: impl Fn(&mut State, &State)) -> usize {
        let apply = |into: &mut State, other: &State| {
            f(into, other);
            self.size.update(into)
        };
        let apply_to_itself = |state: &mut State| {
            let copy = state.clone();
            apply(state, &copy)
        };
        if core::ptr::eq(self, other) {
            return match &mut *self.counts.borrow_mut() {
                Counts::Own(state) => apply_to_itself(state),
                Counts::Shared(state) => apply_to_itself(&mut state.lock()),
            };
        }
        match (&mut *self.counts.borrow_mut(), &*other.counts.borrow()) {
            (Counts::Own(this), Counts::Own(other)) => apply(this, other),
            (Counts::Own(this), Counts::Shared(other)) => apply(this, &other.lock()),
            (Counts::Shared(this), Counts::Own(other)) => apply(&mut this.lock(), other),
            (Counts::Shared(state), Counts::Shared(other)) if Arc::ptr_eq(state, other) => {
                apply_to_itself(&mut state.lock())
            }
            // In the order of their addresses: another thread may be combining the two the other way around.
            (Counts::Shared(this), Counts::Shared(other))
                if Arc::as_ptr(this) < Arc::as_ptr(other) =>
            {
                let mut this = this.lock();
                apply(&mut this, &other.lock())
            }
            (Counts::Shared(this), Counts::Shared(other)) => {
                let other = other.lock();
                apply(&mut this.lock(), &other)
            }
        }
    }
}

/// `ERR_OUT_OF_RANGE` unless `value` is `within`.
fn check_range<'a>(
    heap: &JsHeap<'a>,
    value: f64,
    field_name: &'static [u8],
    msg: &'static [u8],
    within: impl Fn(f64) -> bool,
) -> bindgen::Result<'a, ()> {
    if within(value) {
        return Ok(());
    }
    let options = OutOfRangeOptions {
        field_name,
        msg,
        ..Default::default()
    };
    Err(heap.error_with_code(
        ErrorCode::OUT_OF_RANGE,
        format_args!("{}", out_of_range(NodeNumber(value), options)),
    ))
}

/// `options.confidence`, validated as Node.js's `validateObject()` and `validateNumber()` do.
fn check_confidence<'a>(cx: &mut JsScope<'a>, options: JsLocal<'a>) -> bindgen::Result<'a, f64> {
    let options = options.to_js();
    let options = if options.is_undefined() {
        ConfidenceOptions::default()
    } else if !options.is_object() || options.is_array() || options.is_callable() {
        Err(cx
            .global()
            .throw_invalid_argument_type_value(b"options", b"object", options))?
    } else {
        ConfidenceOptions::from_js(cx.global(), options)?
    };
    let confidence = options.confidence;
    check_range(
        &cx.heap,
        confidence,
        b"options.confidence",
        b"> 0 && < 1",
        |c| c > 0.0 && c < 1.0,
    )?;
    Ok(confidence)
}

/// `value`, a BigInt, if it is at least `min` and fits in an `i64`.
fn big_int_in_range<'a>(
    heap: &JsHeap<'a>,
    value: JSValue,
    min: i64,
    name: &str,
) -> bindgen::Result<'a, u64> {
    if !value.is_big_int_in_int64_range(min, i64::MAX) {
        // https://github.com/nodejs/node/blob/v26.3.0/src/histogram.cc#L181
        return Err(heap.error_with_code(
            ErrorCode::OUT_OF_RANGE,
            format_args!("{name} is out of range"),
        ));
    }
    Ok(value.to_int64() as u64)
}

/// `ELDHistogram`
impl Histogram {
    /// Returns whether that changes it.
    fn set_enabled(&self, enable: bool) -> bool {
        let Some((interval, enabled)) = &self.interval else {
            return false;
        };
        if enabled.replace(enable) == enable {
            return false;
        }
        if enable {
            // The time it was disabled for is not a delay of the event loop.
            self.with(|state| state.previous_ns = 0);
            timer_all().event_loop_delay.enable(interval);
        } else {
            timer_all().event_loop_delay.disable(interval);
        }
        true
    }

    pub(crate) fn enable(&self) -> bool {
        self.set_enabled(true)
    }

    pub(crate) fn disable(&self) -> bool {
        self.set_enabled(false)
    }
}

/// The histograms in the structured clone messages that exist, on any thread, by what the message has in their place.
static IN_FLIGHT: Guarded<BTreeMap<u64, InFlight>> = Guarded::new(BTreeMap::new());

struct InFlight {
    state: Arc<Guarded<State>>,
    size: Arc<Size>,
    recordable: bool,
}

/// `histogram` is a `Histogram`.
#[unsafe(no_mangle)]
pub(crate) extern "C" fn Histogram__retainForStructuredClone(histogram: JSValue) -> u64 {
    let recordable = js_RecordableHistogram::from_js(&histogram).is_some();
    let Some(histogram) = histogram.as_class_ref::<Histogram>() else {
        return 0;
    };
    let in_flight = InFlight {
        state: histogram.share(),
        size: Arc::clone(&histogram.size),
        recordable,
    };
    // Not one that bytes from elsewhere (IPC, `v8.deserialize()`) can guess.
    let mut id = [0; 8];
    bun_boringssl_sys::rand_bytes(&mut id);
    let id = u64::from_ne_bytes(id);
    IN_FLIGHT.lock().insert(id, in_flight);
    id
}

/// `JSValue::ZERO` if no message has this id. A message can be read many times (`BroadcastChannel`).
#[unsafe(no_mangle)]
pub(crate) extern "C" fn Histogram__fromStructuredClone(
    global: &JSGlobalObject,
    id: u64,
) -> JSValue {
    let (histogram, recordable) = {
        let in_flight = IN_FLIGHT.lock();
        let Some(found) = in_flight.get(&id) else {
            return JSValue::ZERO;
        };
        let histogram = Histogram {
            counts: RefCell::new(Counts::Shared(Arc::clone(&found.state))),
            size: Arc::clone(&found.size),
            interval: None,
        };
        (histogram, found.recordable)
    };
    if recordable {
        js_RecordableHistogram::create_wrapper(global, histogram).to_js()
    } else {
        // The clone of an `ELDHistogram` is of neither subclass.
        histogram.into_js_wrapper(global).to_js()
    }
}

/// The message with this id is gone.
#[unsafe(no_mangle)]
pub(crate) extern "C" fn Histogram__releaseStructuredClone(id: u64) {
    IN_FLIGHT.lock().remove(&id);
}

fn number_map<'a>(
    cx: &mut JsScope<'a>,
    entries: impl IntoIterator<Item = (f64, u64)>,
    value_to_js: impl Fn(&JSGlobalObject, u64) -> JsResult<JSValue>,
) -> bindgen::Result<'a, JSValue> {
    let global = cx.global();
    let value = JSMap::create(global);
    if let Some(map) = JSMap::from_js(value) {
        let map = JSMap::opaque_mut(map.as_ptr());
        for (key, at) in entries {
            map.set(global, JSValue::js_number(key), value_to_js(global, at)?)?;
        }
    }
    Ok(value)
}

fn buckets_map<'a>(cx: &mut JsScope<'a>, buckets: Vec<(u64, u64)>) -> bindgen::Result<'a, JSValue> {
    let buckets = buckets.into_iter().map(|(at, count)| (at as f64, count));
    number_map(cx, buckets, |_, count| {
        Ok(JSValue::js_number_from_uint64(count))
    })
}
