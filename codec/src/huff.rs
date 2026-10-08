// SPDX-License-Identifier: MIT
//! Entropía sobre el alfabeto de símbolos: Huffman canónico estático.
//!
//! - Longitudes vía fusión (dos menores, determinista por (freq asc, índice
//!   asc)) sobre arrays fijos (≤ 512 nodos para k ≤ 256).
//! - Códigos canónicos (asignación por (longitud asc, índice asc), estilo
//!   DEFLATE), codificables y decodificables con la tabla clásica
//!   `first_code`/`count` (como `puff.c`).
//! - Bitwriter/bitreader MSB-first; cota de longitud `C_MAX_BITS` (los
//!   histogramas patológicos con longitudes mayores se guardan RAW).
//!
//! Funciones puras y deterministas; `#![forbid(unsafe_code)]`.

/// Máxima longitud de código soportada en el formato v1.
pub const C_MAX_BITS: usize = 12;

/// Tablas de codificación por SÍMBOLO (símbolos = índices 0..k del alfabeto).
pub struct Tables {
    /// longitudes por símbolo (u8).
    pub lengths: [u8; 256],
    /// código canónico por símbolo (u32; solo fiable si `max_len ≤ C_MAX_BITS`).
    pub codes: [u32; 256],
    /// true si la longitud máxima excede `C_MAX_BITS` (el slot debe ir RAW).
    pub over_cap: bool,
}

/// Longitudes de código Huffman para k símbolos (freq[sym] = counts[valor]).
fn huff_lengths(freq: &[u64; 512], k: usize) -> ([u8; 512], u8) {
    const NONE: u16 = 0xFFFF;
    let mut fq = [0u64; 512];
    let mut par = [NONE; 512];
    for s in 0..k {
        fq[s] = freq[s];
    }
    let mut n_nodes = k;

    loop {
        // contar nodos vivos (sin padre)
        let mut live = 0usize;
        for i in 0..n_nodes {
            if par[i] == NONE {
                live += 1;
            }
        }
        if live <= 1 {
            break;
        }
        // dos menores por (freq asc, índice asc)
        let (mut a, mut b) = (usize::MAX, usize::MAX);
        for i in 0..n_nodes {
            if par[i] != NONE {
                continue;
            }
            if a == usize::MAX || fq[i] < fq[a] || (fq[i] == fq[a] && i < a) {
                b = a;
                a = i;
            } else if b == usize::MAX || fq[i] < fq[b] || (fq[i] == fq[b] && i < b) {
                b = i;
            }
        }
        debug_assert!(a != usize::MAX && b != usize::MAX);
        fq[n_nodes] = fq[a] + fq[b];
        par[a] = n_nodes as u16;
        par[b] = n_nodes as u16;
        n_nodes += 1;
    }

    // profundidades = longitudes
    let mut lengths = [0u8; 512];
    let mut max_len = 0u8;
    for s in 0..k {
        let mut d = 0usize;
        let mut x = s;
        while par[x] != NONE {
            d += 1;
            x = par[x] as usize;
        }
        lengths[s] = d as u8;
        if (d as u8) > max_len {
            max_len = d as u8;
        }
    }
    (lengths, max_len)
}

/// Construye las tablas para `counts` por *valor*, `k` símbolos y el orden
/// del alfabeto (`order[sym]` = valor del símbolo `sym`, según
/// `values::alphabet`). `tbl.lengths`/`tbl.codes` están indexados por
/// SÍMBOLO (0..k); en `lib.rs` se mapea valor -> símbolo con `v2s`.
pub fn code_tables(counts: &[u32; 256], k: usize, order: &[u8; 256]) -> Tables {
    let mut freq = [0u64; 512];
    for s in 0..k {
        freq[s] = counts[order[s] as usize] as u64;
    }
    let (lengths, max_len) = huff_lengths(&freq, k);

    let mut tbl = Tables {
        lengths: [0; 256],
        codes: [0; 256],
        over_cap: max_len as usize > C_MAX_BITS,
    };
    for s in 0..k {
        tbl.lengths[s] = lengths[s];
    }

    if tbl.over_cap {
        return tbl;
    }

    // Códigos canónicos por (longitud asc, símbolo asc).
    let mut bl_count = [0u32; C_MAX_BITS + 2];
    for s in 0..k {
        bl_count[tbl.lengths[s] as usize] += 1;
    }
    let mut next = [0u32; C_MAX_BITS + 2];
    next[1] = 0;
    for l in 2..=max_len as usize {
        next[l] = (next[l - 1] + bl_count[l - 1]) << 1;
    }
    // Orden canónico (longitud asc, símbolo asc): equivalente al sort estable
    // por (len, sym) de DEFLATE, sin depender de `slice::sort`.
    for l in 1..=max_len as usize {
        for s in 0..k {
            if tbl.lengths[s] as usize == l {
                tbl.codes[s] = next[l];
                next[l] += 1;
            }
        }
    }
    tbl
}

