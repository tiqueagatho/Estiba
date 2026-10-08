// SPDX-License-Identifier: MIT
//! estiba-bench: corpus canónico + baselines LZ4/LZO + gate del codec propio (LZ+Huffman).
//!
//! Mide (baselines por `dlopen` contra liblz4/liblzo2 del sistema, sin deps de
//! build):
//! - ratio (media geométrica, entrada/salida) de estiba vs LZ4 y vs LZO,
//! - throughput de compresión y descompresión (MB/s) de los tres codecs,
//! - determinismo (compress es función pura) y roundtrip del corpus + fuzzing
//!   xorshift sembrado.
//!
//! El gate (SPEC §1, v2 renegociado 2026-10-08) exige: ratio_geomean ≥ 1.50×,
//! throughput comp/decomp ≥ 10 MB/s por núcleo (sostenido sobre 4K), y
//! determinismo + roundtrip bit-exacto del corpus y del fuzzing. LZ4/LZO se
//! miden como referencia (no son criterio bloqueante). Exit 0 si todo pasa,
//! 1 si algún criterio falla.

mod baselines;
mod corpus;

use std::time::Instant;

use baselines::Codec;

struct Stats {
    geo_ratio: Option<f64>,
    comp_mbs: Option<f64>,
    decomp_mbs: Option<f64>,
}

fn main() {
    let corpus = corpus::canonico();
    let fuzz = corpus::fuzz_cases();

    let mut estiba = baselines::Estiba::new();
    let mut lz4 = baselines::Lz4::new().unwrap_or_else(|| die("liblz4.so.1 no cargable"));
    let mut lzo = baselines::Lzo1::new().unwrap_or_else(|| die("liblzo2.so.2 no cargable"));

    // -- roundtrip (corpus + fuzz) y determinismo ---------------------------
    roundtrip("estiba", &mut estiba, &corpus, &fuzz, true);
    roundtrip("LZ4", &mut lz4, &corpus, &[], false);
    roundtrip("LZO", &mut lzo, &corpus, &[], false);

    // -- medición ------------------------------------------------------------
    let estiba_res = measure(&mut estiba, &corpus);
    detalle_estiba(&mut estiba, &corpus);
    let lz4_res = measure(&mut lz4, &corpus);
    let lzo_res = measure(&mut lzo, &corpus);

    // -- tabla ---------------------------------------------------------------
    println!();
    println!(
        "{:<20}{:>12}{:>12}{:>12}",
        "codificador", "ratio", "comp MB/s", "decomp MB/s"
    );
    for (label, s) in [
        ("estiba (LZ+Huf)", &estiba_res),
        ("LZ4 1.10", &lz4_res),
        ("LZO1X 2.10", &lzo_res),
    ] {
        println!(
            "{:<20}{:>12.2}{:>12.1}{:>12.1}",
            label,
            s.geo_ratio.unwrap_or(0.0),
            s.comp_mbs.unwrap_or(0.0),
            s.decomp_mbs.unwrap_or(0.0),
        );
    }

    // -- gate (SPEC §1, v2) ------------------------------------------------
    println!();
    let mut fails = 0usize;
    let ratio_t = estiba_res.geo_ratio.unwrap_or(0.0);
    let comp_t = estiba_res.comp_mbs.unwrap_or(0.0);
    let decomp_t = estiba_res.decomp_mbs.unwrap_or(0.0);
    check(
        &mut fails,
        ratio_t >= 1.50,
        "ratio_geomean >= 1.50x",
        &format!("estiba {ratio_t:.2} vs criterio 1.50"),
    );
    check(
        &mut fails,
        comp_t >= 10.0,
        "comp_tput >= 10 MB/s",
        &format!("estiba {comp_t:.1} vs criterio 10.0"),
    );
    check(
        &mut fails,
        decomp_t >= 10.0,
        "decomp_tput >= 10 MB/s",
        &format!("estiba {decomp_t:.1} vs criterio 10.0"),
    );

    println!();
    let rel = |a: &Option<f64>, b: &Option<f64>| -> f64 {
        match (a, b) {
            (Some(x), Some(y)) => x / y,
            _ => 0.0,
        }
    };
    println!(
        "[referencia] ratio estiba/LZ4 = {:.2}  (no gate en v2)",
        rel(&estiba_res.geo_ratio, &lz4_res.geo_ratio)
    );
    println!(
        "[referencia] ratio estiba/LZO = {:.2}  (no gate en v2)",
        rel(&estiba_res.geo_ratio, &lzo_res.geo_ratio)
    );
    println!(
        "[referencia] comp estiba/LZ4  = {:.0}%  (no gate en v2)",
        100.0 * rel(&estiba_res.comp_mbs, &lz4_res.comp_mbs)
    );
    println!(
        "[referencia] decomp estiba/LZ4 = {:.0}%  (no gate en v2)",
        100.0 * rel(&estiba_res.decomp_mbs, &lz4_res.decomp_mbs)
    );

    println!();
    if fails == 0 {
        println!("GATE: VERDE");
        std::process::exit(0);
    }
    println!("GATE: ROJO ({fails} criterios sin cumplir)");
    std::process::exit(1);
}

fn die(msg: &str) -> ! {
    eprintln!("[FALLO] {msg}");
    std::process::exit(1);
}

