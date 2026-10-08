#!/bin/sh
# gen-codec.sh — vendorea el codec del crate codec/ al módulo del kernel.
#
# kbuild 6.12 solo compila UN archivo .rs por módulo OOT, así que
# module/estiba/gen_codec/ aloja una copia GENERADA de codec/src con las únicas
# transformaciones necesarias para vivir dentro de un módulo del kernel:
#
#   1) Se suprimen los atributos de crate raíz que kbuild ya impone
#      (`-Zcrate-attr=no_std`) y el `#![forbid(unsafe_code)]` (el driver que lo
#      engloba es obviamente unsafe; la lógica del codec sigue siendo pura y
#      sin alocación en el caliente).
#   2) Se elimina `extern crate alloc;` y `use alloc::vec::Vec;`: el núcleo no
#      expone la crate `alloc` (--sysroot=/dev/null), los wrappers
#      `#[cfg(feature="alloc")]` quedan inertes (feature nunca activada) y los
#      `#[cfg(test)]` no se compilan en kernel.
#   3) Los tests del codec viven en `crate::` (y en feature=alloc) y no
#      compilan dentro de un módulo (crate:: = crate raíz); se truncan desde
#      el primer `#[cfg(test)]`.
#
# Nota (v2 format): la asignación canónica de códigos Huffman ya es un doble
# bucle sin alloc dentro del propio codec/ (nada que reescribir aquí); la
# equivalencia bit-exacta codec ⇔ port se garantiza con el harness
# module/estiba/parity/.
#
set -eu

CODEC_SRC=../../codec/src
OUT=gen_codec

rm -rf "$OUT"
mkdir -p "$OUT"
cp "$CODEC_SRC"/*.rs "$OUT"/

# --- (1) atributos de crate raíz ---
sed -i '/^#!\[cfg_attr(not(test), no_std)\]/d' "$OUT/lib.rs"
sed -i '/^#!\[forbid(unsafe_code)\]/d'         "$OUT/lib.rs"

# --- (2) alloc no existe en el kernel ---
sed -i '/^extern crate alloc;/d'              "$OUT/lib.rs"
sed -i '/^use alloc::vec::Vec;/d'             "$OUT/huff.rs"

# --- (3) truncar tests de crate raíz ---
awk '{ if ($0 ~ /^#\[cfg\(test\)\]$/) exit; print }' "$OUT/lib.rs" > "$OUT/lib.rs.tmp" \
  && mv "$OUT/lib.rs.tmp" "$OUT/lib.rs"

echo "gen_codec regenerado: $(ls "$OUT")"