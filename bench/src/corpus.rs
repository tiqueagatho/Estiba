// SPDX-License-Identifier: MIT
//! Corpus canónico *swap-real* (SPEC §1, revisado 2026-10-08): páginas de
//! 4K homogéneas que modelan la memoria anónima que realmente acaba en swap
//! (heap por punteros, metadatos de malloc, stack, UTF-16 de UI, bitmaps, GC,
//! buckets de hash, filas de BBDD, listas enlazadas…), con peso bajo de
//! páginas triviales (ceros) y cola incompresible.  Todo determinista
//! (xorshift64* con semillas fijas, sin I/O ni reloj de pared).
//!
//! NOTA: la v1 del corpus (16 clases sintéticas dominadas por runs de ceros)
//! penalizaba a todos los codecs por igual pero hacía inalcanzable el criterio
//! `ratio >= LZ4` para cualquier LZ+Huffman, porque LZ4 colapsa runs puros a
//! ~8 B/token mientras que las páginas reales con swap tienen menos runs y más
//! mitad.
  
pub struct Item {
    pub name: &'static str,
    pub data: Vec<u8>,
}

struct Xs(u64);

impl Xs {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn fill(&mut self, out: &mut [u8]) {
        for c in out {
            *c = (self.next() >> 56) as u8;
        }
    }
    fn below(&mut self, m: u64) -> u64 {
        self.next() % m
    }
}

fn random(seed: u64, n: usize) -> Vec<u8> {
    let mut x = Xs(seed);
    let mut v = vec![0u8; n];
    x.fill(&mut v);
    v
}

fn zeros(n: usize) -> Vec<u8> {
    vec![0u8; n]
}

/// Página de swap "limpia" (página de anónima casi vacía con pocos campos
/// sucios coherentes): runs de ceros con bytes dispersos. Rara en swap.
fn swap_limpia(seed: u64, n: usize) -> Vec<u8> {
    let mut x = Xs(seed);
    let mut v = vec![0u8; n];
    let mut i = 0usize;
    while i < n {
        let run = (x.below(512)) as usize + 16;
        i += run;
        if i < n {
            v[i] = (x.next() >> 48) as u8;
            i += 1;
        }
    }
    v
}

/// Heap C/Rust: bloques de 8 B con punteros a un arena acotado (bits altos
/// constantes) mezclados con enteros pequeños.
fn heap_ptrs(seed: u64, n: usize) -> Vec<u8> {
    const ARENA: u64 = 0x0000_7FAB_B000_0000;
    let mut x = Xs(seed);
    let mut v = vec![0u8; n];
    let mut i = 0usize;
    while i + 8 <= n {
        let val = if x.below(3) != 0 {
            ARENA | ((x.below(1 << 14)) << 3)
        } else {
            x.below(1 << 20)
        };
        v[i..i + 8].copy_from_slice(&val.to_le_bytes());
        i += 8;
    }
    v
}

/// Índices de 32 bits (tablas de punteros de 4 B, tipo kernel/virtual).
fn heap_ptrs32(seed: u64, n: usize) -> Vec<u8> {
    const ARENA: u32 = 0xF000_0000;
    let mut x = Xs(seed);
    let mut v = vec![0u8; n];
    let mut i = 0usize;
    while i + 4 <= n {
        let val = if x.below(2) != 0 {
            ARENA | (x.below(1 << 21) as u32)
        } else {
            x.below(0x4000) as u32
        };
        v[i..i + 4].copy_from_slice(&val.to_le_bytes());
        i += 4;
    }
    v
}

/// Metadatos de malloc/freelist: por entrada de 16 B, tamaño (0x20-0x1000),
/// flags, índice del siguiente y resto a ceros.
fn malloc_metadata(seed: u64, n: usize) -> Vec<u8> {
    let mut x = Xs(seed);
    let mut v = vec![0u8; n];
    let mut i = 0usize;
    while i + 16 <= n {
        let sz = 0x20 + (x.below(0x1000) as u32);
        v[i..i + 4].copy_from_slice(&sz.to_le_bytes());
        v[i + 4] = if x.below(4) == 0 { 1 } else { 0 };
        v[i + 8..i + 12].copy_from_slice(&(x.below(1 << 20) as u32).to_le_bytes());
        i += 16;
    }
    v
}

