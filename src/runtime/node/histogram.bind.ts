import * as b from "../../codegen/bindgen-lib.ts";

export default b.module({ rustPath: "crate::node::histogram" });

/** What the members of a class that Node.js writes in JavaScript are. */
const nodeClass = { construct: "illegal", configurable: true, enumerable: false } as const;
const bigInt = { getter: b.u64.asBigInt } as const;
const percentile = {
  args: { percentile: b.f64 },
  expected: { percentile: ["number"] },
  capability: "heap",
  throws: true,
} as const;
const atValue = { args: { value: b.f64 }, expected: { value: ["number"] }, returns: b.f64 } as const;
/** It can make the counts grow, which it tells the GC of the object. */
const grows = { passThis: true, capability: "heap" } as const;

export const HistogramJSON = b.dictionary({
  count: b.u64,
  min: b.u64,
  max: b.u64,
  mean: b.f64,
  exceeds: b.u64,
  stddev: b.f64,
  skewness: b.f64,
  kurtosis: b.f64,
  ewmaMean: b.f64,
  ewmaStddev: b.f64,
  ewmaErrorRate: b.f64,
  percentiles: b.any,
});

/** https://github.com/nodejs/node/blob/v26.3.0/lib/internal/histogram.js */
export const Histogram = b.class({
  ...nodeClass,
  rustPath: "crate::node::histogram::Histogram",
  final: false,
  estimatedSize: true,
  proto: {
    count: { getter: b.u64 },
    countBigInt: { ...bigInt, rustName: "count" },
    min: { getter: b.u64 },
    minBigInt: { ...bigInt, rustName: "min" },
    max: { getter: b.u64 },
    maxBigInt: { ...bigInt, rustName: "max" },
    mean: { getter: b.f64 },
    exceeds: { getter: b.u64 },
    exceedsBigInt: { ...bigInt, rustName: "exceeds" },
    stddev: { getter: b.f64 },
    skewness: { getter: b.f64 },
    kurtosis: { getter: b.f64 },
    /** 0 unless the histogram was created with a `halfLife`. */
    ewmaMean: { getter: b.f64 },
    ewmaStddev: { getter: b.f64 },
    /** 0 unless the histogram was created with a `halfLife` and a `threshold`. */
    ewmaErrorRate: { getter: b.f64 },
    /** How many times faster than `1 - sloTarget` allows the values pass the threshold. */
    burnRate: {
      args: { sloTarget: b.f64 },
      expected: { sloTarget: ["number"] },
      returns: b.f64,
      capability: "heap",
      throws: true,
    },
    percentile: { ...percentile, returns: b.u64 },
    percentileBigInt: { ...percentile, returns: b.u64.asBigInt, rustName: "percentile" },
    /** A `Map`. */
    percentiles: { getter: b.any, capability: "js", throws: true },
    percentilesBigInt: { getter: b.any, capability: "js", throws: true },
    /** A `Map` from each of `percentiles`, in order, to the value at it. */
    percentilesAt: {
      args: { percentiles: b.array(b.f64) },
      expected: { percentiles: ["Array"] },
      returns: b.any,
      capability: "js",
      throws: true,
    },
    /** A `Map` from the lowest value of the slot at each multiple of `stepSize` to the values up to that slot. */
    linearBuckets: { args: { stepSize: b.any }, returns: b.any, capability: "js", throws: true },
    /** A `Map` as `linearBuckets()` makes, at `firstBucket` times each power of `base`. */
    logBuckets: { args: { firstBucket: b.any, base: b.any }, returns: b.any, capability: "js", throws: true },
    countAt: atValue,
    cdf: atValue,
    ccdf: atValue,
    /** How many times `reset()` or `subtract()` removed values. */
    resetCount: { getter: b.u64 },
    /** A `Histogram` with the values recorded so far, which recording more does not change. */
    snapshot: { returns: b.self, capability: "heap" },
    /** A `Histogram` with the values recorded since `other` was a snapshot of this one. */
    diff: {
      args: { other: b.self },
      expected: { other: ["Histogram"] },
      returns: b.self,
      capability: "heap",
      throws: true,
    },
    reset: {},
    toJSON: { returns: HistogramJSON, capability: "heap", throws: true },
    "@@inspectCustom": { builtin: true, length: 2 },
  },
});

export const RecordableHistogram = b.class({
  ...nodeClass,
  extends: Histogram,
  proto: {
    record: { args: { val: b.any }, length: 1, passThis: true, capability: "js", throws: true },
    recordCorrected: {
      ...grows,
      args: { val: b.any, expectedInterval: b.any },
      length: 2,
      capability: "js",
      throws: true,
    },
    recordDelta: grows,
    add: { ...grows, args: { other: b.self }, expected: { other: ["RecordableHistogram"] } },
    subtract: { args: { other: b.self }, expected: { other: ["RecordableHistogram"] } },
  },
});

/** https://github.com/nodejs/node/blob/v26.3.0/lib/internal/perf/event_loop_delay.js */
export const ELDHistogram = b.class({
  ...nodeClass,
  extends: Histogram,
  proto: {
    /** Whether it was disabled. */
    enable: { returns: b.bool },
    /** Whether it was enabled. */
    disable: { returns: b.bool },
    "@@dispose": { builtin: true, length: 0 },
  },
});

/** `null` if no histogram has this range and precision. */
export const createRecordable = b.fn({
  args: { lowest: b.u64.allowBigInt, highest: b.u64.allowBigInt, figures: b.u32, halfLife: b.f64, threshold: b.f64 },
  returns: RecordableHistogram.nullable,
  capability: "heap",
});
export const createInterval = b.fn({ args: { resolution: b.f64 }, returns: ELDHistogram, capability: "heap" });
export const isHistogram = b.fn({ args: { value: b.any }, returns: b.bool });
