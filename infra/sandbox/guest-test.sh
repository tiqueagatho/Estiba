#!/bin/sh
# Init de la VM (initramfs busybox, sin virtio/9p ni rootfs externo). Corre
# como `rdinit=/init`. Monta los seudofs, valida los canarios y el módulo
# estiba.ko (roundtrip + swapon) y emite marcadores ESTIBA-* que el host lee.
# Nunca aborta sin intentar apagar.
export PATH=/bin
set -u

fail() {
    echo "$1"
    poweroff -f 2>/dev/null
    sleep 5
    exit 1
}

mount -t proc   proc   /proc 2>/dev/null
mount -t sysfs  sysfs  /sys  2>/dev/null
mount -t devtmpfs devtmpfs /dev 2>/dev/null

echo "== guest uname =="
uname -a
case "$(uname -r)" in
    *-precompiladoIA-rust) echo "ESTIBA-BOOT-OK" ;;
    *) fail "ESTIBA-BOOT-FAIL uname=$(uname -r)" ;;
esac

echo "== canario rnull_mod.ko (in-tree, Rust) =="
KO=/root/rnull_mod.ko
[ -f "$KO" ] || fail "ESTIBA-CANARY-FAIL missing:$KO"
insmod "$KO" || fail "ESTIBA-CANARY-FAIL insmod"
[ -b /dev/rnullb0 ] || { rmmod rnull_mod 2>/dev/null; fail "ESTIBA-CANARY-FAIL nodev"; }
dd if=/dev/zero of=/dev/rnullb0 bs=1M count=4 conv=fsync 2>/dev/null \
    || { rmmod rnull_mod 2>/dev/null; fail "ESTIBA-CANARY-FAIL write"; }
dd if=/dev/rnullb0 of=/dev/null bs=1M count=4 2>/dev/null \
    || { rmmod rnull_mod 2>/dev/null; fail "ESTIBA-CANARY-FAIL read"; }
rmmod rnull_mod || fail "ESTIBA-CANARY-FAIL rmmod"
echo "ESTIBA-CANARY-OK"

echo "== stub OOT estiba_oot_stub.ko (externo, Rust) =="
STUB=/root/estiba_oot_stub.ko
[ -f "$STUB" ] || fail "ESTIBA-OOT-FAIL missing:$STUB"
insmod "$STUB" || fail "ESTIBA-OOT-FAIL insmod"
rmmod estiba_oot_stub || fail "ESTIBA-OOT-FAIL rmmod"
echo "ESTIBA-OOT-OK"

echo "== estiba.ko (dispositivo de bloque comprimido, Fase 3) =="
KO2=/root/estiba.ko
[ -f "$KO2" ] || fail "ESTIBA-KO-FAIL missing:$KO2"
insmod "$KO2" || fail "ESTIBA-KO-FAIL insmod"
[ -b /dev/estiba0 ] || { rmmod estiba 2>/dev/null; fail "ESTIBA-KO-FAIL nodev"; }

# roundtrip 1: ceros (fast-path K=1 del códec)
dd if=/dev/zero of=/tmp/z bs=4096 count=1024 2>/dev/null
dd if=/tmp/z of=/dev/estiba0 bs=4096 count=1024 conv=fsync 2>/dev/null \
    || { rmmod estiba 2>/dev/null; fail "ESTIBA-KO-FAIL write-zeros"; }
sync
echo 3 > /proc/sys/vm/drop_caches 2>/dev/null
dd if=/dev/estiba0 of=/tmp/zo bs=4096 count=1024 2>/dev/null \
    || { rmmod estiba 2>/dev/null; fail "ESTIBA-KO-FAIL read-zeros"; }
cmp /tmp/z /tmp/zo || { rmmod estiba 2>/dev/null; fail "ESTIBA-KO-FAIL cmp-zeros"; }

# roundtrip 2: pseudoaleatorio (fuerza RAW / K grande)
dd if=/dev/urandom of=/tmp/r bs=4096 count=1024 2>/dev/null
dd if=/tmp/r of=/dev/estiba0 bs=4096 count=1024 conv=fsync 2>/dev/null \
    || { rmmod estiba 2>/dev/null; fail "ESTIBA-KO-FAIL write-rand"; }
sync
echo 3 > /proc/sys/vm/drop_caches 2>/dev/null
dd if=/dev/estiba0 of=/tmp/ro bs=4096 count=1024 2>/dev/null \
    || { rmmod estiba 2>/dev/null; fail "ESTIBA-KO-FAIL read-rand"; }
cmp /tmp/r /tmp/ro || { rmmod estiba 2>/dev/null; fail "ESTIBA-KO-FAIL cmp-rand"; }
echo "ESTIBA-KO-OK"

echo "== swapon /dev/estiba0 =="
mkswap /dev/estiba0 >/dev/null 2>&1 || { rmmod estiba 2>/dev/null; fail "ESTIBA-SWAP-FAIL mkswap"; }
swapon /dev/estiba0 || { rmmod estiba 2>/dev/null; fail "ESTIBA-SWAP-FAIL swapon"; }
cat /proc/swaps
swapoff /dev/estiba0 || fail "ESTIBA-SWAP-FAIL swapoff"
echo "ESTIBA-SWAP-OK"
rmmod estiba || fail "ESTIBA-KO-FAIL rmmod"

echo "ESTIBA-ALL-OK"
sync
poweroff -f
