# estiba — Especificación técnica

Estado: en desarrollo. Fase 0 (kernel 6.12 + `CONFIG_RUST=y` + canarios)
**PASS en sandbox**. Fase 2b (codec + bench + gate v2) **PASS 2026-10-08**.
Fase 3 (`estiba.ko`) **en curso**.

---

## 1. Propósito y alcance

### 1.1 Qué es

estiba es un **dispositivo de bloque de RAM/swap comprimida para Linux escrito
en Rust** (equivalente a `zram`, con su propio códec). Comprime cada página de
4 KiB antes de guardarla en RAM y la descomprime al leerla.

### 1.2 Qué NO es (no-objetivos explícitos)

- **No** es un códec ternario. La idea original (trits balanceados + rANS +
  AVX2) se abandona: el ternario **no aporta ratio** (la información se
  conserva; log₃(256) ≈ 5.05 trits/byte no mejora los 8 bits/byte) y rANS es
  serial (AVX2 no acelera el núcleo). Detalle en §9.
- **No** es un códec algorítmicamente novedoso. LZ + entropía es un campo
  maduro (DEFLATE, zstd). **No se persigue superar a zstd en ratio.**
- **No** es un fork de `zram`/`zswap`/`zsmalloc` (esos son C).

### 1.3 Por qué merece existir (el hueco real)

El valor de estiba **no es el algoritmo, es la combinación**:

1. **Memory-safe en el kernel**: `zram`/`zswap`/`zsmalloc` son C, en un punto
   donde un bug de memoria = corrupción del sistema. El córtex de estiba es
   Rust `no_std`, sin `unsafe` en la lógica del códec.
2. **Determinismo y verificabilidad**: mismo bitstream en userspace y en el
   kernel, **paridad bit-exacta** (`module/estiba/parity/`), CRC16 por slot y
   un gate de benchmark reproducible.
3. **Reproducibilidad**: sandbox QEMU con kernel custom y banco de pruebas
   determinista (sin reloj de pared en las rutas de datos).

El hueco de contribución *algorítmica* —si algún día se ataca— no está en el
códec, sino en los **problemas de sistema** de la compresión de memoria
(§7, Fase 4): asignador de objetos comprimidos de tamaño variable (el infierno
de `zsmalloc`), writeback a disco, dedup entre páginas y política hot/cold.

### 1.4 Motivación de compresión (dónde gana el ratio)

- Las páginas de swap/RAM tienen mucha redundancia local: páginas limpias
  casi-sólo-ceros, bloques repetidos, texto con alfabeto pobre.
- **LZ4/LZO no tienen etapa de entropía**: emiten literales a 8 bits. La
  ganancia de estiba sobre ellos viene de **añadir codificación de entropía**
  (Huffman canónico) sobre los literales restantes tras el LZ — no del
  ternario.
- Coste: estiba **no** compite en MB/s con `zram`+LZ4; compite en RAM ahorrada
  con CPU aceptable (objetivo: paridad/superación de **LZO**).

### 1.5 Objetivos de aceptación (gate v2 — aprobado 2026-10-08)

> **Registro**: el gate v1 exigía ratio ≥ LZ4 y ≥ 1.10× LZO y throughput ≥ 40%
> de LZ4 sobre un corpus de 16 clases **sintéticas** (dominadas por runs de
> ceros, nada realistas). Era inalcanzable para el códec real (LZ+Huffman, con
> tokens de match de ~4 B y empaquetado bit-a-bit: ~24 MB/s). El gate v2 ancla
> en el **propósito** (swap comprimido rentable y que no ahogue la CPU) con
> umbrales absolutos, medidos con `estiba-bench` sobre un **corpus swap-real**
> (17 páginas de 4K de memoria anónima + cola incompresible + fuzz; mediana de
> 3 runs; host compartido ⇒ ±20-40% de ruido):

1. **Ratio**: media geométrica (entrada/salida) **≥ 1.50×** sobre el corpus.
   (Medido 2026-10-08: **2.12×**.)
