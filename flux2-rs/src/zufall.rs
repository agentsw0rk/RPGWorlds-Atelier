//! Zufall aus dem Betriebssystem (`/dev/urandom`) — für Job-IDs, Upload-IDs und
//! gewürfelte Seeds. Reicht dafür und spart eine Abhängigkeit.

use std::io::Read;

/// `n` Zufallsbytes. Fällt der Zugriff aus, ist das ein Fehler der Umgebung,
/// mit dem kein Lauf sinnvoll weitermacht — daher Panic statt Ersatzwert.
pub fn bytes(n: usize) -> Vec<u8> {
    let mut puffer = vec![0u8; n];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut puffer))
        .expect("/dev/urandom nicht lesbar");
    puffer
}

/// Zufällige Hex-Zeichenkette aus `n` Bytes (also `2 * n` Zeichen).
pub fn hex(n: usize) -> String {
    bytes(n).iter().map(|b| format!("{b:02x}")).collect()
}

/// Ein Seed im Bereich 0..1_000_000_000, wie `generate-macos.sh` ihn würfelt.
pub fn seed() -> i64 {
    let b = bytes(4);
    (u32::from_le_bytes([b[0], b[1], b[2], b[3]]) % 1_000_000_000) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_hat_die_doppelte_laenge_und_nur_hexziffern() {
        let h = hex(8);
        assert_eq!(h.len(), 16);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn zwei_ids_sind_verschieden() {
        assert_ne!(hex(8), hex(8));
    }

    #[test]
    fn seed_liegt_im_erlaubten_bereich() {
        for _ in 0..50 {
            assert!((0..1_000_000_000).contains(&seed()));
        }
    }
}
