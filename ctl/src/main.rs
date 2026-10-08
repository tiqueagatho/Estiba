// SPDX-License-Identifier: MIT
//! estiba-ctl — inspecciona el dispositivo de bloque `estiba` (RAM comprimida).
//!
//! `estiba.ko` no expone un sysfs propio (el crate `kernel` 6.12 no ofrece
//! kobject/sysfs), así que `estiba-ctl` reporta lo que el kernel ya publica de
//! cualquier dispositivo de bloque: tamaño (`/sys/block/estiba0/size`) y
//! estadísticas de E/S (`/sys/block/estiba0/stat`), más el uso como swap
//! (`/proc/swaps`). Las estadísticas internas del códec (dedup, uniformes)
//! salen por `dmesg` al descargar el módulo.
//!
//! Uso:
//!   estiba-ctl [status]   # estado del dispositivo (por defecto)
//!   estiba-ctl info       # qué es estiba y licencia
//!   estiba-ctl help

use std::env;
use std::fs;
use std::process::ExitCode;

const DEV: &str = "estiba0";

/// Lee un fichero de texto; `None` si no existe o no se puede leer.
fn leer(ruta: &str) -> Option<String> {
    fs::read_to_string(ruta).ok()
}

/// Interpreta `/sys/block/<dev>/stat` (11 contadores en unidades de 512 B).
#[derive(Debug, PartialEq, Eq, Default)]
struct BlockStat {
    lecturas: u64,
    sectores_leidos: u64,
    escrituras: u64,
    sectores_escritos: u64,
    en_vuelo: u64,
    ms_io: u64,
}

fn parse_stat(txt: &str) -> Option<BlockStat> {
    let v: Vec<u64> = txt
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    if v.len() < 11 {
        return None;
    }
    Some(BlockStat {
        lecturas: v[0],
        sectores_leidos: v[2],
        escrituras: v[4],
        sectores_escritos: v[6],
        en_vuelo: v[8],
        ms_io: v[9],
    })
}

/// Busca el dispositivo en `/proc/swaps` y devuelve (usado_kb, total_kb).
fn parse_swaps(txt: &str) -> Option<(u64, u64)> {
    for linea in txt.lines().skip(1) {
        let c: Vec<&str> = linea.split_whitespace().collect();
        if c.first().map(|p| p.ends_with(DEV)).unwrap_or(false) && c.len() >= 4 {
            let total = c[2].parse().ok()?;
            let usado = c[3].parse().ok()?;
            return Some((usado, total));
        }
    }
    None
}

fn humano(bytes: u64) -> String {
    const U: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut n = bytes as f64;
    let mut i = 0;
    while n >= 1024.0 && i < U.len() - 1 {
        n /= 1024.0;
        i += 1;
    }
    format!("{n:.1} {}", U[i])
}

fn cmd_status() -> ExitCode {
    println!("estiba — dispositivo de RAM comprimida en Rust");
    let ruta_dev = format!("/dev/{DEV}");
    if !std::path::Path::new(&ruta_dev).exists() {
        println!("  {ruta_dev}: NO presente (¿módulo estiba.ko cargado?)");
        return ExitCode::from(1);
    }
    println!("  dispositivo: {ruta_dev}");

    let sys = format!("/sys/block/{DEV}");
    if let Some(sz) = leer(&format!("{sys}/size")).and_then(|s| s.trim().parse::<u64>().ok()) {
        println!(
            "  tamaño:      {} ({} sectores de 512 B)",
            humano(sz * 512),
            sz
        );
    }

    match leer(&format!("{sys}/stat")).as_deref().and_then(parse_stat) {
        Some(st) => {
            println!(
                "  lectura:     {} ops, {} ({})",
                st.lecturas,
                humano(st.sectores_leidos * 512),
                "sectores×512"
            );
            println!(
                "  escritura:   {} ops, {}",
                st.escrituras,
                humano(st.sectores_escritos * 512)
            );
            println!("  en vuelo:    {}   io_ticks: {} ms", st.en_vuelo, st.ms_io);
        }
        None => println!("  stats:       no disponibles"),
    }

    match leer("/proc/swaps").as_deref().and_then(parse_swaps) {
        Some((usado, total)) => {
            println!("  swap:        activa — {usado} KiB usados de {total} KiB");
        }
        None => println!("  swap:        no está en uso como swap"),
    }
    ExitCode::SUCCESS
}

fn cmd_info() -> ExitCode {
    println!(
        "estiba — dispositivo de bloque de RAM/swap comprimida escrito en Rust\n\
         (equivalente a zram, con códec propio LZ + entropía y verificación\n\
         bit-exacta). Proyecto experimental/educativo: NO usar para datos\n\
         críticos; requiere un kernel con CONFIG_RUST=y.\n\
         \n\
         Componentes:\n\
         - códec:  estiba-codec (MIT)\n\
         - módulo: estiba.ko (GPL-2.0)\n\
         - banco:  estiba-bench, estiba-ctl (MIT)\n\
         \n\
         Estadísticas internas (dedup/uniformes): dmesg tras 'rmmod estiba'."
    );
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let arg = env::args().nth(1);
    match arg.as_deref() {
        None | Some("status") => cmd_status(),
        Some("info") => cmd_info(),
        Some("help") | Some("-h") | Some("--help") => {
            println!("uso: estiba-ctl [status|info|help]");
            ExitCode::SUCCESS
        }
        Some(otro) => {
            eprintln!("estiba-ctl: subcomando desconocido: {otro}");
            eprintln!("uso: estiba-ctl [status|info|help]");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_parsea_once_valores() {
        let t = "100 5 800 20 40 2 320 10 0 30 0";
        let st = parse_stat(t).expect("stat");
        assert_eq!(st.lecturas, 100);
        assert_eq!(st.sectores_leidos, 800);
        assert_eq!(st.escrituras, 40);
        assert_eq!(st.sectores_escritos, 320);
        assert_eq!(st.en_vuelo, 0);
        assert_eq!(st.ms_io, 30);
    }

    #[test]
    fn stat_corto_es_none() {
        assert_eq!(parse_stat("1 2 3"), None);
    }

    #[test]
    fn swaps_encuentra_estiba() {
        let t = "Filename\t\t\t\tType\t\tSize\t\tUsed\t\tPriority\n\
                 /dev/sda2                               partition\t8000000\t\t0\t\t-2\n\
                 /dev/estiba0                            partition\t4096\t\t128\t\t100\n";
        assert_eq!(parse_swaps(t), Some((128, 4096)));
    }

    #[test]
    fn swaps_sin_estiba() {
        let t = "Filename\tType\tSize\tUsed\n/dev/sda2\tpartition\t8000\t0\n";
        assert_eq!(parse_swaps(t), None);
    }
}