2. **Throughput por núcleo** (páginas de 4K): compresión y descompresión
   **≥ 10 MB/s** cada una. (Medido: comp **~24 MB/s**, decomp **~50 MB/s**.)
3. **Determinismo**: `compress(página)` es función pura (misma entrada → mismo
   output bit a bit).
4. **Roundtrip bit-exacto**: `decompress(compress(x)) == x` para todo el corpus
   y el fuzzing determinista xorshift sembrado.
5. *(Referencia, no gate)*: cada run publica ratio y MB/s de LZ4/LZO (estiba
   queda a ~71%/69% del ratio y ~4-5% de los MB/s de LZ4), como contexto de
   evolución.

---

## 2. Arquitectura

```
estiba/                      ← workspace Rust (edition 2021, resolver 2)
├─ codec/                    ← crate "estiba-codec" (lib; no_std + feature alloc)
│  └─ src/
│     ├─ lib.rs              ← API: compress_into / decompress_into (+ alloc wrapper)
│     ├─ lz.rs               ← capa de matches (LZ4-ish, ventana 4 KB, hash 4 B)
│     ├─ values.rs           ← alfabeto de valores → símbolos 0..K-1
│     └─ huff.rs             ← Huffman canónico + bitstream MSB-first
├─ bench/                    ← crate "estiba-bench": corpus swap-real + baselines + gate
├─ module/estiba/            ← módulo de bloque OOT (estiba.ko), Rust
│  ├─ estiba.rs              ← driver: blk-mq + bio + codec + store
│  ├─ Makefile               ← obj-m := estiba.o
│  ├─ gen-codec.sh           ← vendorea codec/ dentro del módulo (1 .rs por módulo)
│  └─ parity/                ← harness de paridad bit-exacta codec ⇔ port
├─ infra/sandbox/            ← QEMU/KVM + kernel custom + canarios (Fase 0)
└─ docs/                     ← esta SPEC + guías
```

`estiba-ctl` (operación estilo `zramctl`) es un entregable de Fase 5 (aún no
existe).

### 2.1 Slots y flujo de datos

```
página virtual 4 KiB
  write:  página          → codec::compress   → store[slot] = bytes comprimidos
  read :  store[slot]     → codec::decompress → página
```

Almacén: pool de páginas que crece hasta `disksize`; tabla de slots + mutex.

---

## 3. estiba-codec (el corazón)

Implementación real: **LZ estilo LZ4 + Huffman canónico sobre el alfabeto de
valores**. La ventaja frente a LZ4/LZO es la **etapa de entropía** que ellos no
tienen.

### 3.1 Formato de un slot comprimido

```
offset  tamaño  campo
0       2       magic 0x45 0x53 ("ES", estiba)
2       1       versión (3)
3       1       flags (bit0 = store_raw)
4       2       input_len  (longitud original, ≤ 65535, u16 LE)
6       2       output_len (longitud del payload, u16 LE)
8       2       crc16 (polinomio 0x8005, reflejado) sobre el payload
10      n       payload
```

Payload: `store_raw` (flag bit0) = input sin tocar (`output_len == input_len`);
normal = `tokens LZ` + `sección de entropía`. Si el slot comprimido no mejora,
se guarda **RAW**; el slot nunca supera `input_len + 10 + 1`. `n == 0` se
serializa como slot RAW vacío.

### 3.2 LZ (capa de matches, `lz.rs`)

- Ventana `LZ_WINDOW = 0xFFFE`; tabla hash de 4 bytes (`HASH_TAB_LEN = 4096`,
  SplitMix) con **1 candidato por head** (como LZ4), sin cadenas.
- Greedy hacia delante; match mínimo 4, máximo 255; se emiten como (u16
  offset+1, u8 longitud-4).
- Token stream (bytes, sin entropía):
  - `0b0LLLLLLL` (1..=127): corrida de L literales (sus bytes NO van al stream:
    los consume la capa de entropía);
  - `0b1MMMMMMM` (1..=127): M matches consecutivos, cada uno 3 B;
  - `0xFF`: fin (parseo **estructural**; `0xFF` dentro de offsets es legal).