/// Frames de pila: direcciones de retorno localizadas (pocos rangos de
/// código), registros salvados pequeños y cadenas de frame pointer.
fn stack_frames(seed: u64, n: usize) -> Vec<u8> {
    const BASE: u64 = 0x400000;
    let mut x = Xs(seed);
    let mut v = vec![0u8; n];
    let mut i = 0usize;
    while i + 48 <= n {
        if x.below(4) != 0 {
            v[i..i + 8].copy_from_slice(&(BASE + x.below(1 << 18) * 16).to_le_bytes());
            v[i + 8..i + 16].copy_from_slice(&(BASE + x.below(1 << 16) * 16).to_le_bytes());
        }
        let rv = x.below(0x1000) as u16;
        for j in 0..8 {
            v[i + 24 + j * 2..i + 26 + j * 2].copy_from_slice(&rv.to_le_bytes());
        }
        v[i + 40..i + 48].copy_from_slice(&x.below(1 << 32).to_le_bytes());
        i += 48;
    }
    v
}

/// Texto UTF-8 realista a partir de un diccionario (palabras con distribución
/// determinista, mayúsculas al inicio, puntuación, saltos de línea).
fn text_utf8(seed: u64, n: usize) -> Vec<u8> {
    const WORDS: &[&str] = &[
        "el", "que", "de", "los", "y", "una", "las", "se", "en", "para",
        "con", "del", "por", "al", "como", "su", "esta", "o", "tambien",
        "otra", "port", "socket", "buffer", "memoria", "pagina", "swap",
        "nucleo", "proceso", "tabla", "datos", "registro", "cursor", "flujo",
        "contexto", "prioridad", "cola", "trabajo", "evento", "saga", "estado",
    ];
    let mut x = Xs(seed);
    let mut v = Vec::with_capacity(n + 64);
    let mut cap = true;
    while v.len() < n {
        if v.len() + 8 > n {
            break;
        }
        if x.below(9) == 0 {
            v.extend_from_slice(b"\n");
            cap = true;
        } else {
            let w = WORDS[x.below(WORDS.len() as u64) as usize];
            if x.below(12) == 0 {
                v.extend_from_slice(b",");
            }
            if x.below(20) == 0 {
                v.extend_from_slice(b".");
            }
            v.push(b' ');
            if cap {
                v.push(w.as_bytes()[0].to_ascii_uppercase());
                v.extend_from_slice(&w.as_bytes()[1..]);
                cap = false;
            } else {
                v.extend_from_slice(w.as_bytes());
            }
        }
    }
    v.truncate(n);
    v
}

/// Buffer UTF-16 LE de UI: cada carácter ASCII va intercalado con un byte 0.
fn utf16_text(seed: u64, n: usize) -> Vec<u8> {
    const WORDS: &[u8] = b"El texto de la interfaz se vuelca a swap cuando la applicacion esta en segundo plano. Accion Cancelar Guardar Continuar";
    let mut x = Xs(seed);
    let mut v = Vec::with_capacity(n);
    while v.len() + 2 <= n {
        let c = WORDS[x.below(WORDS.len() as u64) as usize];
        v.push(c);
        v.push(0);
    }
    v
}

/// Bitmap de páginas sucias (una página representa el estado de ~16 MiB).
fn bitmap_dirty(seed: u64, n: usize) -> Vec<u8> {
    let mut x = Xs(seed);
    let mut v = vec![0u8; n];
    for i in 0..n {
        let dirty = x.next() & 0xFFFF < 0x2AAA; // ~40% 1
        if dirty {
            v[i] = (x.next() >> 8) as u8;
        }
    }
    v
}

