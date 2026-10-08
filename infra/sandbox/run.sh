#!/usr/bin/env bash
# Fase 0 — entrada del host: construye la imagen sandbox y ejecuta
# build/boot-test del kernel dentro del contenedor.
#
# Uso:
#   ./run.sh            # (default) build de imagen + build de kernel + VM test
#   ./run.sh imagen     # solo construir la imagen
#   ./run.sh build      # imagen + build de kernel (sin VM)
#   ./run.sh test       # imagen + VM test (requiere build previo)
#
# Variables:
#   TREE=/usr/src/linux-source-6.12   árbol de kernel a reconstruir
set -euo pipefail
cd "$(dirname "$0")"

TREE="${TREE:-/usr/src/linux-source-6.12}"
MODE="${1:-todo}"

if [ ! -d "$TREE" ]; then
    echo "ERROR: no existe el árbol de kernel: $TREE" >&2
    exit 1
fi
if [ ! -e /dev/kvm ]; then
    echo "AVISO: no hay /dev/kvm — la VM correrá en TCG (más lento)" >&2
fi

echo "== [1/2] imagen sandbox =="
podman build -t tram-sandbox .

if [ "$MODE" = "imagen" ]; then
    exit 0
fi

echo "== [2/2] ejecución en el contenedor (modo: $MODE) =="
RUN_ARGS=(
    --rm
    --security-opt label=disable
    -v "$TREE":/tree:rw
    -v "$(cd .. && pwd)":/infra:rw
    -e TREE=/tree
)
if [ -e /dev/kvm ]; then
    RUN_ARGS+=(--device /dev/kvm)
fi

case "$MODE" in
    build) podman run "${RUN_ARGS[@]}" tram-sandbox /infra/sandbox/build-kernel.sh ;;
    test)  podman run "${RUN_ARGS[@]}" tram-sandbox /infra/sandbox/vm-test.sh ;;
    todo)  podman run "${RUN_ARGS[@]}" tram-sandbox /infra/sandbox/build-and-test.sh ;;
    *)
        echo "modo desconocido: $MODE (imagen|build|test|todo)" >&2
        exit 2
        ;;
esac
