//! `createSlidingWindowHistogram()`: a ring of histograms, each of the values of a period of time or of a number of
//! values, of which `snapshot()` adds up those in the window.
// https://github.com/nodejs/node/blob/v26.13.1/src/histogram.cc#L2345

use core::cell::RefCell;
use core::sync::atomic::{AtomicUsize, Ordering};

use bun_core::{Timespec, TimespecMockMode};
use bun_jsc::bindgen::{self, JsHeap, JsLocal, JsPayload as _, JsScope, JsWrapper};
use bun_jsc::{ErrorCode, MAX_SAFE_INTEGER};

use super::{Histogram, big_int_in_range, new_histogram, new_state};
use crate::node::util::validators;

pub(crate) struct SlidingWindowHistogram {
    window: RefCell<Window>,
    /// The bytes of the chunks. An atomic because a marking thread reads it.
    size: AtomicUsize,
}

struct Window {
    /// Empty, of the range and precision of every chunk.
    empty: bun_histogram::Histogram,
    /// `generation % chunks.len()` is the index of the chunk of a generation.
    chunks: Box<[Option<Chunk>]>,
    rotation: Rotation,
}

struct Chunk {
    generation: u64,
    histogram: bun_histogram::Histogram,
}

enum Rotation {
    /// The generation is the number of whole periods since `origin_ns`.
    Time { origin_ns: u64, period_ns: u64 },
    /// The generation grows by one every `per_chunk` values, recorded or not. It is `None` until a value comes.
    Count {
        per_chunk: u64,
        generation: Option<u64>,
        in_chunk: u64,
    },
}

fn now_ns() -> u64 {
    Timespec::now(TimespecMockMode::ForceRealTime).ns()
}

impl Rotation {
    fn reset(&mut self) {
        match self {
            Rotation::Time { origin_ns, .. } => *origin_ns = now_ns(),
            Rotation::Count {
                generation,
                in_chunk,
                ..
            } => {
                *generation = None;
                *in_chunk = 0;
            }
        }
    }
}

pub(crate) fn create_sliding_window<'a>(
    heap: &JsHeap<'a>,
    lowest: u64,
    highest: u64,
    figures: u32,
    chunks: u32,
    time_based: bool,
    rotate_at: u64,
) -> bindgen::Result<'a, JsWrapper<'a, SlidingWindowHistogram>> {
    if chunks == 0 {
        return Err(heap.error_with_code(
            ErrorCode::OUT_OF_RANGE,
            format_args!("options.chunks is out of range"),
        ));
    }
    if rotate_at == 0 {
        return Err(heap.error_with_code(
            ErrorCode::OUT_OF_RANGE,
            format_args!("rotation interval is out of range"),
        ));
    }
    let empty = new_histogram(heap, lowest, highest, figures)?;
    let rotation = if time_based {
        Rotation::Time {
            origin_ns: now_ns(),
            period_ns: rotate_at,
        }
    } else {
        Rotation::Count {
            per_chunk: rotate_at,
            generation: None,
            in_chunk: 0,
        }
    };
    let window = Window {
        empty,
        chunks: (0..chunks).map(|_| None).collect(),
        rotation,
    };
    let histogram = SlidingWindowHistogram {
        window: RefCell::new(window),
        size: AtomicUsize::new(0),
    };
    Ok(histogram.into_js(heap))
}

impl Window {
    fn record(&mut self, value: u64) {
        let generation = match &mut self.rotation {
            Rotation::Time {
                origin_ns,
                period_ns,
            } => now_ns().saturating_sub(*origin_ns) / *period_ns,
            Rotation::Count {
                per_chunk,
                generation,
                in_chunk,
            } => {
                let current = generation.unwrap_or(0);
                let next = if generation.is_some() && in_chunk == per_chunk {
                    *in_chunk = 0;
                    current + 1
                } else {
                    current
                };
                *generation = Some(next);
                *in_chunk += 1;
                next
            }
        };
        let len = self.chunks.len() as u64;
        let chunk = &mut self.chunks[(generation % len) as usize];
        match chunk {
            Some(chunk) if chunk.generation == generation => {}
            Some(chunk) => {
                chunk.generation = generation;
                chunk.histogram.reset();
            }
            None => {
                *chunk = Some(Chunk {
                    generation,
                    histogram: self.empty.clone(),
                })
            }
        }
        if let Some(chunk) = chunk {
            chunk.histogram.record(value);
        }
    }

    fn snapshot(&self) -> bun_histogram::Histogram {
        let mut snapshot = self.empty.clone();
        let current = match self.rotation {
            Rotation::Time {
                origin_ns,
                period_ns,
            } => now_ns().saturating_sub(origin_ns) / period_ns,
            Rotation::Count {
                generation: Some(generation),
                ..
            } => generation,
            Rotation::Count {
                generation: None, ..
            } => return snapshot,
        };
        let len = self.chunks.len() as u64;
        for chunk in self.chunks.iter().flatten() {
            if chunk.generation <= current && current - chunk.generation < len {
                snapshot.add(&chunk.histogram);
            }
        }
        snapshot
    }

    fn memory_size(&self) -> usize {
        let chunks = self.chunks.iter().flatten();
        chunks.map(|chunk| chunk.histogram.memory_size()).sum()
    }
}

impl SlidingWindowHistogram {
    pub(crate) fn estimated_size(&self) -> usize {
        self.size.load(Ordering::Relaxed)
    }

    /// Returns the bytes that the chunks have grown by.
    fn update_size(&self, window: &Window) -> usize {
        let size = window.memory_size();
        size.saturating_sub(self.size.swap(size, Ordering::Relaxed))
    }

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
        let grown = {
            let mut window = self.window.borrow_mut();
            window.record(value);
            self.update_size(&window)
        };
        if grown > 0 {
            // This can collect, so no borrow spans it.
            cx.heap.report_extra_memory(this_value.as_value(), grown);
        }
        Ok(())
    }

    pub(crate) fn snapshot<'a>(&self, heap: &JsHeap<'a>) -> JsWrapper<'a, Histogram> {
        let snapshot = self.window.borrow().snapshot();
        let (state, _) = new_state(snapshot, Default::default());
        Histogram::detached(state).into_js(heap)
    }

    pub(crate) fn reset(&self) {
        let mut window = self.window.borrow_mut();
        window.chunks.iter_mut().for_each(|chunk| *chunk = None);
        window.rotation.reset();
        self.update_size(&window);
    }
}
