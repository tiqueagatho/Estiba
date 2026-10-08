// SPDX-License-Identifier: MIT
//! Capa LZ (estilo LZ4): matches + literales.
//!
//! Formato del token stream (bytes, sin entropía):
//!   - `0xFF`                          -> fin
//!   - `0b0LLLLLLL` (1..=127)          -> una corrida de L literales (bytes de
//!                                        `src`; NO se copian al stream: los
//!                                        consume la capa de entropía).
//!   - `0b1MMMMMMM` (1..=127)          -> M matches consecutivos, seguidos de
//!                                        por cada uno: u16 LE (offset+1) y
//!                                        u8 (longitud-4).
//! El parseo es estructural (nunca se escanea en busca de `0xFF`), por lo que
//! los bytes 0xFF dentro de offsets son legales.

use super::CodecError;

/// Búsqueda del match: tabla hash de cadenas de 4 bytes (v1: 1 candidato).
pub const HASH_TAB_LEN: usize = 4096;
const HASH_BITS: u32 = 12; // 4096 entradas

pub const MIN_MATCH_LEN: usize = 4;
pub const MAX_MATCH_LEN: usize = 255;
/// Distancia máxima representable en u16 (+1).
pub const LZ_WINDOW: usize = 0xFFFE;

const TOK_LIT: u8 = 0x00; // máscara de bits
const TOK_MAT: u8 = 0x80;
const TOK_END: u8 = 0xFF;

/// Una instrucción del token stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tk {
    Lit(usize),
    Mat { off: usize, len: usize },
}

/// Cota de tokens para `n` bytes de entrada (≤ n instrucciones + END).
pub fn write_bound(n: usize) -> usize {
    n + 2
}

#[inline]
fn hash4(w: u32) -> u32 {
    // SplitMix/fnv mezcla: 4 bytes LE de `src`.
    w.wrapping_mul(0x9E37_79B9) >> (32 - HASH_BITS)
}

/// Greedy LZ4-ish. Escribe el token stream en `tok` y devuelve su longitud.
/// `ht` = tabla de 4096 u32 (vecinos de posición+1); el caller la aporta.
pub fn tokenize(src: &[u8], tok: &mut [u8], ht: &mut [u32]) -> Result<usize, CodecError> {
    let n = src.len();
    if tok.len() < write_bound(n) {
        return Err(CodecError::Buffer);
    }
    for slot in ht.iter_mut() {
        *slot = 0;
    }
    let mut w = 0usize; // cursor en `tok`
    let mut pos = 0usize;
    let mut lit_start = 0usize;

    macro_rules! put {
        ($b:expr) => {{
            tok[w] = $b;
            w += 1;
        }};
    }

    while pos + MIN_MATCH_LEN <= n {
        let w4 = u32::from_le_bytes([src[pos], src[pos + 1], src[pos + 2], src[pos + 3]]);
        let h = hash4(w4) as usize;
        let cand = ht[h]; // 0 = vacío; si no, posición+1
        ht[h] = (pos + 1) as u32;

        if cand != 0 {
            let c = cand as usize - 1;
            let off = pos - c;
            if off <= LZ_WINDOW {
                let mut ml = 0usize;
                while ml < MAX_MATCH_LEN && pos + ml < n && src[c + ml] == src[pos + ml] {
                    ml += 1;
                }
                if ml >= MIN_MATCH_LEN {
                    let lits_before = pos - lit_start;
                    if lits_before > 0 {
                        emit_literal_run(lits_before, &mut w, tok)?;
                    }
                    emit_match(off, ml, &mut w, tok)?;
                    pos += ml;
                    lit_start = pos;
                    continue;
                }
            }
        }
        pos += 1;
    }

    let lits_final = n - lit_start;
    if lits_final > 0 {
        emit_literal_run(lits_final, &mut w, tok)?;
    }
    put!(TOK_END);
    Ok(w)
}

fn emit_literal_run(len: usize, w: &mut usize, tok: &mut [u8]) -> Result<(), CodecError> {
    let mut rem = len;
    while rem > 0 {
        let chunk = rem.min(127);
        tok[*w] = TOK_LIT | (chunk as u8);
        *w += 1;
        rem -= chunk;
    }
    Ok(())
}

fn emit_match(off: usize, len: usize, w: &mut usize, tok: &mut [u8]) -> Result<(), CodecError> {
    // Un único match aquí (la emisión aglutinante de M por token es una
    // optimización de espacio; v1 emite uno por token con count=1).
    tok[*w] = TOK_MAT | 1;
    *w += 1;
    let o = (off + 1) as u16;
    tok[*w] = (o & 0xFF) as u8;
    tok[*w + 1] = ((o >> 8) & 0xFF) as u8;
    tok[*w + 2] = (len - MIN_MATCH_LEN) as u8;
    *w += 3;
    Ok(())
}

