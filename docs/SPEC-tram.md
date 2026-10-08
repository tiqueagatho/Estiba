# tRAM — Especificación técnica (Fases 1-5)

Estado: en desarrollo. Fase 0 (kernel 6.12 + CONFIG_RUST=y + canarios) **PASS en sandbox**.
Fase 2b (bench + gate v2) **PASS 2026-10-08** (véase §1/§8).

## 1. Propuesta técnica

> *El párrafo siguiente describe la **visión original** (rANS ternario + AVX2),
> preservada como contexto/roadmap. La implementación v1 (2026-10-08) es un
> codec LZ + Huffman canónico — véase §3 y el registro de cambios §8.*

tRAM es un dispositivo de bloque de RAM comprimida (zram-like) escrito en Rust
que sustituye el backend de compresión (LZO/LZ4) por un **codec ternario**: los
bytes se codifican en trits balanceados y se entropía-codifican con rANS sobre
el alfabeto `{-1, 0, +1}` (≈ 1.585 bits/trit). Se elimina el doble búfer
intermedio (los datos se comprimen en el propio búfer de entrada) y el coproceso
diseñado para AVX2 permite canalizar el empaquetado.

### Motivación (dónde gana)

- Páginas de swap/RAM tienen mucha redundancia local (más que archivos): las
  páginas limpias de swap son casi-sólo-ceros, las de página-cache contienen
  bloques repetidos y texto con alfabeto pobre.
- LZ4/LZO **no tienen etapa de entropía**: emiten literales a 8 bits.
  El codec ternario codifica los literales restantes (tras el LZ) en el límite
  de entropía del propio dato. Cuando el alfabeto es pobre (común en swap) la
  ganancia sobre LZ4 es real (típicamente 8-20% de ratio extra).
- Coste: rANS ternario es más lento que LZ4 por decimal de letra; el objetivo
  es paridad o superación de **LZO** (el "low CPU" clásico de zram), puesto que
  tRAM se orienta a ahorro de RAM (ratio) con CPU aceptable.

### Objetivos de aceptación (gate v2 — aprobado por el gate humano el 2026-10-08)

> **Registro del cambio desde v1 (2026-XX-XX)**: el gate v1 exigía ratio ≥ LZ4
> y ≥ 1.10× LZO y throughput ≥ 40% de LZ4 / ≥ 100% de LZO sobre un corpus de 16
> clases sintéticas. Al medir honesto (véase *Registro de cambios*, §8) ese gate
> era inalcanzable para la implementación real (v1 = LZ + Huffman canónico), por
> dos motivos de diseño y uno de método:
> 1. El corpus sintético estaba dominado por runs de ceros (fáciles para LZ4;
>    nada que ver con el swap anónimo real). Se sustituyó por un **corpus
>    swap-real** de 17 páginas de 4K que modelan memoria anónima.
> 2. El codec v1 emite tokens de match (~4 B/match) y literal bit a bit; en el
>    corpus swap-real los ratios caen a la franja común 2-3× para todos los
>    codecs y tram pierde ~30% contra LZ4 por coste de tokens.
> 3. El throughput v1 exigía ≥40% de LZ4 (≈ 210-520 MB/s): un orden de magnitud
>    por encima de lo que el empaque bit-a-bit permite (~24 MB/s).
>
> Los criterios v2 anclan en el **propósito del gate** — swap comprimido
> claramente rentable y que no ahogue la CPU bajo tormenta de swap — en vez de
> en comparaciones relativas al codec del día. Se miden con `tram-bench` sobre
> el corpus canónico swap-real (mediana de 3 runs; host compartido ⇒ ±20-40% de
> ruido):

1. **Ratio**: media geométrica de ratio (entrada/salida) **≥ 1.50×** sobre el
   corpus completo. (Medido 2026-10-08: **2.12×**, margen 1.41×. Un swap 2:1
   frente al swap crudo duplica la RAM efectiva; >1.5× garantiza rentabilidad
   holgada sin tocar la cola incompresible del corpus.)
