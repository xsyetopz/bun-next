//! RFC 8879 certificate compression for the TLS contexts that `certificateCompression` configures.
//! BoringSSL calls these through the trampolines in `packages/bun-usockets/src/crypto/openssl.cpp`.

use core::ffi::{c_int, c_void};
use core::mem::MaybeUninit;

use bun_core::vec::{BorrowedBuf, append_with};

const ZLIB: u8 = 1;
const BROTLI: u8 = 2;
const ZSTD: u8 = 3;

fn compress(algorithm: u8, mut input: &[u8], out: &mut Vec<u8>) -> bool {
    match algorithm {
        ZLIB => {
            use bun_zlib::deflate::{Container, Deflater, Flush, Options};
            Deflater::new(Container::Zlib, Options::default())
                .and_then(|mut deflater| deflater.deflate_to_vec(input, out, Flush::Finish))
                .is_ok()
        }
        BROTLI => {
            use bun_brotli::{EncodeStatus, Encoder, MAX_QUALITY, Operation};
            let bound = bun_brotli::max_compressed_size(input.len());
            if out.try_reserve_exact(bound).is_err() {
                return false;
            }
            let mut encoder = Encoder::with_quality(MAX_QUALITY);
            append_with(out, bound, |output| {
                encoder.compress(Operation::Finish, &mut input, output)
            }) == Ok(EncodeStatus::Done)
        }
        ZSTD => {
            let bound = bun_zstd::compress_bound(input.len());
            !bun_zstd::is_error(bound)
                && out.try_reserve_exact(bound).is_ok()
                && matches!(
                    bun_zstd::compress_append(out, input, None),
                    bun_zstd::Result::Success(_)
                )
        }
        _ => false,
    }
}

/// The peer announces the uncompressed length, so anything that does not fill `out` exactly is rejected.
fn decompress(algorithm: u8, mut input: &[u8], out: &mut [MaybeUninit<u8>]) -> bool {
    let expected = out.len();
    match algorithm {
        ZLIB => {
            let mut buf = BorrowedBuf::from(out);
            let mut inflater = bun_zlib::Inflater::new(bun_zlib::inflate::Container::Zlib);
            inflater.inflate(&mut input, buf.unfilled())
                == Ok(bun_zlib::inflate::Status::StreamEnd)
                && input.is_empty()
                && buf.len() == expected
        }
        BROTLI => {
            let mut buf = BorrowedBuf::from(out);
            let mut decoder = bun_brotli::Decoder::default();
            decoder.decompress(&mut input, buf.unfilled()) == Ok(bun_brotli::DecodeStatus::Finished)
                && input.is_empty()
                && buf.len() == expected
        }
        ZSTD => bun_zstd::decompress_exact(out, input),
        _ => false,
    }
}

/// Compresses `input` and hands the result to `append(sink, bytes, len)`; returns 1 on success.
///
/// # Safety
/// `input` is valid for `input_len` bytes, and `append` accepts `sink`.
#[unsafe(no_mangle)]
pub(crate) unsafe extern "C" fn bun_cert_compress(
    algorithm: u8,
    input: *const u8,
    input_len: usize,
    sink: *mut c_void,
    append: unsafe extern "C" fn(*mut c_void, *const u8, usize) -> c_int,
) -> c_int {
    // SAFETY: the caller guarantees `input` is valid for `input_len` bytes.
    let input = unsafe { bytes(input, input_len) };
    let mut out = Vec::new();
    if !compress(algorithm, input, &mut out) {
        return 0;
    }
    // SAFETY: the caller guarantees `append` accepts `sink`; `out` is valid for its length.
    unsafe { append(sink, out.as_ptr(), out.len()) }
}

/// Decompresses `input` into exactly `out_len` bytes at `out`; returns 1 on success.
///
/// # Safety
/// `input` is valid for `input_len` bytes, and `out` is writable for `out_len` bytes.
#[unsafe(no_mangle)]
pub(crate) unsafe extern "C" fn bun_cert_decompress(
    algorithm: u8,
    input: *const u8,
    input_len: usize,
    out: *mut u8,
    out_len: usize,
) -> c_int {
    // SAFETY: the caller guarantees both regions are valid for their lengths.
    let (input, out) = unsafe {
        (
            bytes(input, input_len),
            core::slice::from_raw_parts_mut(out.cast::<MaybeUninit<u8>>(), out_len),
        )
    };
    c_int::from(decompress(algorithm, input, out))
}

/// # Safety
/// `ptr` is valid for `len` bytes unless `len` is 0.
unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if len == 0 {
        return &[];
    }
    // SAFETY: guaranteed by the caller.
    unsafe { core::slice::from_raw_parts(ptr, len) }
}
