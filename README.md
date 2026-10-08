# tRAM — swap y zram con compresión ternaria (Rust)

Módulo de bloque de Linux escrito en **Rust** que comprime la RAM con el
motor ternario (AVX2) en lugar de LZO/LZ4:

- **zram ternaria**: dispositivo de bloque de RAM comprimida
  (`/dev/tram0`), sysfs estilo zram.
- **swap ternario**: `swapon /dev/tram0 -p 100` como swap primario, con
  overflow a SSD vía el trait `WritebackBackend`.

Proyecto comunitario: **GPL-2.0** para el módulo/kernel (`LICENSE-GPL`),
**MIT** para el codec/userspace (`LICENSE-MIT`).

> **Estado**: Fase 0 — reconstrucción del kernel custom con
> `CONFIG_RUST=y` y validación en sandbox (ver `infra/sandbox/`).

## Requisitos

Los módulos Rust fuera del árbol solo cargan en kernels compilados con
`CONFIG_RUST=y` (y `CONFIG_MODVERSIONS=n`). El fragmento de config y las
instrucciones de rebuild se publicarán con la Fase 7.

## Sandbox (Fase 0)

Nada se instala en el host hasta que la VM pase todos los marcadores:

```bash
cd infra/sandbox
./run.sh          # imagen + build del kernel + boot-test en VM (QEMU/KVM)
./run.sh imagen   # solo la imagen
./run.sh build    # build del kernel
./run.sh test     # solo el boot-test (requiere build previo)
```
