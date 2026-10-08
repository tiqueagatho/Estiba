# estiba — presentación para la comunidad

## En una frase

**estiba** es un *dispositivo de bloque de RAM/swap comprimida para Linux,
escrito en Rust*, con códec propio y verificación bit-exacta. Es el "zram en
Rust": experimental, educativo y abierto a mejoras.

## El problema, en cristiano

Cuando la RAM se llena, Linux manda páginas a un disco (swap) que es enorme
pero lentísimo. `zram` (el que usa Android) lo evita comprimiendo esas páginas
y guardándolas en la propia RAM. estiba hace lo mismo, pero:

- el códec es **propio** (LZ + entropía, no LZO/LZ4 de terceros),
- todo está en **Rust**, memory-safe, y
- hay **verificación bit-exacta**: el mismo bitstream en userspace y dentro del
  kernel.

## Qué NO es (honestidad por delante)

- **No es un códec novedoso** ni pretende ganar a `zstd` en ratio. LZ+entropía
  es un campo maduro (DEFLATE/zstd).
- **No está pensado para producción todavía**: requiere un kernel con
  `CONFIG_RUST=y` y solo se ha probado en VM (QEMU/KVM).

El valor no es el algoritmo: es la **combinación memory-safe + determinista +
verificable**, y ser un **buen material para aprender** cómo se escribe un
driver de bloque en Rust (bio/blk-mq, folios grandes, contexto atómico,
verificación con paridad).

## Estado (2026-10)

| Fase | Qué | Estado |
|---|---|---|
| 0 | Sandbox QEMU + kernel custom `CONFIG_RUST=y` | ✅ |
| 1 | Códec (LZ + Huffman, CRC, fallback RAW) | ✅ |
| 2 | Verificación bit-exacta + benchmark con swap real + gate | ✅ |
| 3 | `estiba.ko`: insmod + roundtrip + `swapon` en VM | ✅ |
| 4 | Dedup por contenido + páginas uniformes ✅ · asignador por clases, writeback, aging ⬜ | 🟡 |
| 5 | `estiba-ctl`, CI, documentación | ✅ |

## Cómo empezar

```bash
cd codec && cargo test --features alloc    # códec (17 tests)
cd ../module/estiba && make parity          # paridad bit-exacta códec ⇔ kernel
cd ../../infra/sandbox && ./run.sh          # VM completa (kernel custom + estiba.ko + swapon)
```

## Cómo ayudar

Lo más pedido está en la **Fase 4**: asignador de objetos de tamaño variable
(estilo `zsmalloc`), writeback a disco y envejecimiento hot/cold. También se
agradecen más tests, ports y documentación. Detalles en
[`CONTRIBUTING.md`](../CONTRIBUTING.md).

## Licencia

**GPL-2.0** para el módulo/kernel, **MIT** para el códec y las herramientas.
