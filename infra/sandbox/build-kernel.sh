#!/usr/bin/env bash
# Fase 0 — config + build del kernel dentro del contenedor sandbox.
# Idempotente: se puede re-ejecutar tras un fallo.
set -euo pipefail
TREE="${TREE:-/tree}"
cd "$TREE"

echo "== [1/5] ediciones de config =="
./scripts/config -d MODVERSIONS           # requisito de CONFIG_RUST
./scripts/config -e RUST                  # módulos Rust en el kernel
./scripts/config -m BLK_DEV_RUST_NULL      # canario rnull.ko (tristate)
./scripts/config --set-str LOCALVERSION "-precompiladoIA-rust"
# requisitos de virtme-ng (boot de la VM por 9p)
./scripts/config -e 9P_FS
./scripts/config -e NET_9P
./scripts/config -e NET_9P_VIRTIO
./scripts/config -e BINFMT_SCRIPT
./scripts/config -e DEVTMPFS
./scripts/config -e DEVTMPFS_MOUNT
./scripts/config -e TMPFS

echo "== [2/5] olddefconfig =="
make olddefconfig

echo "== [3/5] rustavailable (gate del toolchain) =="
make rustavailable

echo "== [4/5] build del kernel =="
make -j"$(nproc)"

REL="$(make -s kernelrelease)"
echo "ESTIBA-KERNEL-BUILD-OK kernelrelease=$REL"
test -f "$TREE/arch/x86/boot/bzImage"
test -f "$TREE/drivers/block/rnull_mod.ko"

echo "== [5/5] módulo estiba.ko (OOT, Rust) =="
# El target `all` del Makefile del módulo vendorea el códec (gen-codec.sh)
# y luego invoca kbuild: make -C $KDIR M=<módulo> modules.
make -C /estiba/module/estiba KDIR="$TREE"
test -f /estiba/module/estiba/estiba.ko
echo "ESTIBA-KO-BUILD-OK"
