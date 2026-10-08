# Contribuir a estiba

¡Gracias por el interés! estiba es un proyecto **experimental y educativo**:
un dispositivo de bloque de RAM comprimida en Rust para Linux. Todo el mundo
puede usarlo, estudiarlo y mejorarlo.

## Filosofía

- **Honestidad técnica**: no presumimos de nada que no exista. El objetivo no
  es ganar a `zstd` en ratio, sino aportar un backend **memory-safe,
  determinista y verificable** (el aporte es de ingeniería de sistemas).
- **Verificación primero**: cualquier cambio en el códec debe pasar la
  paridad bit-exacta (`make parity`) y los tests.
- **Nada en el host**: el módulo se compila y se prueba **dentro de una VM**
  (sandbox QEMU/KVM). No se instala en el sistema anfitrión.

## Requisitos

- Rust **1.98+** (`cargo`).
- Para el módulo: un kernel Linux **6.12** con `CONFIG_RUST=y` (el sandbox lo
  construye solo).
- `podman` (o `docker`) con `/dev/kvm` para la VM.

## Cómo construir y probar

```bash
# Códec (userspace) + paridad bit-exacta con el port del kernel
cd codec && cargo test --features alloc
cd ../ctl && cargo test
cd ../module/estiba && make parity

# Benchmark (gate de ratio/throughput sobre corpus swap-real)
cd ../../bench && cargo run --release

# Todo el ciclo en una VM (kernel custom + estiba.ko + roundtrip + swapon)
cd ../infra/sandbox
./run.sh            # imagen + build del kernel + VM test
./run.sh build      # solo el kernel (+ estiba.ko)
./run.sh test       # solo la VM (requiere build previo)
```

El resultado de la VM se lee por **marcadores** en el log (`ESTIBA-ALL-OK`).

## Dónde ayudar (ideas)

La **Fase 4** está a medias y es el frente con más margen:

- **Asignador de objetos de tamaño variable** (estilo `zsmalloc`): agrupar los
  payloads por clases de tamaño para reducir fragmentación y sobrecarga.
- **Writeback a disco**: volcar páginas comprimidas cuando la RAM se llena
  (el `trait` de backend ya está esbozado).
- **Aging hot/cold**: promover/expulsar páginas según su frecuencia de uso.
- **Dedup**: hoy es por contenido comprimido; se puede estudiar dedup por
  página completa antes de comprimir.

Otras ideas:

- Portar el sandbox a otras arquitecturas / kernels.
- Más casos en el corpus de benchmark y en el fuzz.
- Exponer estadísticas del códec por `sysfs`/`debugfs` (el crate `kernel` 6.12
  aún no lo facilita; se agradece una vía segura).
- Documentación y traducción.

## Estilo

- Errores explícitos; **sin `unwrap`/`panic!` en producción**.
- El códec es `no_std` y **sin allocación en el camino caliente**.
- El driver sigue las convenciones de Rust-for-Linux (GPL-2.0).
- Formato: `cargo fmt`. Lint del códec: `cargo test` debe quedar verde.

## Licencia

- `module/` e `infra/`: **GPL-2.0** (consistente con el kernel).
- `codec/`, `bench/`, `ctl/`: **MIT**.

Al enviar un cambio aceptas publicarlo bajo estas licencias.
