// SPDX-License-Identifier: (GPL-2.0 OR MIT)
//! Fase 0 del plan estiba: stub mínimo de módulo Rust OUT-OF-TREE.
//!
//! Si `insmod` funciona en la VM del sandbox, el árbol 6.12 soporta
//! módulos Rust externos y `estiba.ko` (Fase 3) es viable sin recompilar
//! el kernel dentro del árbol.

use kernel::prelude::*;

module! {
    type: EstibaOotStub,
    name: "estiba_oot_stub",
    description: "estiba Fase 0 - stub OOT para validar modulos Rust externos",
    license: "GPL v2",
}

struct EstibaOotStub;

impl kernel::Module for EstibaOotStub {
    fn init(_module: &'static ThisModule) -> Result<Self> {
        pr_info!("estiba OOT stub cargado\n");
        Ok(EstibaOotStub)
    }
}

impl Drop for EstibaOotStub {
    fn drop(&mut self) {
        pr_info!("estiba OOT stub descargado\n");
    }
}