/// Recorre los tokens estructuralmente validándolos. `f` devuelve error si un
/// token es inconsistente con el estado del caller.
pub fn walk(tok: &[u8], f: &mut dyn FnMut(Tk) -> Result<(), CodecError>) -> Result<(), CodecError> {
    let mut i = 0usize;
    loop {
        let b = *tok.get(i).ok_or(CodecError::Slot)?;
        if b == TOK_END {
            return Ok(());
        }
        if b & 0x80 == 0 {
            // literal run
            let len = (b & 0x7F) as usize;
            if len == 0 {
                return Err(CodecError::Slot);
            }
            f(Tk::Lit(len))?;
            i += 1;
        } else {
            let count = (b & 0x7F) as usize;
            if count == 0 {
                return Err(CodecError::Slot);
            }
            // cada match ocupa 4 bytes (1 count + 2B offset + 1B len)
            let end = i + 1 + 3 * count;
            if end > tok.len() {
                return Err(CodecError::Slot);
            }
            for m in 0..count {
                let base = i + 1 + 3 * m;
                let o = u16::from_le_bytes([tok[base], tok[base + 1]]) as usize;
                let off = o - 1;
                let len = tok[base + 2] as usize + MIN_MATCH_LEN;
                if off == usize::MAX || off > LZ_WINDOW || len > MAX_MATCH_LEN {
                    return Err(CodecError::Slot);
                }
                f(Tk::Mat { off, len })?;
            }
            i = end;
        }
    }
}

/// Longitud de la sección de tokens (hasta el `TOK_END`), `None` si corrupta.
pub fn token_end(tok: &[u8]) -> Option<usize> {
    let mut i = 0usize;
    loop {
        let b = *tok.get(i)?;
        if b == TOK_END {
            return Some(i + 1);
        }
        if b & 0x80 == 0 {
            i += 1;
        } else {
            let count = (b & 0x7F) as usize;
            if count == 0 {
                return None;
            }
            i += 1 + 3 * count;
        }
    }
}

/// Nº total de literales en el token stream.
pub fn literal_total(tok: &[u8]) -> Result<usize, CodecError> {
    let mut n = 0usize;
    walk(tok, &mut |tk| match tk {
        Tk::Lit(len) => {
            n += len;
            Ok(())
        }
        _ => Ok(()),
    })?;
    Ok(n)
}

/// Recorre las corridas de literales de `src` según los tokens y aplica `f` a
/// cada byte literal. Devuelve el total de literales procesados. Suscribe
/// además el seguimiento de posición para validar los límites de `src`.
pub fn fold_literals(
    src: &[u8],
    tok: &[u8],
    mut f: impl FnMut(u8),
) -> Result<usize, CodecError> {
    let mut sp = 0usize;
    let mut total = 0usize;
    walk(tok, &mut |tk| {
        match tk {
            Tk::Lit(len) => {
                let end = sp + len;
                if end > src.len() {
                    return Err(CodecError::Slot);
                }
                for i in sp..end {
                    f(src[i]);
                }
                sp = end;
                total += len;
            }
            Tk::Mat { len, .. } => {
                sp += len;
                if sp > src.len() {
                    return Err(CodecError::Slot);
                }
            }
        }
        Ok(())
    })?;
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_roundtrip_simple() {
        let src: Vec<u8> = b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_vec();
        let mut tok = [0u8; 64];
        let mut ht = [0u32; HASH_TAB_LEN];
        let t = tokenize(&src, &mut tok, &mut ht).unwrap();
        assert!(t <= write_bound(src.len()));
        // la página repetida debe generar al menos un match
        let mut has_match = false;
        walk(&tok[..t], &mut |tk| {
            if let Tk::Mat { .. } = tk {
                has_match = true;
            }
            Ok(())
        })
        .unwrap();
        assert!(has_match);
        // el primer byte no puede tener match (sin predecesor) -> 1 literal
        assert_eq!(fold_literals(&src, &tok[..t], |_| {}).unwrap(), 1);
    }

    #[test]
    fn literal_total_counts() {
        let src = b"hola que tal, esto es una prueba".to_vec();
        let mut tok = [0u8; 128];
        let mut ht = [0u32; HASH_TAB_LEN];
        let t = tokenize(&src, &mut tok, &mut ht).unwrap();
        assert_eq!(literal_total(&tok[..t]).unwrap(), src.len());
        assert_eq!(fold_literals(&src, &tok[..t], |_| {}).unwrap(), src.len());
    }
}