/// Tabla de decodificación canónica (por longitud).
pub struct DecodeTable {
    max_len: usize,
    first_code: [u32; C_MAX_BITS + 2],
    count: [u32; C_MAX_BITS + 2],
    first_sym: [u32; C_MAX_BITS + 2],
    /// Orden canónico de símbolos (longitud asc, símbolo asc): los símbolos
    /// de una misma longitud NO son necesariamente contiguos en índice, así
    /// que la decodificación debe consultar este orden (no `first_sym+n`).
    sym_order: [u16; 256],
    code: u32,
    len: usize,
}

/// Construye la tabla de decodificación desde las longitudes (orden de
/// símbolos 0..k, como en el slot).
pub fn decode_table(k: usize, lengths: &[u8]) -> Option<DecodeTable> {
    if k == 0 || k > 256 {
        return None;
    }
    let mut max_len = 0usize;
    let mut bl_count = [0u32; C_MAX_BITS + 2];
    for s in 0..k {
        let l = lengths[s] as usize;
        if l == 0 || l > C_MAX_BITS {
            return None;
        }
        bl_count[l] += 1;
        if l > max_len {
            max_len = l;
        }
    }
    let mut first_code = [0u32; C_MAX_BITS + 2];
    first_code[1] = 0;
    for l in 2..=max_len {
        first_code[l] = (first_code[l - 1] + bl_count[l - 1]) << 1;
    }
    // first_sym[l] = nº de símbolos de longitud < l (orden canónico).
    let mut first_sym = [0u32; C_MAX_BITS + 2];
    {
        let mut cum = 0u32;
        for l in 1..=max_len {
            first_sym[l] = cum;
            cum += bl_count[l];
        }
    }
    // Orden canónico por (longitud asc, símbolo asc).
    let mut sym_order = [0u16; 256];
    let mut pos = 0usize;
    for l in 1..=max_len {
        for s in 0..k {
            if lengths[s] as usize == l {
                sym_order[pos] = s as u16;
                pos += 1;
            }
        }
    }
    Some(DecodeTable {
        max_len,
        first_code,
        count: bl_count,
        first_sym,
        sym_order,
        code: 0,
        len: 0,
    })
}

impl DecodeTable {
    /// Alimenta un bit; devuelve `Ok(Some(sym))` si se completó un símbolo,
    /// `Ok(None)` si aún es prefijo, `Err` si el stream es corrupto.
    pub fn decode_bit(&mut self, bit: bool) -> Result<Option<usize>, ()> {
        self.code = (self.code << 1) | (bit as u32);
        self.len += 1;
        let l = self.len;
        if l > self.max_len {
            self.reset();
            return Err(());
        }
        let code = self.code as i64 - self.first_code[l] as i64;
        if code >= 0 && (code as u32) < self.count[l] {
            let off = (code as u32) as usize;
            let sym = self.sym_order[(self.first_sym[l] as usize) + off] as usize;
            self.reset();
            Ok(Some(sym))
        } else {
            Ok(None)
        }
    }

    fn reset(&mut self) {
        self.code = 0;
        self.len = 0;
    }
}

/// Bitwriter MSB-first.
pub struct BitWriter<'a> {
    buf: &'a mut [u8],
    cur: u8,
    nbits: u8,
    pos: usize,
}

impl<'a> BitWriter<'a> {
    pub fn new(buf: &'a mut [u8]) -> Self {
        BitWriter {
            buf,
            cur: 0,
            nbits: 0,
            pos: 0,
        }
    }

    pub fn write(&mut self, v: u32, len: usize) {
        debug_assert!(len <= 32);
        for bit in (0..len).rev() {
            self.put(((v >> bit) & 1) as u8);
        }
    }

