// SPDX-License-Identifier: MIT
//! tRAM codec — núcleo de compresión para slots de swap/RAM.
//!
//! Pipeline (v1, determinista, sin alloc en el caliente):
//!   1. Capa LZ estilo LZ4: matches (offset+longitud) + literales
//!      (`lz::tokenize`).
//!   2. Traducción de valores: el alfabeto de *valores* de los literales se
//!      ordena por (count desc, valor asc); cada literal se mapea a un
//!      símbolo 0..K-1. Cabecera: K (1 B) + alfabeto (K B).
//!      Caso óptimo para páginas borradas/sólo-ceros: K==1 -> 0 bits payload.
//!   3. Huffman canónico sobre el alfabeto de símbolos (estático; longitudes
//!      ≤ C_MAX), que da ≈ la entropía del dato real (LZ4/LZO no hacen
//!      entropía: emiten los literales a 8 bits).
//!   4. Si el resultado no mejora, el slot se guarda RAW.
//!
//! Garantías: determinismo bit a bit, CRC16 (reflect 0x8005) por slot,
//! búferes acotados (`compress_bound`), `no_std` + `#![forbid(unsafe_code)]`.
//!
//! El paso "ternario" (planos de trits / packing AVX2) es una vía *futura*
//! sobre los planos de signo; v1 codifica entropía de forma correcta sobre el
//! alfabeto de valores, que es la cota que gana el gate de ratio frente a
//! LZ4/LZO (que no hacen entropía).

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

mod huff;
mod lz;
mod values;
/// Tamaño de la tabla hash de matches que el caller de `compress_into` debe
/// aportar (vive en el estado del dispositivo, nunca en la pila).
#[doc(hidden)]
pub use lz::HASH_TAB_LEN;

/// Formato del slot (10 B):
///   [0..2)  magic 0x54 0x52 ("TR")
///   [2]     version (1)
///   [3]     flags (bit0 = store_raw)
///   [4..6)  input_len (u16 LE)
///   [6..8)  output_len (u16 LE)
///   [8..10) crc16 reflect 0x8005 sobre el payload
///   [10..)  payload: RAW (flag) o tokens-lz + sección de entropía.
pub(crate) const MAGIC: [u8; 2] = [0x54, 0x52];
pub(crate) const VERSION: u8 = 1;
pub(crate) const FLAG_RAW: u8 = 1 << 0;
pub(crate) const HEADER_LEN: usize = 10;

pub const MAX_INPUT_LEN: usize = 0xFFFF;
/// Máxima longitud de código Huffman del formato (patológicos -> RAW).
pub const C_MAX: usize = huff::C_MAX_BITS;

/// Cota del slot comprimido para una entrada de `n` bytes.
pub fn compress_bound(input_len: usize) -> usize {
    debug_assert!(input_len <= MAX_INPUT_LEN);
    // tokens ≤ n+2 (instrucciones de literal de 1 B + fin), bits ≤ n*C_MAX/8,
    // sección de entropía (K+2K+1) ≤ 513, cabecera 10, margen 16.
    HEADER_LEN + (input_len + 2) + (input_len * C_MAX) / 8 + 513 + 16
}

/// Compresión. `dst` y `scratch` deben tener `compress_bound(n)`; `ht` debe
/// tener `lz::HASH_TAB_LEN` entradas (el caller aporta su tabla; en el kernel
/// vive en el estado del dispositivo, nunca en la pila).
pub fn compress_into(
    src: &[u8],
    dst: &mut [u8],
    scratch: &mut [u8],
    ht: &mut [u32],
) -> Result<usize, CodecError> {
    let n = src.len();
    if n > MAX_INPUT_LEN {
        return Err(CodecError::Buffer);
    }
    if dst.len() < compress_bound(n) || scratch.len() < compress_bound(n) {
        return Err(CodecError::Buffer);
    }
    if n == 0 {
        let crc = crc16(&[]);
        write_header(dst, FLAG_RAW, 0, 0, crc);
        return Ok(HEADER_LEN);
    }

    // 1) LZ: tokens en scratch[..t]; el resto acoge la entropía.
    let t = lz::tokenize(src, scratch, ht)?;
    debug_assert!(ht.len() >= lz::HASH_TAB_LEN, "ht debe tener lz::HASH_TAB_LEN");

    // 2)+3) valores + Huffman en scratch[t..].
    let (tok_part, ent_part) = scratch.split_at_mut(t);
    let used = match build_entropy(src, tok_part, ent_part) {
        Ok(v) => v,
        Err(_) => usize::MAX, // promueve a RAW
    };
    let total = if used != usize::MAX { t + used } else { usize::MAX };

    // 4) RAW fallback si no mejora.
    let (flags, plen) = if total != usize::MAX && total < HEADER_LEN + n {
        (0, total)
    } else {
        (FLAG_RAW, n)
    };

    if flags & FLAG_RAW == 0 {
        dst[HEADER_LEN..HEADER_LEN + plen].copy_from_slice(&scratch[..total]);
    } else {
        dst[HEADER_LEN..HEADER_LEN + plen].copy_from_slice(src);
    }
    let crc = crc16(&dst[HEADER_LEN..HEADER_LEN + plen]);
    write_header(dst, flags, n, plen, crc);
    Ok(HEADER_LEN + plen)
}