2. **Throughput por núcleo** (sostenido sobre páginas de 4K): compresión y
   descompresión **≥ 10 MB/s** cada una. (Medido: comp **~24 MB/s**, decomp
   **~50 MB/s**; margen ≥ 2.4×. El umbral cubre una tormenta de ~2500
   páginas/s con CPU sobrante y es ~20× el rendimiento de un swap de disco a
   4K aleatorio. tRAM no compite con zram+AVX2 en MB/s; compite en RAM
   ahorrada.)
3. **Determinismo**: `compress(página)` es función pura (dos compresiones del
   mismo búfer → mismo output bit a bit).
4. **Roundtrip bit-exacto**: `decompress(compress(x)) == x` para todo el corpus
   y el fuzzing determinista xorshift sembrado.
5. *(Referencia, no gate)*: cada run publica ratio y MB/s de LZ4/LZO (tram:
   ratio ~71%/69% de LZ4/LZO; MB/s ~4-5% de LZ4) como contexto de evolución,
   sin ser criterio bloqueante. *Nota metodológica*: los números de LZ4/LZO
   fluctúan ±40% por host compartido; el gate v2 usa umbrales absolutos
   (deterministas y robustos a esa varianza), lo que elimina el ruido de
   comparación inter-run.

## 2. Arquitectura (writable)

```
tram/                     ← workspace Rust (edition 2021, resolver 2)
├─ codec/                 ← crate "tram-codec" [lib, no_std a discreción del call-site]
│  └─ src/
│     ├─ lib.rs           ← API: compress(src:&[u8])->Vec<u8>, decompress(...)
│     ├─ lz.rs            ← capa match (bloques 4KB locales, tabla hash 4 bytes)
│     ├─ trinans.rs       ← rANS sobre trits balanceados (forward/reverse)
│     ├─ trits.rs         ← conversión byte↔trits balanceados + packing 5-trits→8bits
│     ├─ zerorun.rs       ← fast-path de páginas casi-cero (RLE ternario)
│     └─ format.rs        ← cabecera y descripción del formato de slots
├─ bench/
│  └─ src/main.rs         ← tram-bench: corpus + baselines lz4/lzo + informe
├─ ctl/
│  └─ src/main.rs         ← tram-ctl: status/mk/resize matching sysfs de tram.ko
├─ module/                ← crate/árbol "tram" OOT para Kbuild (tram.ko)
│  ├─ Makefile
│  ├─ tram.rs             ← módulo Rust (blk-mq + bio + codec + sysfs + backend)
│  └─ cargo/…             ← infraestructura de build OOT (recomendado: cargo rustc via kbuild)
└─ docs/                  ← guías (build, instalación, seguro, benchmark)
```

### 2.1 Slots y diagrama de datos

```
página virtual 4KB (a menudo success ↔ swap slot)

  write:  [sector_in/4K] → codec::compress(page)            → store[index] = bytes
  read :  store[index]   → codec::decompress(store[index])  → page

Almacen físico: pool de páginas que solo crece hasta `disksize`.
Estrategia de alojamiento: tabla de slots indexada por `pfn`, mutex global.
```

## 3. tram-codec (el corazón)

> **Alineamiento 2026-10-08**: esta sección describe la implementación real v1
> (**LZ estilo LZ4 + Huffman canónico sobre el alfabeto de valores**). La
> propuesta original de rANS ternario queda fuera del alcance de v1 — véanse
> §8 (registro) y la nota final de esta sección.

### 3.1 Formato de un slot comprimido

```
offset  tamaño   campo
0       2        magic 0x54 0x52 ("TR")
2       1        versión (1)
3       1        flags  (bit0 = store_raw)
4       2        input_len  (longitud de la página original, ≤ 65535, u16 LE)
6       2        output_len (longitud del payload almacenado, u16 LE)
8       2        checksum CRC16 (polinomio 0x8005, reflejado) sobre el payload
10      n        payload
```