    #[inline]
    fn put(&mut self, bit: u8) {
        self.cur = (self.cur << 1) | (bit & 1);
        self.nbits += 1;
        if self.nbits == 8 {
            self.buf[self.pos] = self.cur;
            self.pos += 1;
            self.nbits = 0;
            self.cur = 0;
        }
    }

    /// Vacía y devuelve los bytes usados.
    pub fn finish(mut self) -> usize {
        if self.nbits > 0 {
            self.buf[self.pos] = self.cur << (8 - self.nbits);
            self.pos += 1;
        }
        self.pos
    }
}

/// Bitreader MSB-first.
pub struct BitReader<'a> {
    buf: &'a [u8],
    bitpos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        BitReader { buf, bitpos: 0 }
    }

    pub fn read_bit(&mut self) -> Option<bool> {
        if self.bitpos >= self.buf.len() * 8 {
            None
        } else {
            let b = (self.buf[self.bitpos >> 3] >> (7 - (self.bitpos & 7))) & 1;
            self.bitpos += 1;
            Some(b == 1)
        }
    }
}

/// Convención: `CodecError` para mapear fallos de decodificación en `lib.rs`.
#[cfg(test)]
pub fn decode_err() -> super::CodecError {
    super::CodecError::Slot
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_roundtrip_known() {
        let mut counts = [0u32; 256];
        // A=2,B=3,C=4,D=5 -> fusión determinista de dos menores (freq asc,
        // índice asc): {A,B}=5, {C,D}=9 -> todas las longitudes 2.
        counts[b'A' as usize] = 2;
        counts[b'B' as usize] = 3;
        counts[b'C' as usize] = 4;
        counts[b'D' as usize] = 5;
        let mut order = [0u8; 256];
        for (i, o) in order.iter_mut().enumerate() {
            *o = i as u8;
        }
        order.sort_by(|&a, &b| {
            counts[b as usize]
                .cmp(&counts[a as usize])
                .then_with(|| a.cmp(&b))
        });
        let tbl = code_tables(&counts, 4, &order);
        assert!(!tbl.over_cap);
        assert_eq!(tbl.lengths[3], 2); // D
        assert_eq!(tbl.lengths[2], 2); // C
        assert_eq!(tbl.lengths[0], 2); // A
        assert_eq!(tbl.lengths[1], 2); // B
                                       // Códigos canónicos por símbolo: A=0, B=1, C=2, D=3 (2 bits).
        assert_eq!(tbl.codes[0], 0);
        assert_eq!(tbl.codes[1], 1);
        assert_eq!(tbl.codes[2], 2);
        assert_eq!(tbl.codes[3], 3);
    }

    #[test]
    fn encode_decode_roundtrip() {
        let mut counts = [0u32; 256];
        let bytes = b"abracadabra abracadabra abracadabra";
        for &b in bytes {
            counts[b as usize] += 1;
        }
        let mut v2s = [0u8; 256];
        let mut order = [0u8; 256];
        for i in 0..256 {
            order[i] = i as u8;
        }
        order.sort_by(|&a, &b| {
            counts[b as usize]
                .cmp(&counts[a as usize])
                .then_with(|| a.cmp(&b))
        });
        let k = order
            .iter()
            .position(|&v| counts[v as usize] == 0)
            .unwrap_or(256);
        for s in 0..k {
            v2s[order[s] as usize] = s as u8;
        }
        let tbl = code_tables(&counts, k, &order);
        assert!(!tbl.over_cap);

        let mut bitbuf = [0u8; 512];
        let mut bw = BitWriter::new(&mut bitbuf);
        for &b in bytes {
            let sym = v2s[b as usize] as usize;
            bw.write(tbl.codes[sym], tbl.lengths[sym] as usize);
        }
        let used = bw.finish();

        let mut dt = decode_table(k, &tbl.lengths).unwrap();
        let mut br = BitReader::new(&bitbuf[..used]);
        let mut out = Vec::new();
        let mut result = Ok(());
        while out.len() < bytes.len() {
            match br.read_bit() {
                Some(b) => match dt.decode_bit(b) {
                    Ok(Some(sym)) => out.push(order[sym]),
                    Ok(None) => {}
                    Err(_) => {
                        result = Err(decode_err());
                        break;
                    }
                },
                None => {
                    result = Err(decode_err());
                    break;
                }
            }
        }
        result.unwrap();
        assert_eq!(&out, &bytes);
    }
}
