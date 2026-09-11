//! Auswertung der Umgebungsvariablen-Parameter — der testbare Teil der CLI.

use anyhow::{bail, Result};
use std::path::PathBuf;

/// Zerlegt eine kommaseparierte Liste von Referenzbild-Pfaden.
///
/// Leere Einträge werden ignoriert. Ein nicht existierender Pfad ist ein
/// **Fehler**: `diffusion-rs` überspringt fehlende Referenzbilder stillschweigend
/// (`api.rs`: `if ref_path.exists()`), der Lauf liefe also ohne Referenz weiter.
pub fn ref_image_paths(raw: &str) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let path = PathBuf::from(part);
        if !path.exists() {
            bail!("Referenzbild {part} existiert nicht.");
        }
        paths.push(path);
    }
    Ok(paths)
}

/// Wertet eine Schaltervariable aus: `0`, `false`, `off` und `no` schalten ab.
///
/// Getrennt von der Umgebung, damit es ohne Seiteneffekte testbar bleibt —
/// Tests laufen parallel, und `std::env::set_var` wäre zwischen ihnen geteilt.
pub fn flag_wert(wert: Option<&str>, default: bool) -> bool {
    match wert {
        Some(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        ),
        None => default,
    }
}

/// Wie [`flag_wert`], liest den Wert aber aus der Umgebung.
pub fn env_flag(key: &str, default: bool) -> bool {
    flag_wert(std::env::var(key).ok().as_deref(), default)
}

/// Stellt sicher, dass eine Kantenlänge durch 16 teilbar ist.
///
/// FLUX.2 arbeitet im Latent-Raum mit Faktor 16; krumme Werte lehnt das Modell ab.
pub fn check_multiple_of_16(name: &str, value: i32) -> Result<()> {
    if value % 16 != 0 {
        bail!("{name}={value} ist nicht durch 16 teilbar — das Modell verlangt das.");
    }
    Ok(())
}

/// Liest eine Farbe im Format `rrggbb` oder `#rrggbb`.
///
/// Kurzschreibweisen sind bewusst nicht erlaubt: eine unlesbare Angabe muss
/// abbrechen, statt still auf einen Default zurückzufallen.
pub fn hex_farbe(raw: &str) -> Result<[u8; 3]> {
    let hex = raw.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("'{raw}' ist keine Farbe im Format rrggbb, z. B. ffffff.");
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).expect("Hexziffern geprüft");
    Ok([byte(0), byte(2), byte(4)])
}

/// Wie das freigestellte Bild abgelegt wird.
#[derive(Debug, PartialEq, Eq)]
pub enum Canvas {
    /// Originalabmessungen behalten — es wird nur der Hintergrund auf alpha = 0 gezogen.
    Original,
    /// Auf das Motiv zuschneiden und in eine Fläche dieser Größe einpassen.
    Fit(u32, u32),
}

/// Leitet aus den optionalen Zielabmessungen ab, was mit dem Bild passieren soll.
///
/// Ohne Angabe bleibt das Bild, wie es ist — das ist der Normalfall
/// "Hintergrund transparent machen". Mit Angabe entsteht die Token-Variante:
/// auf das Motiv zuschneiden, einpassen, Rest transparent lassen.
pub fn canvas(width: Option<u32>, height: Option<u32>) -> Result<Canvas> {
    match (width, height) {
        (None, None) => Ok(Canvas::Original),
        (Some(w), Some(h)) => Ok(Canvas::Fit(w, h)),
        _ => bail!("OUT_W und OUT_H nur gemeinsam angeben — eine Kante allein ist mehrdeutig."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn referenzliste_wird_an_kommas_geteilt() {
        // Cargo.toml existiert garantiert: Tests laufen im Crate-Wurzelverzeichnis.
        let paths = ref_image_paths("Cargo.toml, Cargo.lock").expect("beide Dateien existieren");
        assert_eq!(paths, vec![PathBuf::from("Cargo.toml"), PathBuf::from("Cargo.lock")]);
    }

    #[test]
    fn leere_eintraege_werden_ignoriert() {
        let paths = ref_image_paths(" , Cargo.toml ,, ").expect("ein gültiger Pfad reicht");
        assert_eq!(paths, vec![PathBuf::from("Cargo.toml")]);
    }

    #[test]
    fn fehlender_pfad_ist_ein_fehler() {
        // Stilles Ignorieren wäre schlimmer als ein Abbruch: das Bild entstünde ohne Referenz.
        let err = ref_image_paths("gibt-es-nicht.png").expect_err("Pfad existiert nicht");
        assert!(err.to_string().contains("gibt-es-nicht.png"), "Fehler nennt den Pfad: {err}");
    }

    #[test]
    fn kantenlaenge_muss_durch_16_teilbar_sein() {
        assert!(check_multiple_of_16("WIDTH", 288).is_ok());
        let err = check_multiple_of_16("HEIGHT", 300).expect_err("300 ist nicht durch 16 teilbar");
        assert!(err.to_string().contains("HEIGHT"), "Fehler nennt die Variable: {err}");
    }

    #[test]
    fn ohne_zielgroesse_bleibt_das_original_erhalten() {
        assert_eq!(canvas(None, None).unwrap(), Canvas::Original);
    }

    #[test]
    fn mit_zielgroesse_wird_eingepasst() {
        assert_eq!(canvas(Some(138), Some(244)).unwrap(), Canvas::Fit(138, 244));
    }

    #[test]
    fn halbe_zielgroesse_ist_ein_fehler() {
        // Nur eine Kante anzugeben ist mehrdeutig — lieber abbrechen als raten.
        assert!(canvas(Some(138), None).is_err());
        assert!(canvas(None, Some(244)).is_err());
    }

    #[test]
    fn hexfarbe_wird_mit_und_ohne_raute_gelesen() {
        assert_eq!(hex_farbe("ffffff").unwrap(), [255, 255, 255]);
        assert_eq!(hex_farbe("#808080").unwrap(), [128, 128, 128]);
    }

    #[test]
    fn unlesbare_hexfarbe_ist_ein_fehler() {
        // Lieber abbrechen als still auf Schwarz zurückfallen — genau das ist ja der Bug.
        assert!(hex_farbe("weiss").is_err());
        assert!(hex_farbe("fff").is_err());
    }

    #[test]
    fn schaltervariablen_werden_gelesen() {
        assert!(!flag_wert(Some("0"), true));
        assert!(!flag_wert(Some("false"), true));
        assert!(!flag_wert(Some(" OFF "), true));
        assert!(flag_wert(Some("1"), false));
        assert!(flag_wert(None, true), "ohne Angabe gilt der Default");
        assert!(!flag_wert(None, false));
    }
}
