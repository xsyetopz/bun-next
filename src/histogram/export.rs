//! `histogram.export()`: the counts and the moving averages as a CBOR (RFC 8949) map with the
//! keys and the layout of the counts that Node.js gives those of HdrHistogram.
// https://github.com/nodejs/node/blob/v26.13.1/src/histogram.cc#L1570

use crate::{Ewma, Histogram};

const UINT: u8 = 0;
const ARRAY: u8 = 4 << 5;
const MAP: u8 = 5 << 5;
const FLOAT64: u8 = 0xfb;
const INT64_MAX: u64 = i64::MAX as u64;

/// What `export` writes.
const VERSION: u64 = 2;

const KEY_VERSION: u64 = 0;
const KEY_LOWEST: u64 = 1;
const KEY_HIGHEST: u64 = 2;
const KEY_FIGURES: u64 = 3;
const KEY_TOTAL_COUNT: u64 = 4;
const KEY_MIN: u64 = 5;
const KEY_MAX: u64 = 6;
const KEY_NORMALIZING_OFFSET: u64 = 7;
const KEY_CONVERSION_RATIO: u64 = 8;
const KEY_COUNTS_LEN: u64 = 9;
const KEY_COUNTS: u64 = 10;
const KEY_EWMA: u64 = 11;
/// The keys of the EWMA map below this are numbers: alpha, mean, variance and error rate.
const EWMA_KEY_THRESHOLD: u64 = 4;

fn write_head(out: &mut Vec<u8>, major: u8, value: u64) {
    let (info, width) = match value {
        0..=23 => (value as u8, 0),
        24..=0xff => (24, 1),
        0x100..=0xffff => (25, 2),
        0x1_0000..=0xffff_ffff => (26, 4),
        _ => (27, 8),
    };
    out.push(major | info);
    out.extend_from_slice(value.to_be_bytes().get(8 - width..).unwrap_or_default());
}

fn write_f64(out: &mut Vec<u8>, value: f64) {
    out.push(FLOAT64);
    out.extend_from_slice(&value.to_bits().to_be_bytes());
}

impl Histogram {
    /// How many slots HdrHistogram allocates for this range and precision.
    fn counts_len(&self) -> u64 {
        let mut untrackable = 2u64 << (self.sub_bucket_half_count_magnitude + self.unit_magnitude);
        let mut buckets = 1;
        while untrackable <= self.highest_trackable_value {
            buckets += 1;
            if untrackable > INT64_MAX / 2 {
                break;
            }
            untrackable <<= 1;
        }
        (buckets + 1) << self.sub_bucket_half_count_magnitude
    }

    /// The counts and `ewma` in the format that Node.js writes.
    ///
    /// Node.js: writes the lowest and the highest value recorded, where this writes the lowest and the highest value
    /// of the slots that they are in. Those are what Node.js reports as the minimum and the maximum of either.
    pub fn export(&self, ewma: &Ewma) -> Vec<u8> {
        let mut out = Vec::new();
        let fields = [
            (KEY_VERSION, VERSION),
            (KEY_LOWEST, self.lowest_discernible_value),
            (KEY_HIGHEST, self.highest_trackable_value),
            (KEY_FIGURES, u64::from(self.significant_figures)),
            (KEY_TOTAL_COUNT, self.count),
            (KEY_MIN, self.min()),
            (KEY_MAX, self.max()),
            (KEY_NORMALIZING_OFFSET, 0),
        ];
        write_head(&mut out, MAP, if ewma.alpha > 0.0 { 12 } else { 11 });
        for (key, value) in fields {
            write_head(&mut out, UINT, key);
            write_head(&mut out, UINT, value);
        }
        write_head(&mut out, UINT, KEY_CONVERSION_RATIO);
        write_f64(&mut out, 1.0);
        write_head(&mut out, UINT, KEY_COUNTS_LEN);
        write_head(&mut out, UINT, self.counts_len());
        write_head(&mut out, UINT, KEY_COUNTS);
        write_head(&mut out, ARRAY, 2 * self.counts().count() as u64);
        let mut previous = 0;
        for (index, n) in self.counts() {
            write_head(&mut out, UINT, (index - previous) as u64);
            write_head(&mut out, UINT, n);
            previous = index;
        }
        if ewma.alpha > 0.0 {
            write_head(&mut out, UINT, KEY_EWMA);
            write_head(&mut out, MAP, 5);
            for (value, key) in [ewma.alpha, ewma.mean, ewma.variance, ewma.error_rate]
                .into_iter()
                .zip(0..)
            {
                write_head(&mut out, UINT, key);
                write_f64(&mut out, value);
            }
            write_head(&mut out, UINT, EWMA_KEY_THRESHOLD);
            write_head(&mut out, UINT, ewma.threshold as u64);
        }
        out
    }
}