/// Construye la sección de entropía en `dst` (tras los tokens): K+alfabeto+
/// longitudes + bitstream de los símbolos. Devuelve bytes usados.
fn build_entropy(src: &[u8], tok: &[u8], dst: &mut [u8]) -> Result<usize, CodecError> {
    // Recuento de valores de los literales (una pasada sobre los tokens).
    let mut counts = [0u32; 256];
    lz::fold_literals(src, tok, |v| counts[v as usize] += 1)?;
    let lit_total = lz::literal_total(tok)?;

    let (k0, alphabet, v2s) = values::alphabet(&counts);
    let k = k0.max(1); // página sin literales (solo matches): 1 símbolo "fantasma"

    let tbl = huff::code_tables(&counts, k, &alphabet);
    if tbl.over_cap {
        return Err(CodecError::Buffer); // histograma patológico -> RAW
    }

    // Cabecera de entropía: K | alfabeto | longitudes.
    let hh = 1 + k + k;
    let bits_cap = ((lit_total * huff::C_MAX_BITS + 7) / 8) + 2;
    if dst.len() < hh + bits_cap {
        return Err(CodecError::Buffer);
    }
    dst[0] = k as u8;
    dst[1..1 + k].copy_from_slice(&alphabet[..k]);
    dst[1 + k..1 + 2 * k].copy_from_slice(&tbl.lengths[..k]);

    if k == 1 || lit_total == 0 {
        return Ok(hh); // sin bits
    }

    // Bitstream de los símbolos (MSB-first).
    let mut bw = huff::BitWriter::new(&mut dst[hh..]);
    let n = lz::fold_literals(src, tok, |v| {
        let sym = v2s[v as usize] as usize;
        bw.write(tbl.codes[sym], tbl.lengths[sym] as usize);
    });
    debug_assert_eq!(n?, lit_total);
    Ok(hh + bw.finish())
}

/// Descompresión. `scratch` debe tener `compress_bound(input_len)`.
pub fn decompress_into(
    slot: &[u8],
    dst: &mut [u8],
    scratch: &mut [u8],
) -> Result<usize, CodecError> {
    if slot.len() < HEADER_LEN {
        return Err(CodecError::Slot);
    }
    if slot[0] != MAGIC[0] || slot[1] != MAGIC[1] || slot[2] != VERSION {
        return Err(CodecError::Slot);
    }
    let flags = slot[3];
    let input_len = u16::from_le_bytes([slot[4], slot[5]]) as usize;
    let output_len = u16::from_le_bytes([slot[6], slot[7]]) as usize;
    let crc = u16::from_le_bytes([slot[8], slot[9]]);
    if dst.len() < input_len {
        return Err(CodecError::Buffer);
    }
    if HEADER_LEN + output_len > slot.len() {
        return Err(CodecError::Slot);
    }
    let payload = &slot[HEADER_LEN..HEADER_LEN + output_len];
    if crc16(payload) != crc {
        return Err(CodecError::Crc);
    }
    if flags & FLAG_RAW != 0 {
        if output_len != input_len {
            return Err(CodecError::Slot);
        }
        dst[..input_len].copy_from_slice(payload);
        return Ok(input_len);
    }
    restore_payload(payload, dst, scratch, input_len)
}