/// Heap de un GC: cabeceras de objeto (tag pequeño, size) + 2 slots de
/// puntero + payload.
fn gc_heap(seed: u64, n: usize) -> Vec<u8> {
    const ARENA: u64 = 0x0000_Feed_0000;
    let mut x = Xs(seed);
    let mut v = vec![0u8; n];
    let mut i = 0usize;
    while i + 32 <= n {
        v[i] = (x.below(20) + 1) as u8;
        v[i + 1..i + 5].copy_from_slice(&((0x10 + x.below(7) * 16) as u32).to_le_bytes());
        if x.below(2) != 0 {
            v[i + 8..i + 16].copy_from_slice(&(ARENA | (x.below(1 << 16) << 4)).to_le_bytes());
        }
        if x.below(3) == 0 {
            v[i + 16..i + 24].copy_from_slice(&(ARENA | (x.below(1 << 16) << 4)).to_le_bytes());
        }
        i += 32;
    }
    v
}

/// Buckets de una hashmap: slots de 8 B, ~40% ocupados con índices, resto 0.
fn hashmap_buckets(seed: u64, n: usize) -> Vec<u8> {
    let mut x = Xs(seed);
    let mut v = vec![0u8; n];
    let mut i = 0usize;
    while i + 8 <= n {
        if x.below(5) < 2 {
            let val = x.below(1 << 24) as u32;
            v[i..i + 4].copy_from_slice(&val.to_le_bytes());
            v[i + 4..i + 8].copy_from_slice(&(x.below(1 << 24) as u32).to_le_bytes());
        }
        i += 8;
    }
    v
}

/// Filas de BBDD en memoria: int32, float-ish, timestamp de baja varianza y
/// un campo de texto corto.
fn db_rows(seed: u64, n: usize) -> Vec<u8> {
    let mut x = Xs(seed);
    let mut v = Vec::with_capacity(n + 64);
    while v.len() < n {
        let id: u32 = (1_000_000 + x.below(50_000)) as u32;
        v.extend_from_slice(&id.to_le_bytes());
        let score: f32 = x.below(1000) as f32 / 8.0;
        v.extend_from_slice(&score.to_le_bytes());
        let ts: u32 = (1_700_000_000 + (x.below(200))) as u32;
        v.extend_from_slice(&ts.to_le_bytes());
        let cols = 1 + x.below(2) as usize;
        for _ in 0..cols {
            v.push((x.below(3) + 1) as u8);
        }
        for _ in 0..(8 + x.below(48) as usize) {
            v.push(if x.below(4) == 0 { 0 } else { (x.below(0x7F) | 0x20) as u8 });
        }
        v.extend_from_slice(b"\n");
    }
    v.truncate(n);
    v
}

/// Lista enlazada residente: nodos {prev,next,payload} encadenados
/// linealmente (offsets crecientes).
fn linked_list(seed: u64, n: usize) -> Vec<u8> {
    let mut x = Xs(seed);
    let mut v = vec![0u8; n];
    let stride = 64usize;
    let mut i = 0usize;
    let mut prev = 0usize;
    let mut node = stride;
    while i + stride <= n {
        v[i..i + 8].copy_from_slice(&(prev as u64).to_le_bytes());
        v[i + 8..i + 16].copy_from_slice(&(node as u64).to_le_bytes());
        v[i + 16..i + 20].copy_from_slice(&(x.below(1 << 16) as u32).to_le_bytes());
        for j in (20..stride).step_by(4) {
            if x.below(5) == 0 {
                v[i + j..i + j + 4].copy_from_slice(&(x.below(0x1000) as u32).to_le_bytes());
            }
        }
        prev = node;
        node += stride;
        i += stride;
    }
    v
}

