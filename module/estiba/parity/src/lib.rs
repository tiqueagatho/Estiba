//! Harness de paridad bit-exacta: `codec/` (crate) vs `gen_codec/` (port
//! no_std que se compila dentro del módulo del kernel).
//!
//! Garantiza que la copia genera por `module/estiba/gen-codec.sh` — la asignación
//! canónica de códigos Huffman sin `alloc`, que vive ya en el propio codec —
//! produce exactamente el mismo bitstream que el crate, para un corpus
//! determinista (ceros, patrones, alfabeto completo k=256, valores, datos
//! estructurados, incompresible). Ejecutar: `make parity`.

#[path = "../../gen_codec/lib.rs"]
mod codec_kernel;

#[cfg(test)]
mod parity {
    use super::codec_kernel as k;
    use estiba_codec as c;

    fn hd_eq(a: &[u8], b: &[u8], label: &str) {
        assert_eq!(
            a.len(),
            b.len(),
            "{label}: longitud distinta ({} vs {})",
            a.len(),
            b.len()
        );
        assert_eq!(a, b, "{label}: bitstream distinto");
    }

    fn check(src: &[u8]) {
        let label = format!("len={}", src.len());
        let bound = c::compress_bound(src.len());

        // Compresión: longitudes y bitstream idénticos.
        let mut slot1 = vec![0u8; bound];
        let mut slot2 = vec![0u8; bound];
        let n1 = c::compress_into(
            src,
            &mut slot1,
            &mut vec![0u8; bound],
            &mut vec![0u32; c::HASH_TAB_LEN],
        )
        .unwrap_or_else(|e| panic!("codec: {e:?} (len {})", src.len()));
        let n2 = k::compress_into(
            src,
            &mut slot2,
            &mut vec![0u8; bound],
            &mut vec![0u32; k::HASH_TAB_LEN],
        )
        .unwrap_or_else(|e| panic!("kernel-port: {e:?} (len {})", src.len()));
        assert_eq!(n1, n2, "{label}: tamaño comprimido distinto");
        hd_eq(&slot1[..n1], &slot2[..n2], &label);

        // Roundtrip con cada motor.
        let mut d1 = vec![0u8; src.len()];
        let mut d2 = vec![0u8; src.len()];
        let m1 = c::decompress_into(&slot1[..n1], &mut d1, &mut vec![0u8; bound]);
        let m2 = k::decompress_into(&slot2[..n2], &mut d2, &mut vec![0u8; bound]);
        assert_eq!(m1, Ok(src.len()), "{label}: roundtrip codec");
        assert_eq!(m2, Ok(src.len()), "{label}: roundtrip kernel-port");
        hd_eq(&d1[..src.len()], src, &format!("{label}: decode codec"));
        hd_eq(&d2[..src.len()], src, &format!("{label}: decode kernel-port"));

        // Cross-decoding: el slot del kernel lo lee el codec y al revés
        // (compatibilidad de payload con la versión original).
        let mut x1 = vec![0u8; src.len()];
        let mut x2 = vec![0u8; src.len()];
        let mx1 = c::decompress_into(&slot2[..n2], &mut x1, &mut vec![0u8; bound]);
        let mx2 = k::decompress_into(&slot1[..n1], &mut x2, &mut vec![0u8; bound]);
        assert_eq!(mx1, Ok(src.len()), "{label}: codec lee slot del port");
        assert_eq!(mx2, Ok(src.len()), "{label}: port lee slot del codec");
        hd_eq(&x1[..src.len()], src, &format!("{label}: cross-decode A"));
        hd_eq(&x2[..src.len()], src, &format!("{label}: cross-decode B"));
    }

    fn lcg(seed: &mut u64) -> u64 {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *seed
    }

    fn corpus() -> Vec<Vec<u8>> {
        let mut v = Vec::new();
        let sizes = [0usize, 1, 2, 7, 63, 255, 256, 511, 1024, 2048, 4096, 8000];
        for &n in &sizes {
            v.push(vec![0u8; n]);
            v.push(vec![0xFF; n]);
            v.push((0..n).map(|i| i as u8).collect());
            v.push((0..n).map(|i| (i % 7) as u8).collect());
            // pseudoaleatorio determinista (mayormente incompresible)
            let mut seed = 0x1234_5678_9ABC_DEF0u64 ^ (n as u64);
            v.push((0..n).map(|_| lcg(&mut seed) as u8).collect());
            // valores de baja entropía por byte (buen LZ)
            v.push(
                (0..n)
                    .map(|i| {
                        if i & 3 == 0 {
                            10u8
                        } else {
                            200u8
                        }
                        .wrapping_add((i % 5) as u8)
                    })
                    .collect(),
            );
        }
        // texto/código con líneas repetidas
        let mut code = Vec::new();
        for r in 0..600 {
            code.extend_from_slice(
                format!("pub fn caso{r}() -> u8 {{ 0x{r:x} }}\n").as_bytes(),
            );
        }
        v.push(code);
        v
    }

    #[test]
    fn parity_bit_exacta_corpus() {
        for src in corpus() {
            check(&src);
        }
    }

    #[test]
    fn port_determinista_y_raw_flag_compatible() {
        // El port resetea su ht por llamada: dos pasadas deben ser idénticas.
        let src = b"hola hola hola, estiba codec estiba codec estiba";
        let bound = c::compress_bound(src.len());
        let mut a = vec![0u8; bound];
        let mut b = vec![0u8; bound];
        let mut h = vec![0u32; k::HASH_TAB_LEN];
        k::compress_into(src, &mut a, &mut vec![0u8; bound], &mut h).unwrap();
        h = vec![0u32; k::HASH_TAB_LEN];
        k::compress_into(src, &mut b, &mut vec![0u8; bound], &mut h).unwrap();
        assert_eq!(a, b, "port no determinista");

        // Los slots compactos/residuales siempre llevan el flag del codec.
        let mut slot = vec![0u8; bound];
        let n = c::compress_into(src, &mut slot, &mut vec![0u8; bound], &mut vec![0u32; c::HASH_TAB_LEN]).unwrap();
        assert!(n >= 10, "slot debe tener cabecera (10 B)");
        let mut h2 = vec![0u32; k::HASH_TAB_LEN];
        let mut slot2 = vec![0u8; bound];
        let n2 = k::compress_into(src, &mut slot2, &mut vec![0u8; bound], &mut h2).unwrap();
        assert_eq!(slot2[..n2], slot[..n]);
    }
}