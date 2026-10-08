#!/bin/sh
#
# vm-test-standalone.sh - Canario Fase 0 SIN virtio/9p.
#
# Construye un initramfs autocontenido (base de virtme-mkinitramfs) con los
# .ko embebidos y lo arranca en QEMU/KVM. No necesita virtio-pci ni rootfs
# externo: los ficheros viajan DENTRO del initramfs.
#
# Markers esperados en el log:
#   TRAM-BOOT-OK / TRAM-CANARY-INSMOD-OK / TRAM-CANARY-WRITE-OK
#   TRAM-CANARY-READ-OK / TRAM-CANARY-RMMOD-OK / TRAM-OOT-INSMOD-OK
#   TRAM-OOT-RMMOD-OK / TRAM-ALL-OK
#
# Uso:   sh infra/sandbox/vm-test-standalone.sh [LOG]
# (el binario qemu y virtme-mkinitramfs deben existir en la imagen POD).

set -u

LOG="${1:-/tmp/tram-standalone.log}"
: "${TREE:=/tree}"
: "${KO_OOT:=${TREE}/infra/oot-stub/tram_oot_stub.ko}"
: "${INIT_SRC:=${TREE}/infra/sandbox/init-canario.sh}"
KO_BUILTIN="${TREE}/drivers/block/rnull_mod.ko"
STAGE=/tmp/vmi-can
CPIO=/tmp/vmi-can.cpio
QEMU=/usr/bin/qemu-system-x86_64

command -v virtme-mkinitramfs >/dev/null 2>&1 || {
    echo "FALLO: falta virtme-mkinitramfs (paquete virtme-ng)" >&2
    exit 1
}
[ -f "${KO_BUILTIN}" ] || { echo "FALLO: no existe ${KO_BUILTIN}" >&2; exit 1; }
[ -f "${KO_OOT}" ] || { echo "FALLO: no existe ${KO_OOT} (haz make M=infra/oot-stub)" >&2; exit 1; }

rm -rf "${STAGE}"
virtme-mkinitramfs --rw --outfile /tmp/base.cpio || exit 1
mkdir -p "${STAGE}"
( cd "${STAGE}" && cpio -id < /tmp/base.cpio >/dev/null 2>&1 )

# Inyectar modulos + init canario + applets busybox necesarios.
mkdir -p "${STAGE}/root"
cp "${KO_BUILTIN}" "${KO_OOT}" "${STAGE}/root/"
cp "${INIT_SRC}" "${STAGE}/init"
chmod +x "${STAGE}/init"
for a in dd rmmod grep ls test '[' sync; do
    ln -sf busybox "${STAGE}/bin/${a}"
done

( cd "${STAGE}" && find . | cpio -o -H newc 2>/dev/null > "${CPIO}" )

echo "== standalone canary (sin virtio) ==" > "${LOG}"
"${QEMU}" -enable-kvm -m 768 -kernel "${TREE}/arch/x86/boot/bzImage" \
    -initrd "${CPIO}" \
    -append "console=ttyS0 rdinit=/init" \
    -nographic </dev/null >> "${LOG}" 2>&1

grep -aE "TRAM-\|rnull_mod:" "${LOG}"
ok=$(grep -ac "TRAM-ALL-OK" "${LOG}")
[ "${ok}" -ge 1 ] && echo "STANDALONE-CANARY: PASS" || echo "STANDALONE-CANARY: FAIL"
exit 0