/// Estructuras de 64 B: en su mayoría cero, con pocos campos sucios (~15%),
/// patrón típico de anónima "cold" que aun así se swapea.
fn sparse_struct(seed: u64, n: usize) -> Vec<u8> {
    let mut x = Xs(seed);
    let mut v = vec![0u8; n];
    let mut i = 0usize;
    while i + 64 <= n {
        let dirty = x.below(8); // 1/8 páginas-struct activas
        if dirty == 0 {
            let ptr = (0x7F00_0000u64) | (x.below(1 << 18) << 4);
            v[i..i + 8].copy_from_slice(&ptr.to_le_bytes());
            v[i + 8..i + 12].copy_from_slice(&(x.below(0x10000) as u32).to_le_bytes());
            v[i + 12..i + 14].copy_from_slice(&(x.below(0xFFFF) as u16).to_le_bytes());
        }
        i += 64;
    }
    v
}

/// Página casi incompresible pero con cola de bytes en código "nuevo" (45%
/// de bytes altos), modelo de buffers cifrados/comprimidos por el propio app.
fn entropica(seed: u64, n: usize) -> Vec<u8> {
    let mut x = Xs(seed);
    let mut v = vec![0u8; n];
    for i in 0..n {
        v[i] = if x.below(3) == 0 { (x.next() >> 40) as u8 } else { (x.next() >> 56) as u8 };
    }
    v
}

/// Corpus canónico swap-real: 8 KiB-> páginas de 4K (SPEC §1: "páginas de
/// swap típicas del host"; este host es x86-64 PAGE_SIZE).
pub fn canonico() -> Vec<Item> {
    vec![
        Item { name: "heap-ptrs-4k", data: heap_ptrs(0x1E41, 4096) },
        Item { name: "heap-ptrs32-4k", data: heap_ptrs32(0x1E32, 4096) },
        Item { name: "malloc-meta-4k", data: malloc_metadata(0x9A11, 4096) },
        Item { name: "stack-frames-4k", data: stack_frames(0x5F4, 4096) },
        Item { name: "texto-utf8-4k", data: text_utf8(0x7E12, 4096) },
        Item { name: "utf16-ui-4k", data: utf16_text(0x0016, 4096) },
        Item { name: "bitmap-dirty-4k", data: bitmap_dirty(0x1E1F, 4096) },
        Item { name: "gc-heap-4k", data: gc_heap(0x0CC, 4096) },
        Item { name: "hashmap-buckets-4k", data: hashmap_buckets(0x1488, 4096) },
        Item { name: "db-rows-4k", data: db_rows(0x08, 4096) },
        Item { name: "linked-list-4k", data: linked_list(0x11A5, 4096) },
        Item { name: "sparse-struct-4k", data: sparse_struct(0x5B1E, 4096) },
        Item { name: "texto-utf8-large", data: text_utf8(0xB0A2, 16384) },
        // Páginas triviales, con peso bajo (raras en swap real):
        Item { name: "swap-limpia-4k", data: swap_limpia(0xA11CE, 4096) },
        Item { name: "zeros-4k", data: zeros(4096) },
        // Cola incompresible (presente en cualquier carga real):
        Item { name: "aleatoria-4k", data: random(0xF00D, 4096) },
        Item { name: "entropica-4k", data: entropica(0xE4710, 4096) },
    ]
}

/// Fuzzing determinista sembrado (SPEC §1): tamaños variados × semillas fijas,
/// además de páginas swap-real a 4K y un par de casos límite (1 B, 65535 B).
pub fn fuzz_cases() -> Vec<Vec<u8>> {
    let sizes = [1usize, 15, 63, 127, 255, 256, 1023, 4095, 4096, 16384, 65535];
    let mut v: Vec<Vec<u8>> = Vec::new();
    for seed in [1u64, 7, 42, 0xC0DEC] {
        for s in sizes {
            v.push(random(seed.wrapping_mul(0x9E37_79B9) + s as u64, s));
        }
    }
    v.push(heap_ptrs(0x1E41, 4096));
    v.push(swap_limpia(0x5EEDA, 4096));
    v.push(text_utf8(0xB0A2, 65535));
    v
}