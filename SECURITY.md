# Política de seguridad

estiba es un proyecto **experimental**: no hay versiones estables ni soporte de
producción. Aun así, agradecemos los reportes responsables.

## Reportar una vulnerabilidad

Usa el botón **"Report a vulnerability"** de la pestaña **Security** de este
repositorio ([GitHub Security Advisories](https://github.com/tiqueagatho/Estiba/security/advisories/new)).
El informe es **privado** hasta que exista un arreglo; **no abras un issue
público** para un fallo de seguridad.

Incluye, si puedes:

- Versión de kernel (`uname -r`) y si tiene `CONFIG_RUST=y`.
- Cómo compilaste/cargaste (p. ej. `infra/sandbox/run.sh`).
- Reproducción mínima y trazas (`dmesg`, log de la VM).
- Impacto esperado (corrupción de datos, DoS, escalada…).

## Alcance

- **En alcance**: `module/` (driver del kernel), `codec/` (corrupción/roundtrip),
  `infra/sandbox/`.
- **Fuera de alcance**: que requiera un kernel a medida, y cualquier uso en
  producción (no soportado).

## Divulgación

- Confirmación de recepción: ~7 días.
- Arreglo/mitigación antes de publicar el advisory.
- Crédito al reportero si lo desea.