- Determinista: misma entrada → mismo token stream bit a bit.

### 3.3 Entropía: valores + Huffman canónico

1. **Alfabeto de valores**: los K valores distintos de los literales se ordenan
   (count desc, valor asc); cada literal se mapea a símbolo 0..K-1. `K == 1`
   (página de un solo byte, típico de swap limpio) ⇒ 0 bits de payload.
2. **Huffman canónico estático** (longitud ≤ `C_MAX = 12`; histogramas
   patológicos → slot RAW).
3. Cabecera de entropía: `K (1 B) | alfabeto (K B) | longitudes (K B)` +
   **bitstream MSB-first** de los símbolos de los literales (en orden de
   tokens). **K se guarda 0-based** (`K-1`, byte 0..=255): el `K as u8` de v1
   desbordaba a 0 cuando el alfabeto cubría los 256 valores.

`build_entropy` corre sobre el buffer `scratch` (~`compress_bound`) que aporta
el caller — **sin alloc en el camino caliente** y con `#![forbid(unsafe_code)]`.

> **Bug v1 → v2 (2026-10-08, cazado por `module/estiba/parity/`)**: el corpus
> incluye `(0..1024).map(|i| i as u8)` (los 256 valores → `K = 256`). El header
> v1 `dst[0] = k as u8` desbordaba a `0`, y el orden canónico de
> `huff::code_tables` (`(0..k as u8)`) también (`0..0` ⇒ códigos a 0). El slot
> quedaba ilegible (`k==0 → Err(Slot)`). Fix: (a) K **0-based**; (b) asignación
> canónica por doble bucle (longitud asc, símbolo asc), equivalente al sort
> estable DEFLATE y **sin `Vec`** (lo que el kernel necesita). La asignación
> vive ya en `codec/src/huff.rs`; `gen-codec.sh` copia el fuente tal cual, y
> `make parity` valida bit-exactitud codec ⇔ port en todo el corpus (incl. K=256).

### 3.4 Descompresión (`lib.rs::restore_payload`)

1. Separa tokens (fin estructural) de la sección de entropía.
2. Lee K/alfabeto/longitudes; decodifica el bitstream (tabla canónica,
   `BitReader` bit a bit).
3. Emite: recorre tokens empalmando literales y matches; los matches se copian
   **byte a byte hacia delante** (permite matches solapados RLE-like,
   `off < len`). Los límites se validan contra `expected = input_len`, NO contra
   el tamaño del búfer de salida (patrón kernel/swap; regresión
   `decompress_into_buffer_mayor_que_input`).

### 3.5 Verificación bit-exacta

`module/estiba/parity/` compila el códec (crate, con `std`) **y** el port
`no_std` vendored (`gen_codec/`) y compara, sobre un corpus determinista:
bitstream comprimido byte a byte, roundtrip de cada motor y **cross-decoding**
(el port lee el slot del crate y viceversa). Ejecutar: `make parity`.

---

## 4. estiba.ko (módulo de bloque Rust, Fase 3 — en curso)

- API `kernel::block::mq` (como `samples/rust/rnull.rs`, que ya carga en la VM).
- `queue_rq` síncrono: itera los bios, extrae vectores, `kmap_local_page`,
  comprime/descomprime por slot. Todo fallo se audita y **siempre** se completa
  el request (OK o `BLK_STS_IOERR`); nunca `?` que lo dejaría colgado.
- Almacén: `KVec<Option<KVec<u8>>>` + scratch + tabla hash por dispositivo
  (`GFP_KERNEL`).
- Backend de writeback (trait): `Ninguno` (RAM pura) y `Ssd` — off por defecto.
- Se valida en VM: insmod → `dd` + `cmp` + `swapon` sobre un archivo en memoria.

---

## 5. Fase 0 (sandbox) — lecciones

- `estiba_oot_stub` (módulo Rust **OOT**) carga en la VM ⇒ `estiba.ko` es viable
  fuera del árbol sin recompilar el kernel.
