# estiba — comprime lo que cabe en tu RAM

> Dispositivo de bloque de **RAM comprimida** para Linux, escrito en **Rust**,
> con su **propio compresor** (nada de LZO/LZ4). Proyecto comunitario,
> experimental y educativo.

## ¿Qué es esto, en cristiano?

Imagina tu memoria RAM como un **escritorio** donde tienes todos los papeles a
mano. Cuando el escritorio se llena, Linux empieza a guardar papeles en un
cajón (el disco), que es enorme pero **lentísimo**. Todo lo que se mueve al
cajón tarda muchísimo en volver.

La idea de estiba es **apretar los papeles antes de dejarlos en el escritorio**:

- Linux le pide a un "disco de mentira" (`/dev/estiba0`) que guarde una hoja de
  4 KiB.
- estiba la **comprime** (hace la maleta a vacío) y guarda la maletita en la
  RAM que le queda libre.
- Cuando Linux necesita la hoja de nuevo, estiba la **descomprime** al vuelo.

Resultado: cabe más en el escritorio y se toca menos el cajón lento.

Esto no es nuevo en sí: **Android ya lo usa** (se llama *zram*). Lo diferencial
de estiba **no es el algoritmo** (LZ + entropía es un campo maduro; no pretende
superar a zstd), sino *cómo* está hecho:

- todo el dispositivo está escrito en **Rust**, un lenguaje pensado para que los
  fallos de memoria —la pesadilla de los módulos del kernel— sean difíciles de
  cometer;
- su compresor es **propio y determinista**, con un **harness de paridad
  bit-exacta** que garantiza que comprime igual en userspace y dentro del
  kernel.

## ¿Qué tiene de especial?

| Idea | Qué significa |
|---|---|
| Compresor propio | Dos etapas: encuentra trozos repetidos y luego *aprieta* los símbolos al límite de la entropía (8 bits de media → menos). |
| Determinismo | Dos máquinas comprimiendo lo mismo producen **exactamente los mismos bytes**. |
| Honestidad a prueba de datos | Cada página comprimida lleva su **checksum (CRC)**: si algo se corrompe, se detecta al leer. Si comprimir no merece la pena, la página se guarda tal cual. |
| Sin gasto extra de RAM | El camino caliente (comprimir/descomprimir) **no reserva memoria nueva**: usa búferes que el dueño del dispositivo le presta. |
| Verificación bit-exacta | Un "harness de paridad" compara el codec compilado para usuarios con el que corre dentro del kernel: **byte a byte**. |

## ⚠️ Estado honesto

| Etapa | Estado |
|---|---|
| Codec + formato (v3) | ✅ Validado: tests bit-exactos + benchmark con páginas de swap reales (gate verde 3/3). |
| Kernel custom (sandbox) | ✅ Compila y arranca en VM con `CONFIG_RUST=y` |
| Driver `estiba.ko` | 🚧 **En desarrollo** (Fase 3): aún no validado en VM |
| Uso en un equipo real | ❌ **Fuera de alcance por ahora** |

**No lo uses todavía como sistema de swap de un equipo de verdad.** Funciona
en un kernel que tú mismo compilas con `CONFIG_RUST=y`; los kernels de las
distribuciones normales **no** lo cargan. Esto es un proyecto para aprender,
experimentar y ayudarlo a crecer.

## Cómo funciona (un pelín más técnico)

Cada página de 4 KiB que Linux escribe en `/dev/estiba0` se convierte en un
"slot" con una cabecera de 10 bytes (magia `ES`, versión, banderas, tamaños,
CRC) y una carga comprimida:

1. **LZ**: busca repeticiones (parecido a cómo encoge un ZIP básico).
2. **Entropía**: a lo que queda ya repetido-deducido, le codifica los valores
   con códigos de longitud variable según su frecuencia (la parte "inteligente"
   que aporta el Huffman sobre LZO/LZ4).
3. Si el resultado no cabe mejor que el original → **RAW** (se guarda sin más).

Detalles técnicos completos en [`docs/SPEC-estiba.md`](docs/SPEC-estiba.md).

## Cómo probarlo desde el código

```bash
# 1) Pruebas del codec (17 tests, incluye el caso de las 256 frecuencias)
cd codec && cargo test --features alloc

# 2) Harness de paridad bit-exacta: el codec "de usuarios" vs el que irá
#    dentro del kernel
cd module/estiba && make parity          # genera la copia vendored y compara bitstreams

# 3) Benchmark con corpus de swap real + gate de aceptación
#    (ver bench/README o SPEC §1)
```

## Estructura del repo

```
codec/               # compresor (librería no_std + feature alloc para userspace)
module/estiba/       # estiba.ko: módulo de bloque del kernel (Rust OOT)
  ├── gen-codec.sh   # genera una copia del codec válida dentro de un módulo
  ├── parity/        # harness de paridad bit-exacta codec ⇔ kernel
  └── estiba.rs      # el driver (Fase 3, en desarrollo)
bench/               # estiba-bench: benchmark reproducible + gate (ratio/throughput)
infra/sandbox/       # sandbox QEMU/KVM: kernel custom + canarios (Fase 0)
docs/SPEC-estiba.md  # especificación técnica detallada + registro de cambios
```

## Licencia

- **Módulo/kernel** (`module/`, `infra/`): **GPL-2.0** — ver `LICENSE-GPL`.
- **Codec y userspace** (`codec/`, `bench/`): **MIT** — ver `LICENSE-MIT`.

Contribuciones bienvenidas: issues, tests, relectura de la SPEC, o ayuda con
el driver.