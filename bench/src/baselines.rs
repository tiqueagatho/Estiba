// SPDX-License-Identifier: MIT
//! Implementaciones del trait `Codec` usadas por estiba-bench: el codec propio
//! (`Estiba`) y los baselines LZ4 / LZO1X cargados en runtime por `dlopen`
//! contra `liblz4.so.1` / `liblzo2.so.2` del sistema (sin dependencias de
//! build ni headers: los prototipos se declaran aquí mismo).

use std::os::raw::{c_char, c_int, c_void};

const RTLD_LAZY: c_int = 1;
const LZO_E_OK: c_int = 0;

unsafe extern "C" {
    fn dlopen(file: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

unsafe fn load<T>(handle: *mut c_void, name: &str) -> Option<T> {
    let mut sym = Vec::with_capacity(name.len() + 1);
    sym.extend_from_slice(name.as_bytes());
    sym.push(0);
    let p = dlsym(handle, sym.as_ptr().cast());
    if p.is_null() {
        None
    } else {
        Some(std::mem::transmute_copy::<*mut c_void, T>(&p))
    }
}

/// El codec de estiba (lazy bindings del crate; los buffers se reutilizan para
/// no contar el alloc como parte del throughput).
pub struct Estiba {
    dst: Vec<u8>,
    scratch: Vec<u8>,
    ht: Vec<u32>,
}

impl Estiba {
    pub fn new() -> Self {
        let bound = estiba_codec::compress_bound(estiba_codec::MAX_INPUT_LEN);
        Estiba {
            dst: vec![0u8; bound],
            scratch: vec![0u8; bound],
            ht: vec![0u32; estiba_codec::HASH_TAB_LEN],
        }
    }
}

/// Contrato común: `out` se vacía y recibe el resultado en cada llamada.
pub trait Codec {
    fn name(&self) -> &str;
    fn compress_into(&mut self, src: &[u8], out: &mut Vec<u8>) -> Result<(), ()>;
    fn decompress_into(&mut self, src: &[u8], expected: usize, out: &mut Vec<u8>)
        -> Result<(), ()>;
}

impl Codec for Estiba {
    fn name(&self) -> &str {
        "estiba (LZ+Huf)"
    }
    fn compress_into(&mut self, src: &[u8], out: &mut Vec<u8>) -> Result<(), ()> {
        let n = estiba_codec::compress_into(src, &mut self.dst, &mut self.scratch, &mut self.ht)
            .map_err(|_| ())?;
        out.clear();
        out.extend_from_slice(&self.dst[..n]);
        Ok(())
    }
    fn decompress_into(
        &mut self,
        src: &[u8],
        _expected: usize,
        out: &mut Vec<u8>,
    ) -> Result<(), ()> {
        let n =
            estiba_codec::decompress_into(src, &mut self.dst, &mut self.scratch).map_err(|_| ())?;
        out.clear();
        out.extend_from_slice(&self.dst[..n]);
        Ok(())
    }
}

// -- LZ4 ---------------------------------------------------------------------

type Lz4Compress =
    extern "C" fn(src: *const u8, dst: *mut u8, src_size: c_int, dst_cap: c_int) -> c_int;
type Lz4Decompress =
    extern "C" fn(src: *const u8, dst: *mut u8, comp: c_int, dst_cap: c_int) -> c_int;

pub struct Lz4 {
    comp: Lz4Compress,
    decomp: Lz4Decompress,
    buf: Vec<u8>,
}

impl Lz4 {
    pub fn new() -> Option<Lz4> {
        unsafe {
            let h = dlopen(b"liblz4.so.1\0".as_ptr().cast(), RTLD_LAZY);
            if h.is_null() {
                return None;
            }
            let comp = load::<Lz4Compress>(h, "LZ4_compress_default")?;
            let decomp = load::<Lz4Decompress>(h, "LZ4_decompress_safe")?;
            Some(Lz4 {
                comp,
                decomp,
                buf: Vec::new(),
            })
        }
    }
}

impl Codec for Lz4 {
    fn name(&self) -> &str {
        "LZ4 1.10"
    }
    fn compress_into(&mut self, src: &[u8], out: &mut Vec<u8>) -> Result<(), ()> {
        let cap = src.len() + src.len() / 255 + 16;
        self.buf.resize(cap, 0);
        let n = (self.comp)(
            src.as_ptr(),
            self.buf.as_mut_ptr(),
            src.len() as c_int,
            self.buf.len() as c_int,
        );
        if n <= 0 {
            return Err(());
        }
        out.clear();
        out.extend_from_slice(&self.buf[..n as usize]);
        Ok(())
    }
    fn decompress_into(
        &mut self,
        src: &[u8],
        expected: usize,
        out: &mut Vec<u8>,
    ) -> Result<(), ()> {
        self.buf.resize(expected, 0);
        let n = (self.decomp)(
            src.as_ptr(),
            self.buf.as_mut_ptr(),
            src.len() as c_int,
            self.buf.len() as c_int,
        );
        if n != expected as c_int {
            return Err(());
        }
        out.clear();
        out.extend_from_slice(&self.buf[..expected]);
        Ok(())
    }
}

// -- LZO1X -------------------------------------------------------------------

type LzoCompress = extern "C" fn(
    src: *const u8,
    src_len: usize,
    dst: *mut u8,
    dst_len: *mut usize,
    wrkmem: *mut u8,
) -> c_int;
type LzoDecompress = extern "C" fn(
    src: *const u8,
    src_len: usize,
    dst: *mut u8,
    dst_len: *mut usize,
    wrkmem: *mut u8,
) -> c_int;

pub struct Lzo1 {
    comp: LzoCompress,
    decomp: LzoDecompress,
    buf: Vec<u8>,
    wrkmem: Vec<u8>,
}

impl Lzo1 {
    pub fn new() -> Option<Lzo1> {
        unsafe {
            let h = dlopen(b"liblzo2.so.2\0".as_ptr().cast(), RTLD_LAZY);
            if h.is_null() {
                return None;
            }
            let comp = load::<LzoCompress>(h, "lzo1x_1_compress")?;
            let decomp = load::<LzoDecompress>(h, "lzo1x_decompress_safe")?;
            // LZO1X_1_MEM_COMPRESS ≈ 64 KiB; damos holgura para el wrkmem.
            Some(Lzo1 {
                comp,
                decomp,
                buf: Vec::new(),
                wrkmem: vec![0u8; 1 << 20],
            })
        }
    }
}

impl Codec for Lzo1 {
    fn name(&self) -> &str {
        "LZO1X 2.10"
    }
    fn compress_into(&mut self, src: &[u8], out: &mut Vec<u8>) -> Result<(), ()> {
        let cap = src.len() + src.len() / 16 + 64 + 3;
        self.buf.resize(cap, 0);
        let mut out_len = self.buf.len();
        let rc = (self.comp)(
            src.as_ptr(),
            src.len(),
            self.buf.as_mut_ptr(),
            &mut out_len,
            self.wrkmem.as_mut_ptr(),
        );
        if rc != LZO_E_OK {
            return Err(());
        }
        out.clear();
        out.extend_from_slice(&self.buf[..out_len]);
        Ok(())
    }
    fn decompress_into(
        &mut self,
        src: &[u8],
        expected: usize,
        out: &mut Vec<u8>,
    ) -> Result<(), ()> {
        self.buf.resize(expected, 0);
        let mut out_len = self.buf.len();
        let rc = (self.decomp)(
            src.as_ptr(),
            src.len(),
            self.buf.as_mut_ptr(),
            &mut out_len,
            self.wrkmem.as_mut_ptr(),
        );
        if rc != LZO_E_OK || out_len != expected {
            return Err(());
        }
        out.clear();
        out.extend_from_slice(&self.buf[..expected]);
        Ok(())
    }
}
