#!/usr/bin/env bash
# Fase 0 — build de kernel + prueba en VM (secuencia completa).
set -euo pipefail
/infra/sandbox/build-kernel.sh
/infra/sandbox/vm-test.sh