- Config: `CONFIG_RUST=y`; `CONFIG_MODULE_SIG=y` (sin FORCE → los OOT cargan con
  warning). El canario se bypassa con initramfs autocontenido.
- El build OOT se verifica con `make M=<tree>` contra el árbol (`build-kernel.sh`).

---

## 6. Distribución a la comunidad

- Guías de build (kernel + módulo + herramienta), fragmento de config
  `config-fragment-estiba`, CI (tests del códec + `make parity`).
- Licencia dual: **GPL-2.0** para el módulo/kernel, **MIT** para el
  códec/userspace.

---

## 7. Roadmap / milestones

| Fase | Entregable | Estado |
|---|---|---|
| 0 | Sandbox QEMU + kernel custom `CONFIG_RUST=y` + canarios | ✅ |
| 1 | Códec (LZ + entropía, CRC, RAW) + formato | ✅ |
| 2 | Verificación (parity bit-exacta + fuzz) + bench + gate v2 | ✅ |
| 3 | `estiba.ko` validado en VM (roundtrip + swapon) | 🚧 |
| 4 | **Problemas de sistema** (donde hay margen de aporte real): páginas idénticas/cero, asignador de objetos de tamaño variable (≈`zsmalloc`), writeback, dedup, aging hot/cold | ⬜ |
| 5 | `estiba-ctl`, CI, distribución | ⬜ |

**No se instala en el host local** hasta que la VM pase todos los marcadores.

---

## 8. Registro de cambios

- **2026-10-08 — replanteo y renombrado `tRAM` → `estiba`**. Se abandona la
  premisa ternaria (nunca aportó ratio; ver §9) y el proyecto se reposiciona
  como **zram en Rust, memory-safe y verificado** (el aporte es de ingeniería
  de sistemas, no de algoritmo). Cambios: magic del slot `"TR"` → `"ES"`,
  `VERSION` 2 → 3; descripciones de crates alineadas; SPEC reescrita sin la
  visión rANS/AVX2; roadmap con la Fase 4 (problemas de sistema) como el único
  frente con margen de contribución real.
- **2026-10-08 — fix K=256 (`VERSION` 1 → 2)**. Header K **0-based** y
  asignación canónica sin `Vec`; bug cazado por el harness de paridad. Ver §3.3.
- **2026-10-08 — gate v2**. Renegociado a umbrales absolutos sobre corpus
  swap-real; **VERDE 3/3** (ratio 2.12×, comp ~24 MB/s, decomp ~50 MB/s). §1.5.
- **2026-10-08 — fix `restore_payload`**. La descompresión validaba contra el
  tamaño del búfer de salida en vez de `input_len`; con búfer sobredimensionado
  (patrón kernel/swap) fallaba. Regresión
  `decompress_into_buffer_mayor_que_input`.

---

## 9. Nota histórica: por qué no hay ternario

La visión original codificaba cada byte como trits balanceados y aplicaba rANS
sobre `{-1, 0, +1}` con packing AVX2. Se descarta por motivos técnicos, no de
esfuerzo:

1. **Sin ganancia de ratio**: representar 256 valores exige 6 trits
   (3⁶ = 729 ≥ 256) = 9.5 bits; el ternario no crea información. La ganancia
   real (8-20% sobre LZ4) venía de *añadir entropía*, no del ternario — y eso se
   logra igual con Huffman/rANS **sobre bytes**.
2. **rANS es serial**: el estado de un símbolo alimenta al siguiente; AVX2 no
   acelera el núcleo (solo el álgebra booleana de planos de trits, que no es el
   cuello de botella).
3. **Páginas pequeñas**: en 4 KiB, la tabla de frecuencias y el *flush* del
   estado rANS pesan demasiado; LZ4/LZO (sin modelo) dominan este régimen.
4. **zstd ya lo tiene**: su etapa FSE es *tANS* (familia ANS). Un "rANS sobre
   bytes" sería reimplementar peor parte de zstd.

El códec ternario sí es una herramienta válida en **otros** dominios (p.ej.
redes BitNet de pesos `{-1,0,+1}`), pero no en compresión de memoria.