El payload es:
- `store_raw` (flag bit0): el input sin tocar (`output_len == input_len`);
- normal: `tokens LZ` (terminadores estructurales, sin `0xFF` de búsqueda) +
  `sección de entropía`.

Regla de contracción: si el slot comprimido no mejora, se almacena **RAW**
(`output_len = input_len`). El slot nunca es mayor que `input_len + 10 + 1`.
`n == 0` se serializa siempre como slot RAW vacío.

### 3.2 LZ (capa de matches, `lz.rs`)

- Ventana 4 KB (`LZ_WINDOW = 0xFFFE`), tabla hash de cadenas de 4 bytes
  (`HASH_TAB_LEN = 4096`, hash SplitMix sobre los 4 bytes) — v1: **1 candidato
  por hash head**, sin cadenas (como LZ4).
- Greedy hacia delante; match mínimo 4, máximo 255; los matches se emiten
  como (u16 offset+1, u8 longitud-4).
- Token stream (bytes, sin entropía):
  - `0b0LLLLLLL` (1..=127): corrida de L literales (los bytes NO se copian al
    stream: los consume la capa de entropía);
  - `0b1MMMMMMM` (1..=127): M matches consecutivos, cada uno 3 B (off+1 u16 LE,
    len-4 u8);
  - `0xFF`: fin (parseo **estructural**; `0xFF` dentro de offsets es legal).
- Determinista: misma entrada → mismo token stream bit a bit.

### 3.3 Entropía: valores + Huffman canónico (`values.rs` + `huff.rs`)

1. **Alfabeto de valores**: los K valores distintos de los literales se ordenan
   (count desc, valor asc). Cada literal se mapea a un símbolo 0..K-1.
   `K == 1` (página de un solo byte, típico de swap limpio) ⇒ padding de la
   salida tras la cabecera de entropía, 0 bits de payload.
2. **Huffman canónico estático** sobre los símbolos (longitud ≤ `C_MAX = 12`;
   histogramas patológicos → slot RAW).
3. Cabecera de entropía: `K (1 B) | alfabeto (K B) | longitudes canónicas (K B)`,
   seguido del **bitstream MSB-first** de los símbolos de los literales (recorridos
   en orden de tokens).

`build_entropy` corre toda la etapa sobre el buffer `scratch` (~`compress_bound`)
aportado por el caller — **sin alloc en el camino caliente** y con
`#![forbid(unsafe_code)]`.

### 3.4 Descompresión (`lib.rs::restore_payload`)

1. Separa tokens (fin marcado estructuralmente) de la sección de entropía.
2. Lee K/alfabeto/longitudes; decodifica el bitstream a los valores de los
   literales (tabla de decodificación canónica, `BitReader` bit a bit).
3. Emite: recorre los tokens empalmando literales y matches; los matches se
   copian **byte a byte hacia delante** (permite matches solapados RLE-like,
   `off < len`). Los límites de `off`/`out_pos` se validan contra `expected =
   input_len`, NO contra el tamaño del búfer de salida (patrón kernel/swap,
   prueba de regresión `decompress_into_buffer_mayor_que_input`).

### 3.5 Nota: el paso "ternario"

La codificación frontal "trits balanceados + rANS" (planos de signo P/N y
packing AVX2, Fase 6) es una **vía futura**: v1 alcanza aproximadamente la cota
de entropía del dato (fruto de la etapa Huffman, que LZ4/LZO no tienen) sobre
el alfabeto de los valores de los literales. La especificación de trits
(`BITS_PER_TRIT`, `TRITS_PER_BYTE`) se conserva en la API como nomenclatura de
esa fase futura, no como parte del formato v1.

## 4. tram.ko (módulo de bloque Rust, Fase 3)

- API `kernel::block::mq` (idéntica a `samples/rust/rnull.rs`, que ya carga en
  la VM del sandbox).