/// Reconstruye la página desde tokens+entropía.
fn restore_payload(payload: &[u8], dst: &mut [u8], scratch: &mut [u8], expected: usize) -> Result<usize, CodecError> {
    // 1) Separar tokens (fin marcado estructuralmente).
    let te = lz::token_end(payload).ok_or(CodecError::Slot)?;
    let tok = &payload[..te];
    let ent = &payload[te..];

    // 2) Cabecera de entropía.
    let k = ent[0] as usize;
    if k == 0 || k > 256 || ent.len() < 1 + 2 * k {
        return Err(CodecError::Slot);
    }
    let alphabet = &ent[1..1 + k];
    let lengths = &ent[1 + k..1 + 2 * k];

    // 3) Decodificar el bitstream a los valores de literales (en orden).
    let n_lit = lz::literal_total(tok)?;
    if scratch.len() < n_lit {
        return Err(CodecError::Buffer);
    }
    if k == 1 {
        scratch[..n_lit].fill(alphabet[0]);
    } else {
        let mut dt = huff::decode_table(k, lengths).ok_or(CodecError::Slot)?;
        let mut br = huff::BitReader::new(&ent[1 + 2 * k..]);
        let mut lit_pos = 0usize;
        lz::walk(tok, &mut |tk| {
            if let lz::Tk::Lit(nn) = tk {
                // Decodificar nn símbolos: cada símbolo consume `len` bits (más de un `read_bit`).
                let mut got = 0usize;
                while got < nn {
                    let bit = br.read_bit().ok_or(CodecError::Slot)?;
                    if let Some(sym) = dt.decode_bit(bit).map_err(|_| CodecError::Slot)? {
                        scratch[lit_pos] = alphabet[sym];
                        lit_pos += 1;
                        got += 1;
                    }
                }
            }
            Ok(())
        })?;
        if lit_pos != n_lit {
            return Err(CodecError::Slot);
        }
    }

    // 4) Emisión: re-recórrer los tokens empalmando literales y matches.
    let mut lit_pos = 0usize;
    let mut out_pos = 0usize;
    lz::walk(tok, &mut |tk| match tk {
        lz::Tk::Lit(nn) => {
            let end = lit_pos + nn;
            if end > scratch.len() || out_pos + nn > dst.len() {
                return Err(CodecError::Slot);
            }
            dst[out_pos..out_pos + nn].copy_from_slice(&scratch[lit_pos..end]);
            lit_pos = end;
            out_pos += nn;
            Ok(())
        }
        lz::Tk::Mat { off, len } => {
            if off > out_pos || out_pos + len > expected {
                return Err(CodecError::Slot);
            }
            // Copia byte a byte hacia delante: permite matches solapados
            // (off < len, RLE-like), igual que en LZ4/Huffman.
            let src_pos = out_pos - off;
            for i in 0..len {
                dst[out_pos + i] = dst[src_pos + i];
            }
            out_pos += len;
            Ok(())
        }
    })?;
    if out_pos != expected {
        return Err(CodecError::Slot);
    }
    Ok(out_pos)
}

fn write_header(dst: &mut [u8], flags: u8, input_len: usize, payload_len: usize, crc: u16) {
    dst[0] = MAGIC[0];
    dst[1] = MAGIC[1];
    dst[2] = VERSION;
    dst[3] = flags;
    dst[4] = (input_len & 0xFF) as u8;
    dst[5] = ((input_len >> 8) & 0xFF) as u8;
    dst[6] = (payload_len & 0xFF) as u8;
    dst[7] = ((payload_len >> 8) & 0xFF) as u8;
    dst[8] = (crc & 0xFF) as u8;
    dst[9] = ((crc >> 8) & 0xFF) as u8;
}

pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= b as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xA001 } else { crc >> 1 };
        }
    }
    crc
}

// -- wrappers con alloc (userspace: tram-bench / tram-ctl / tests) ----------
#[cfg(feature = "alloc")]
pub fn compress(src: &[u8]) -> Result<alloc::vec::Vec<u8>, CodecError> {
    let bound = compress_bound(src.len());
    let mut dst = alloc::vec![0u8; bound];
    let mut scratch = alloc::vec![0u8; bound];
    let mut ht = alloc::vec![0u32; lz::HASH_TAB_LEN];
    let n = compress_into(src, &mut dst, &mut scratch, &mut ht)?;
    dst.truncate(n);
    Ok(dst)
}