/// Roundtrip de cada elemento del corpus y del fuzzing; si `det` es true,
/// verifica además que comprimir dos veces da exactamente los mismos bytes.
fn roundtrip(
    label: &str,
    codec: &mut dyn Codec,
    corpus: &[corpus::Item],
    fuzz: &[Vec<u8>],
    det: bool,
) {
    let mut slot: Vec<u8> = Vec::new();
    let mut out: Vec<u8> = Vec::new();
    for item in corpus {
        test_io(
            label, codec, &item.data, &mut slot, &mut out, det, &item.name,
        );
    }
    for (i, fc) in fuzz.iter().enumerate() {
        test_io(
            label,
            codec,
            fc,
            &mut slot,
            &mut out,
            det,
            &format!("fuzz#{i}"),
        );
    }
    println!(
        "[{:>8}] roundtrip: {} items + {} fuzz",
        "PASS",
        corpus.len(),
        fuzz.len()
    );
}

fn test_io(
    label: &str,
    codec: &mut dyn Codec,
    src: &[u8],
    slot: &mut Vec<u8>,
    out: &mut Vec<u8>,
    det: bool,
    name: &str,
) {
    slot.clear();
    if codec.compress_into(src, slot).is_err() {
        die(&format!("{label} no comprime `{name}`"));
    }
    if det {
        let mut slot2: Vec<u8> = Vec::new();
        if codec.compress_into(src, &mut slot2).is_err() || slot2 != *slot {
            die(&format!("{label} determinismo roto en `{name}`"));
        }
    }
    out.clear();
    if codec.decompress_into(slot, src.len(), out).is_err() {
        die(&format!(
            "{label} no descomprime `{name}` (slot {} B)",
            slot.len()
        ));
    }
    if *out != src {
        die(&format!("{label} roundtrip distinto en `{name}`"));
    }
}

/// Comprime todo el corpus con `codec`, mide ratios y throughput (sustain en
/// ~150 ms; los buffers se reutilizan, así el alloc no cuenta en la velocidad).
fn measure(codec: &mut dyn Codec, corpus: &[corpus::Item]) -> Stats {
    let mut ratios = Vec::with_capacity(corpus.len());
    let mut comps: Vec<(usize, Vec<u8>)> = Vec::with_capacity(corpus.len());
    let mut out: Vec<u8> = Vec::new();
    for item in corpus {
        out.clear();
        if codec.compress_into(&item.data, &mut out).is_err() {
            die(&format!("{} explota en `{}`", codec.name(), item.name));
        }
        ratios.push(ratio(item.data.len(), out.len()));
        comps.push((item.data.len(), std::mem::take(&mut out)));
    }
    let c = speed_compress(codec, corpus);
    let d = speed_decompress(codec, &comps);
    Stats {
        geo_ratio: Some(geomean(&ratios)),
        comp_mbs: Some(c),
        decomp_mbs: Some(d),
    }
}

fn speed_compress(codec: &mut dyn Codec, corpus: &[corpus::Item]) -> f64 {
    let total: usize = corpus.iter().map(|i| i.data.len()).sum();
    let mut iters = 1usize;
    let mut out: Vec<u8> = Vec::new();
    let mut t = std::time::Duration::ZERO;
    while t.as_secs_f64() < 0.15 && iters < 512 {
        iters *= 2;
        let t0 = Instant::now();
        for _ in 0..iters {
            for item in corpus {
                out.clear();
                let _ = codec.compress_into(&item.data, &mut out);
            }
        }
        t = t0.elapsed();
    }
    let bytes = (total as u64).saturating_mul(iters as u64);
    bytes as f64 / t.as_secs_f64() / 1e6
}

fn speed_decompress(codec: &mut dyn Codec, comps: &[(usize, Vec<u8>)]) -> f64 {
    let total: usize = comps.iter().map(|(_, c)| c.len()).sum();
    let mut iters = 1usize;
    let mut out: Vec<u8> = Vec::new();
    let mut t = std::time::Duration::ZERO;
    while t.as_secs_f64() < 0.15 && iters < 512 {
        iters *= 2;
        let t0 = Instant::now();
        for _ in 0..iters {
            for (exp, comp) in comps {
                out.clear();
                let _ = codec.decompress_into(comp, *exp, &mut out);
            }
        }
        t = t0.elapsed();
    }
    let bytes = (total as u64).saturating_mul(iters as u64);
    bytes as f64 / t.as_secs_f64() / 1e6
}

fn ratio(in_b: usize, out_b: usize) -> f64 {
    if out_b == 0 {
        f64::INFINITY
    } else {
        in_b as f64 / out_b as f64
    }
}

/// Ratios por clase de estiba (diagnóstico: dónde pierde/gana contra LZ4).
fn detalle_estiba(codec: &mut dyn Codec, corpus: &[corpus::Item]) {
    let mut out: Vec<u8> = Vec::new();
    println!("\nestiba ratios por clase:");
    for item in corpus {
        out.clear();
        if codec.compress_into(&item.data, &mut out).is_err() {
            continue;
        }
        println!(
            "  {:>18} {:>5} B -> {:>5} B  ({:.2}x)",
            item.name,
            item.data.len(),
            out.len(),
            ratio(item.data.len(), out.len())
        );
    }
}

fn geomean(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let s: f64 = v.iter().map(|r| r.ln()).sum();
    (s / v.len() as f64).exp()
}

fn check(fails: &mut usize, pass: bool, label: &str, detail: &str) {
    if !pass {
        *fails += 1;
    }
    println!(
        "[{:>4}] {label:<24} {detail}",
        if pass { "PASS" } else { "FAIL" }
    );
}