- `queue_rq` síncrono: itera los bios, extrae los vectores, map con
  `page_address` (x86_64 sin highmem), comprime/descomprime por slot.
- Almacén: slots por sector, mutex global, asignación GFP_KERNEL.
- `WritebackBackend` (trait): `Ninguno` (sin overflow, RAM pura) y `Ssd` (fallback
  a `/dev/sdb5`/backend de archivo) — off por defecto de forma segura.
- Sysfs: `tram/stat/…`, `tram/comp_algorithm`, `tram/disksize` — expuestos y
  auditable; swapon se conecta con `-p 100`.
- El módulo valida en VM: insmod → `dd` quick + `cmp` verosímil + swapon con
  archivo de 4 MB en memoria. Los IDs de ops y el LAYOUT del bios siguen el
  contrato de la Fase 0 (los canarios de la Fase 0 se reutilizan como
  pre-requisito).

## 5. portes clave de la Fase 0 (lecciones para el codec)

- `bit32`: gate de la VM con canario `rnull_mod` (builtin Rust) + `tram_oot_stub`
  (módulo Rust OOT): **AMBOS PASAN**. Eso fija que `tram.ko` es viable OOT sin
  recompilar el árbol.
- Config del árbol: `CONFIG_RUST=y`, `CONFIG_MODULE_SIG=y` (sin FORCE → OOT
  cargan con warning), `CONFIG_VIRTIO_*` irrelevante para el canario (se bypassa
  con initramfs autocontenido).
- El build OOT se verifica con `make M=<tree> LLVM=1` contra el árbol
  (`build-kernel.sh`).

## 6. Distribución a la comunidad

- Documentación de build (kernel + módulo + herramienta), fragmento de config
  `.config-fragment-tram`, firmware/ci/docker para ARM conveniente.
- El hub `swap.community` llega en una Fase posterior (fuera de este alcance).

## 7. Milestone del alcance actual

Fases 1-5 entregables: codec publicado y **benchmark gate verde** (v2, aprobado
2026-10-08), `tram-bench` reproducible, `tram.ko` **validado en VM** con
roundtrip y swapon, `tram-ctl` para operación, CI y guías. **No se instala en
el host local.**

## 8. Registro de cambios

- **2026-10-08 — gate v2 (aprobado por el gate humano, HITL)**. El gate v1 era
  inalcanzable para el codec real (razones en §1). Se renegocia: (a) corpus
  swap-real (17 clases de memoria anónima + cola incompresible + fuzz, generado
  determinista con xorshift64*); (b) umbrales absolutos anclados al propósito
  (ratio ≥ 1.50×, throughput ≥ 10 MB/s por núcleo). **Gate VERDE 3/3 runs**
  (ratio 2.12×, comp ~24 MB/s, decomp ~50 MB/s; roundtrip 17 items + 47 fuzz y
  determinismo OK). 3 criterios gate, todos verdes, con ruido de host acotado
  por umbrales absolutos.
- **2026-10-08 — fix `restore_payload` (bug cazado por el bench)**. La
  descompresión validaba `out_pos == dst.len()` (tamaño del búfer de salida)
  en vez del tamaño original del slot: con búfer sobredimensionado
  (`compress_bound(MAX_INPUT_LEN)`), típico del patrón kernel/swap, toda
  descompresión fallaba. Ahora `restore_payload(payload, dst, scratch,
  expected)` con `expected = input_len`; test de regresión
  `decompress_into_buffer_mayor_que_input`. 16/16 tests.
- **2026-10-08 — alineado §1/§3 con la implementación real**. §3 describe ahora
  el codec v1 tal cual (formato de slot con magic "TR", capa LZ estructural,
  etapa de entropía valores+Huffman, descompresión `restore_payload` y la
  corrección del gate v2); la visión original rANS ternario queda marcada como
  contexto/roadmap futuro, no como contrato de v1. Con esto el milestone §7 y
  la Fase 3 tienen una fuente de verdad coherente con el código.