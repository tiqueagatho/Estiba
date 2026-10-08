#!/usr/bin/env bash
# Fase 3 — build de estiba.ko + prueba en VM QEMU/KVM con un initramfs
# busybox AUTOCONTENIDO (sin virtio/9p ni rootfs externo: los .ko y el init
# viajan DENTRO del initramfs). Nada se instala en el host.
#
# Prueba:
#   1. el kernel nuevo arranca con el sufijo -rust
#   2. canario in-tree rnull_mod.ko (Rust) insmod/dd/rmmod
#   3. stub OOT estiba_oot_stub.ko (módulo Rust externo) insmod/rmmod
#   4. estiba.ko: insmod + roundtrip dd/cmp (ceros y aleatorio) + swapon
set -euo pipefail
TREE="${TREE:-/tree}"
cd "$TREE"

REL="$(make -s kernelrelease)"
echo "== kernelrelease: $REL =="

echo "== OOT rust stub: build (make -C tree M=...) =="
make -C "$TREE" M=/infra/oot-stub modules
test -f /infra/oot-stub/estiba_oot_stub.ko

echo "== estiba.ko: build (módulo de bloque comprimido, Fase 3) =="
make -C /estiba/module/estiba KDIR="$TREE"
test -f /estiba/module/estiba/estiba.ko

echo "== initramfs busybox =="
STAGE=/tmp/estiba-initramfs
rm -rf "$STAGE"
mkdir -p "$STAGE"/bin "$STAGE"/root "$STAGE"/proc "$STAGE"/sys "$STAGE"/dev "$STAGE"/tmp
cp /bin/busybox "$STAGE/bin/busybox"
for a in sh mount umount insmod rmmod ls dd cmp mkswap swapon swapoff \
         poweroff sleep cat echo grep uname sync; do
    ln -sf /bin/busybox "$STAGE/bin/$a"
done
cp "$TREE/drivers/block/rnull_mod.ko"       "$STAGE/root/rnull_mod.ko"
cp /infra/oot-stub/estiba_oot_stub.ko       "$STAGE/root/estiba_oot_stub.ko"
cp /estiba/module/estiba/estiba.ko          "$STAGE/root/estiba.ko"
cp /infra/sandbox/guest-test.sh             "$STAGE/init"
chmod +x "$STAGE/init"
( cd "$STAGE" && find . -print0 | cpio --null -o -H newc 2>/dev/null ) > /tmp/estiba-initramfs.cpio
test -s /tmp/estiba-initramfs.cpio

echo "== VM: QEMU/KVM =="
LOG=/infra/sandbox/.vm-test.log
: > "$LOG"
qemu-system-x86_64 -enable-kvm -m 1024 -smp 1 \
    -kernel "$TREE/arch/x86/boot/bzImage" \
    -initrd /tmp/estiba-initramfs.cpio \
    -append "console=ttyS0 rdinit=/init panic=-1" \
    -nographic -no-reboot < /dev/null > "$LOG" 2>&1 || true

# Los marcadores son la fuente de verdad (poweroff corta el exit code).
for m in ESTIBA-BOOT-OK ESTIBA-CANARY-OK ESTIBA-OOT-OK ESTIBA-KO-OK ESTIBA-SWAP-OK ESTIBA-ALL-OK; do
    if ! grep -aq "$m" "$LOG"; then
        echo "== VM TEST FAIL: falta marcador $m =="
        echo "--- log de la VM (/infra/sandbox/.vm-test.log) ---"
        cat "$LOG"
        exit 1
    fi
done
echo "== VM TEST PASS =="
