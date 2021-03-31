#![allow(dead_code)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(unused_assignments)]
#![allow(unused_mut)]

extern crate libc;
use std::convert::TryInto;
use std::ffi::c_void;
use thiserror::Error;

pub(crate) mod raw;

const BLOCK_SIZE: usize = 64000;
const MAX_BLOCK_COMPRESS_SIZE: usize = BLOCK_SIZE + (BLOCK_SIZE / 16) + 64 + 3;

#[derive(Error, Debug)]
pub enum MiniLzoError {
    #[error("Header expected but not found.")]
    NoHeader,
    #[error("Invalid header format.")]
    InvalidHeader,
    #[error("LZO initialization failed: {code:?}")]
    LzoInit { code: i8 },
    #[error("LZO de/compression error: {code:?}")]
    LzoError { code: i8 },
}

// Required by LZO to be called before operations
fn init() -> Result<(), MiniLzoError> {
    let r = unsafe { raw::lzo_initialize() };
    if r != 0 {
        Err(MiniLzoError::LzoInit { code: r as i8 })
    } else {
        Ok(())
    }
}

/// Calculate the max compressed len of an input.
pub fn max_compress_len(input_len: usize) -> usize {
    // ref: docs/LZO.FAQ
    input_len + (input_len / 16) + 64 + 3
}

/// Convenience function to handle vec allocation for decompression, taking into account the header.
///
/// ### Note
/// Will always fail if the input does not contain a header; if this is your case, you'll need
/// to pre-allocate a vec of appropriate length for decompression output and use [decompress](fn.decompress.html)
/// directly.
pub fn decompress_vec(input: &[u8]) -> Result<Vec<u8>, MiniLzoError> {
    if [0xf0, 0xf1].contains(&input[0]) {
        let length_bytes: [u8; 4] = input[1..5]
            .try_into()
            .map_err(|_| MiniLzoError::InvalidHeader)?;
        let mut output = vec![0; u32::from_be_bytes(length_bytes) as usize];
        let n = decompress(&input[5..], output.as_mut_slice())?;
        output.truncate(n);
        Ok(output)
    } else {
        Err(MiniLzoError::NoHeader)
    }
}

/// Decompress input into output. Will ignore any header if present in the input.
pub fn decompress(input: &[u8], output: &mut [u8]) -> Result<usize, MiniLzoError> {
    init()?;

    // Determine if there is a header
    let input_buf = if [0xf0, 0xf1].contains(&input[0]) {
        &input[5..]
    } else {
        &input[..]
    };

    let (n_bytes_written, _n_bytes_consumed) = unsafe {
        let mut wrkmem: [u8; 0] = std::mem::MaybeUninit::uninit().assume_init();
        let mut out_len: u32 = 0;
        let (r, n_consumed_bytes) = raw::lzo1x_decompress_safe(
            input_buf.as_ptr(),
            input_buf.len() as u64,
            output.as_mut_ptr(),
            &out_len as *const _ as *mut _,
            wrkmem.as_mut_ptr() as *mut c_void,
        );
        if r != 0 {
            return Err(MiniLzoError::LzoError { code: r as i8 });
        }
        (out_len, n_consumed_bytes)
    };
    Ok(n_bytes_written as usize)
}

/// Compress input into output buffer, optionally with a header written to the front of the output
/// buffer.
pub fn compress(input: &[u8], output: &mut [u8], header: bool) -> Result<usize, MiniLzoError> {
    init()?;

    let mut out_len = 0;
    let mut out = if header {
        &mut output[5..]
    } else {
        &mut output[..]
    };
    let r = unsafe {
        let mut wrkmem: [u8; 64000] = std::mem::MaybeUninit::uninit().assume_init();
        raw::lzo1x_1_compress(
            input.as_ptr(),
            input.len() as u64,
            out.as_mut_ptr(),
            &out_len as *const _ as *mut _,
            wrkmem.as_mut_ptr() as *mut c_void,
        )
    };
    if r != 0 {
        return Err(MiniLzoError::LzoError { code: r as i8 });
    }
    if header {
        output[0] = 0xf0;
        output[1..5].copy_from_slice(&(input.len() as u32).to_be_bytes());
        out_len += 5;
    }
    Ok(out_len as usize)
}

/// Convenience function to compress input into an appropriately sized output buffer, optionally
/// with a header.
pub fn compress_vec(input: &[u8], header: bool) -> Result<Vec<u8>, MiniLzoError> {
    let mut output = vec![0; max_compress_len(input.len())];
    let n = compress(input, output.as_mut_slice(), header)?;
    output.truncate(n);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use crate::{compress, decompress, max_compress_len, compress_vec, decompress_vec};

    fn gen_data() -> Vec<u8> {
        (0..100000)
            .map(|_| b"Oh what a beautiful day, oh what a beaitufl morning!!!".to_vec())
            .flat_map(|v| v)
            .collect::<Vec<u8>>()
    }

    #[test]
    fn roundtrip_slices() {
        let input = gen_data();

        let mut compressed = vec![0; max_compress_len(input.len())];
        let n_bytes = compress(&input, compressed.as_mut_slice(), true).unwrap();

        let mut decompressed: Vec<u8> = vec![0; input.len()];
        let n_bytes = decompress(&compressed[..n_bytes], decompressed.as_mut_slice()).unwrap();

        assert_eq!(&decompressed[..n_bytes], input.as_slice());
    }

    #[test]
    fn rountrip_vecs_with_header() {
        let input = gen_data();
        let compressed = compress_vec(input.as_slice(), true).unwrap();
        let decompressed = decompress_vec(compressed.as_slice()).unwrap();
        assert_eq!(decompressed, input);
    }
    #[test]
    fn rountrip_vecs_without_header() {
        let input = gen_data();
        let compressed = compress_vec(input.as_slice(), false).unwrap();
        let decompressed = decompress_vec(compressed.as_slice());
        assert!(decompressed.is_err())  // decompress_vec needs to have a header.
    }
}
