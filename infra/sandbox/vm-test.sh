#!/usr/bin/env bash
# Fase 0 — boot-test del kernel recién construido en una VM QEMU/KVM
# (virtme-ng) SIN instalar nada en el host. Prueba:
#   1. el kernel nuevo arranca con el sufijo -rust
#   2. el canario in-tree rnull.ko (Rust) insmod/dd/rmmod
#   3. el stub OOT estiba_oot_stub.ko (módulo Rust externo) insmod/rmmod
set -euo pipefail
TREE="${TREE:-/tree}"
cd "$TREE"

REL="$(make -s kernelrelease)"
echo "== kernelrelease: $REL =="

echo "== OOT rust stub: build (make -C tree M=...) =="
make -C "$TREE" M=/infra/oot-stub modules
test -f /infra/oot-stub/estiba_oot_stub.ko

echo "== VM: virtme-run (QEMU/KVM) =="
LOG=/tmp/vm-test.log
if ! virtme-run --kdir "$TREE" --mods auto --memory 768 --show-boot-console \
        --script-sh "sh /infra/sandbox/guest-test.sh" 2>&1 | tee "$LOG"; then
    echo "== virtme-run devolvió ≠0 (revisar log) =="
fi

# Los marcadores son la fuente de verdad (poweroff corta el exit code).
for m in ESTIBA-BOOT-OK ESTIBA-CANARY-OK ESTIBA-OOT-OK ESTIBA-ALL-OK; do
    if ! grep -q "$m" "$LOG"; then
        echo "== VM TEST FAIL: falta marcador $m =="
        exit 1
    fi
done
echo "== VM TEST PASS =="
