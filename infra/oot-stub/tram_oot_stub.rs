// SPDX-License-Identifier: (GPL-2.0 OR MIT)
//! Fase 0 del plan tRAM: stub mínimo de módulo Rust OUT-OF-TREE.
//!
//! Si `insmod` funciona en la VM del sandbox, el árbol 6.12 soporta
//! módulos Rust externos y `tram.ko` (Fase 3) es viable sin recompilar
//! el kernel dentro del árbol.

use kernel::prelude::*;

module! {
    type: TramOotStub,
    name: "tram_oot_stub",
    description: "tRAM Fase 0 - stub OOT para validar modulos Rust externos",
    license: "GPL v2",
}

struct TramOotStub;

impl kernel::Module for TramOotStub {
    fn init(_module: &'static ThisModule) -> Result<Self> {
        pr_info!("tRAM OOT stub cargado\n");
        Ok(TramOotStub)
    }
}

impl Drop for TramOotStub {
    fn drop(&mut self) {
        pr_info!("tRAM OOT stub descargado\n");
    }
}
