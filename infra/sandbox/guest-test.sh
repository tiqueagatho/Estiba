#!/bin/sh
# Corre DENTRO de la VM (virtme-ng). Emite marcadores ESTIBA-* que el host
# hace grep en la salida. Nunca aborta sin intentar poweroff.
set -u

fail() {
    echo "$1"
    poweroff -f 2>/dev/null
    sleep 5
    exit 1
}

echo "== guest uname =="
uname -a
case "$(uname -r)" in
    *-precompiladoIA-rust) echo "ESTIBA-BOOT-OK" ;;
    *) fail "ESTIBA-BOOT-FAIL uname=$(uname -r)" ;;
esac

echo "== guest canario rnull_mod.ko (in-tree, Rust) =="
KO=/tree/drivers/block/rnull_mod.ko
[ -f "$KO" ] || fail "ESTIBA-CANARY-FAIL missing:$KO"
insmod "$KO" || fail "ESTIBA-CANARY-FAIL insmod"
[ -b /dev/rnullb0 ] || { rmmod rnull_mod 2>/dev/null; fail "ESTIBA-CANARY-FAIL nodev"; }
dd if=/dev/zero of=/dev/rnullb0 bs=1M count=4 conv=fsync 2>/dev/null \
    || { rmmod rnull_mod 2>/dev/null; fail "ESTIBA-CANARY-FAIL write"; }
dd if=/dev/rnullb0 of=/dev/null bs=1M count=4 2>/dev/null \
    || { rmmod rnull_mod 2>/dev/null; fail "ESTIBA-CANARY-FAIL read"; }
rmmod rnull_mod || fail "ESTIBA-CANARY-FAIL rmmod"
echo "ESTIBA-CANARY-OK"

echo "== guest stub OOT estiba_oot_stub.ko (externo, Rust) =="
STUBKO=/infra/oot-stub/estiba_oot_stub.ko
[ -f "$STUBKO" ] || fail "ESTIBA-OOT-FAIL missing:$STUBKO"
insmod "$STUBKO" || fail "ESTIBA-OOT-FAIL insmod"
rmmod estiba_oot_stub || fail "ESTIBA-OOT-FAIL rmmod"
echo "ESTIBA-OOT-OK"

echo "ESTIBA-ALL-OK"
poweroff -f
