//! `histogram.export()` and `importHistogram()`: the counts and the moving averages as a CBOR (RFC 8949) map with the
//! keys and the layout of the counts that Node.js gives those of HdrHistogram.
// https://github.com/nodejs/node/blob/v26.13.1/src/histogram.cc#L1570

use crate::{Ewma, Histogram};

const UINT: u8 = 0;
const ARRAY: u8 = 4 << 5;
const MAP: u8 = 5 << 5;
const FLOAT64: u8 = 0xfb;
const INT64_MAX: u64 = i64::MAX as u64;
const INT32_MAX: u64 = i32::MAX as u64;

/// What `export` writes. Version 1 has the same layout, but its importer refuses keys that it does not know.
const VERSION: u64 = 2;
const STRICT_VERSION: u64 = 1;

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
/// How deep the value of an unknown key can nest.
const MAX_SKIP_DEPTH: u32 = 16;

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

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: u64) -> Option<&'a [u8]> {
        let (taken, rest) = self.bytes.split_at_checked(usize::try_from(n).ok()?)?;
        self.bytes = rest;
        Some(taken)
    }

    /// The major type, in the top 3 bits, and the argument of the next item. Indefinite lengths are refused.
    fn head(&mut self) -> Option<(u8, u64)> {
        let &[initial] = self.take(1)? else {
            return None;
        };
        let argument = match initial & 0x1f {
            info @ 0..=23 => u64::from(info),
            info @ 24..=27 => {
                let bytes = self.take(1 << (info - 24))?;
                bytes
                    .iter()
                    .fold(0, |value, &byte| value << 8 | u64::from(byte))
            }
            _ => return None,
        };
        Some((initial & 0xe0, argument))
    }

    fn argument(&mut self, major: u8) -> Option<u64> {
        let (actual, argument) = self.head()?;
        (actual == major).then_some(argument)
    }

    fn uint(&mut self, max: u64) -> Option<u64> {
        self.argument(UINT).filter(|&value| value <= max)
    }

    /// A float64 or an unsigned integer.
    fn number(&mut self) -> Option<f64> {
        if self.bytes.first() != Some(&FLOAT64) {
            return Some(self.argument(UINT)? as f64);
        }
        let (_, bits) = self.head()?;
        Some(f64::from_bits(bits))
    }

    /// A count of items that each take a byte or more, so the count cannot be larger than what is left.
    fn length(&mut self, major: u8) -> Option<u64> {
        self.argument(major)
            .filter(|&n| n <= self.bytes.len() as u64)
    }

    fn skip(&mut self, depth: u32) -> Option<()> {
        let &initial = self.bytes.first()?;
        if depth > MAX_SKIP_DEPTH {
            return None;
        }
        if initial >> 5 == 7 {
            // Simple values and floats: their argument is not a length.
            let extra = match initial & 0x1f {
                0..=23 => 0,
                info @ 24..=27 => 1 << (info - 24),
                _ => return None,
            };
            self.take(1 + extra)?;
            return Some(());
        }
        let (major, argument) = self.head()?;
        match major >> 5 {
            2 | 3 => {
                self.take(argument)?;
            }
            4 | 5 if argument > self.bytes.len() as u64 => return None,
            4 => (0..argument).try_for_each(|_| self.skip(depth + 1))?,
            5 => (0..argument * 2).try_for_each(|_| self.skip(depth + 1))?,
            6 => self.skip(depth + 1)?,
            _ => {}
        }
        Some(())
    }
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

    /// What [`Self::export`] or Node.js wrote, or `None` if `data` is not that or holds no histogram.
    ///
    /// Node.js: keeps a minimum and a maximum that `data` gives, even if no slot in use holds them.
    pub fn import(data: &[u8]) -> Option<(Self, Ewma)> {
        let mut reader = Reader { bytes: data };
        let (mut lowest, mut highest, mut figures) = (1, INT64_MAX, 3);
        let (mut total_count, mut counts_len, mut normalizing_offset) = (None, 0, 0);
        let mut version = STRICT_VERSION;
        let mut unknown_keys = false;
        let (mut seen, mut seen_ewma) = (0u64, 0u64);
        let mut counts = Vec::new();
        let mut ewma = [0.0; 4];
        let mut threshold = 0;
        let first_time = |seen: &mut u64, key: u64| {
            let bit: u64 = 1 << key;
            core::mem::replace(seen, *seen | bit) & bit == 0
        };
        for _ in 0..reader.argument(MAP)? {
            let key = reader.argument(UINT)?;
            if key > KEY_EWMA {
                reader.skip(0)?;
                unknown_keys = true;
                continue;
            }
            if !first_time(&mut seen, key) {
                return None;
            }
            match key {
                KEY_VERSION => version = reader.uint(VERSION).filter(|&v| v >= STRICT_VERSION)?,
                KEY_LOWEST => lowest = reader.uint(INT64_MAX)?,
                KEY_HIGHEST => highest = reader.uint(INT64_MAX)?,
                KEY_FIGURES => figures = reader.uint(INT32_MAX)?,
                KEY_TOTAL_COUNT => total_count = Some(reader.uint(INT64_MAX)?),
                KEY_MIN | KEY_MAX => {
                    reader.uint(INT64_MAX)?;
                }
                KEY_NORMALIZING_OFFSET => normalizing_offset = reader.uint(INT32_MAX)?,
                KEY_CONVERSION_RATIO => {
                    reader.number()?;
                }
                KEY_COUNTS_LEN => counts_len = reader.uint(INT32_MAX)?,
                KEY_COUNTS => {
                    let len = reader.length(ARRAY).filter(|len| len % 2 == 0)?;
                    let mut index = 0;
                    for pair in 0..len / 2 {
                        let delta = reader
                            .uint(INT32_MAX)
                            .filter(|&delta| pair == 0 || delta > 0)?;
                        let n = reader.uint(INT64_MAX)?;
                        index = Some(index + delta).filter(|&index| index <= INT32_MAX)?;
                        counts.push((index, n));
                    }
                }
                _ => {
                    for _ in 0..reader.argument(MAP)? {
                        let key = reader.argument(UINT)?;
                        if key > EWMA_KEY_THRESHOLD {
                            reader.skip(0)?;
                            unknown_keys = true;
                        } else if !first_time(&mut seen_ewma, key) {
                            return None;
                        } else if let Some(value) = ewma.get_mut(key as usize) {
                            *value = reader.number()?;
                        } else {
                            threshold = reader.uint(INT64_MAX)?;
                        }
                    }
                }
            }
        }
        if version == STRICT_VERSION && unknown_keys {
            return None;
        }
        let mut histogram = Self::new(lowest, highest, u32::try_from(figures).ok()?)?;
        if histogram.counts_len() != counts_len || normalizing_offset >= counts_len {
            return None;
        }
        let mut total = 0u64;
        for &(index, n) in &counts {
            total = total
                .checked_add(n)
                .filter(|&total| total <= INT64_MAX && index < counts_len)?;
        }
        if total_count.is_some_and(|count| count != total) {
            return None;
        }
        for (index, n) in counts {
            histogram.add_at(
                usize::try_from((index + normalizing_offset) % counts_len).ok()?,
                n,
            );
        }
        let [alpha, mean, variance, error_rate] = ewma;
        let half_life = if alpha > 0.0 && alpha < 1.0 {
            -1.0 / (1.0 - alpha).log2()
        } else {
            0.0
        };
        let mut ewma = Ewma::new(half_life, threshold as i64);
        if alpha > 0.0 {
            ewma.restore(mean, variance, error_rate);
        }
        Some((histogram, ewma))
    }
}
