## Qué cambia

<!-- Resumen breve del cambio y por qué. Si cierra un issue, enlázalo. -->

## Checklist

- [ ] `cargo fmt --all -- --check` limpio
- [ ] `cargo test -p estiba-codec --features alloc` verde (códec)
- [ ] `cargo test -p estiba-ctl` verde
- [ ] `make -C module/estiba parity` verde (bit-exacto códec ⇔ kernel)
- [ ] Si toco el driver: probado en VM (`infra/sandbox/run.sh`); **nada instalado en el host**
- [ ] Licencias correctas (módulo **GPL-2.0** / códec y CLI **MIT**)
- [ ] Sin secretos ni datos privados

## Notas

<!-- Método (SDD/TDD/ODD), decisiones de diseño, resultados de benchmark. -->