#[cfg(feature = "alloc")]
pub fn decompress(slot: &[u8]) -> Result<alloc::vec::Vec<u8>, CodecError> {
    if slot.len() < HEADER_LEN {
        return Err(CodecError::Slot);
    }
    let input_len = u16::from_le_bytes([slot[4], slot[5]]) as usize;
    let mut dst = alloc::vec![0u8; input_len];
    let mut scratch = alloc::vec![0u8; compress_bound(input_len)];
    let n = decompress_into(slot, &mut dst, &mut scratch)?;
    dst.truncate(n);
    Ok(dst)
}

/// Bits por trit en el límite de Shannon (log2 3 ≈ 1.585).
pub const BITS_PER_TRIT: f64 = 1.584_962_500_721_156;
/// Trits necesarios para representar un byte (3^6 = 729 ≥ 256); constante de
/// nomenclatura para la fase AVX2 (planos de signo), no del payload v1.
pub const TRITS_PER_BYTE: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecError {
    Buffer,
    Slot,
    Crc,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(data: &[u8]) {
        let slot = crate::compress(data).expect("compress");
        let out = crate::decompress(&slot).expect("decompress");
        assert_eq!(out, data, "roundtrip no bit-exacto");
    }

    #[test]
    fn decompress_into_buffer_mayor_que_input() {
        // El patrón kernel/swap: el buffer de salida es la página de RAM,
        // típicamente más grande que `input_len`. El gate de restauración
        // compara contra el tamaño *original*, no contra `dst.len()`.
        let bound = crate::compress_bound(4096);
        let mut dst = vec![0u8; bound];
        let mut scratch = vec![0u8; bound];
        let mut ht = vec![0u32; crate::HASH_TAB_LEN];
        let src = vec![0u8; 4096];
        let n = crate::compress_into(&src, &mut dst, &mut scratch, &mut ht).expect("compress");
        let mut big = vec![0u8; 8192];
        let mut sc2 = vec![0u8; bound];
        let m = crate::decompress_into(&dst[..n], &mut big, &mut sc2).expect("decompress");
        assert_eq!(m, 4096);
        assert_eq!(&big[..4096], &src[..]);
    }

    #[test]
    fn roundtrip_empty() {
        roundtrip(&[]);
    }

    #[test]
    fn roundtrip_zeros_4k() {
        roundtrip(&[0u8; 4096]);
    }

    #[test]
    fn roundtrip_ones_4k() {
        roundtrip(&[0xABu8; 4096]);
    }

    #[test]
    fn roundtrip_ascii() {
        let mut d = Vec::new();
        for i in 0..8192u32 {
            d.push(b'a' + (i % 26) as u8);
        }
        roundtrip(&d);
    }

    #[test]
    fn roundtrip_random() {
        let mut x = 0x1234_5678u64;
        let mut d = Vec::new();
        d.reserve(4096);
        for _ in 0..4096 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            d.push((x >> 32) as u8);
        }
        roundtrip(&d);
    }

    #[test]
    fn roundtrip_structured() {
        // entradas tipo swap: bloques repetidos con ruido
        let mut d = Vec::new();
        for i in 0..4096 {
            let base = (i / 64) as u8;
            d.push(base.wrapping_add((i as usize).wrapping_mul(7) as u8 & 0x03));
        }
        roundtrip(&d);
    }

    #[test]
    fn determinismo_bit_a_bit() {
        let data = b"hola mundo holamundo, esto es una prueba de determinismo".repeat(4);
        let a = crate::compress(&data).unwrap();
        let b = crate::compress(&data).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn slot_raw_no_empeora() {
        // incompresible: el slot nunca es más grande que la entrada+cabecera.
        let mut x = 0xDEADBEEFu64;
        let mut d = Vec::new();
        for _ in 0..4096 {
            x = x.wrapping_mul(0x5851F42D4C957F2D).wrapping_add(1);
            d.push((x >> 16) as u8);
        }
        let slot = crate::compress(&d).unwrap();
        assert!(slot.len() <= d.len() + HEADER_LEN + 1);
    }

    #[test]
    fn crc_detecta_corrupcion() {
        let d = b"datos con redundancia para detectar".repeat(8);
        let mut slot = crate::compress(&d).unwrap();
        let n = slot.len();
        if n > HEADER_LEN + 1 {
            slot[n - 1] ^= 0x40;
            assert_eq!(crate::decompress(&slot), Err(CodecError::Crc));
        }
    }
}
