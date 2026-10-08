// SPDX-License-Identifier: MIT
//! Traducción de valores: alfabeto de los literales ordenado por frecuencia.
//!
//! Cada byte literal se mapea a un símbolo 0..K-1 dentro del alfabeto de los
//! K valores distintos que aparecen en la página. El orden es determinista:
//! count desc, valor asc. `K==1` significa "toda la página es el mismo byte"
//! (típico de swap limpio): se codifica con 0 bits.

/// Construye el alfabeto (valores ordenados) y la tabla inversa valor->símbolo.
pub fn alphabet(counts: &[u32; 256]) -> (usize, [u8; 256], [u8; 256]) {
    let mut order = [0u8; 256];
    for (i, o) in order.iter_mut().enumerate() {
        *o = i as u8;
    }
    // Orden: count desc, valor asc.
    order.sort_by(|&a, &b| {
        counts[b as usize]
            .cmp(&counts[a as usize])
            .then_with(|| a.cmp(&b))
    });
    let k = order
        .iter()
        .position(|&v| counts[v as usize] == 0)
        .unwrap_or(256);

    let mut v2s = [0u8; 256];
    for s in 0..k {
        v2s[order[s] as usize] = s as u8;
    }
    (k, order, v2s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alphabet_order() {
        // bytes: a(5 veces), b(2), z(7) -> z,a,b
        let mut counts = [0u32; 256];
        counts[b'a' as usize] = 5;
        counts[b'b' as usize] = 2;
        counts[b'z' as usize] = 7;
        let (k, alpha, v2s) = alphabet(&counts);
        assert_eq!(k, 3);
        assert_eq!(&alpha[..3], &[b'z', b'a', b'b']);
        assert_eq!(v2s[b'z' as usize], 0);
        assert_eq!(v2s[b'a' as usize], 1);
        assert_eq!(v2s[b'b' as usize], 2);
    }

    #[test]
    fn alphabet_all_zero() {
        let mut counts = [0u32; 256];
        counts[0] = 4096;
        let (k, alpha, v2s) = alphabet(&counts);
        assert_eq!(k, 1);
        assert_eq!(alpha[0], 0);
        assert_eq!(v2s[0], 0);
    }
}
