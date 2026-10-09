//! Freistellen eines generierten Bildes: der Teil von `matte`, der keine
//! Kommandozeile braucht. Der Server ruft ihn direkt auf.

use crate::auftrag::Freistellen;
use crate::keying::{despill, hintergrund_maske, hintergrundfarbe, Toleranzen};
use crate::matting::{apply_mask_as_alpha, crop_to_content, fit_into, transparenz_anteil};
use crate::saliency;
use anyhow::{Context, Result};
use image::{RgbImage, RgbaImage};
use std::path::Path;

/// Ergebnis des Freistellens samt Kontrollwert.
pub struct Freigestellt {
    pub bild: RgbaImage,
    /// Anteil der Fläche bei alpha = 0, 0.0..=1.0. Bei ~0 hat das Freistellen
    /// nichts gefunden, auch wenn die Datei formal ein gültiges RGBA-PNG ist.
    pub transparent: f32,
}

/// Stellt `rgb` nach `einstellung` frei.
///
/// `u2netp` ist nur für den Saliency-Weg nötig (`keying == false`).
pub fn freistellen(rgb: &RgbImage, einstellung: &Freistellen, u2netp: Option<&Path>) -> Result<Freigestellt> {
    let (maske, key) = if einstellung.keying {
        // Hintergrundfarbe vom Rand: der Hintergrund ist im Stil festgelegt
        // (mittelgrau), Keying behält dadurch auch Sockel und Beiwerk.
        let key = hintergrundfarbe(rgb);
        (hintergrund_maske(rgb, key, Toleranzen::default()), Some(key))
    } else {
        let modell = u2netp.context("Für das Freistellen über u2netp fehlt die Modelldatei")?;
        (saliency::maske(modell, rgb, einstellung.cutoff)?, None)
    };

    let rgba = apply_mask_as_alpha(rgb, &maske);
    // Despill geht nur beim Keying: bei u2netp kennt niemand die Hintergrundfarbe.
    let rgba = match key {
        Some(key) => despill(&rgba, key),
        None => rgba,
    };
    let bild = match einstellung.einpassen {
        None => rgba,
        Some((w, h)) => {
            let zugeschnitten = crop_to_content(&rgba, einstellung.cutoff)
                .context("Maske ist komplett leer — kein Motiv erkannt")?;
            fit_into(&zugeschnitten, w, h)
        }
    };
    let transparent = transparenz_anteil(&bild);
    Ok(Freigestellt { bild, transparent })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    /// Mittelgrauer Grund mit einem roten Quadrat in der Mitte.
    fn motiv() -> RgbImage {
        RgbImage::from_fn(64, 64, |x, y| {
            if (20..44).contains(&x) && (20..44).contains(&y) {
                Rgb([200, 30, 30])
            } else {
                Rgb([128, 128, 128])
            }
        })
    }

    fn keying(einpassen: Option<(u32, u32)>) -> Freistellen {
        Freistellen { keying: true, cutoff: 12, einpassen }
    }

    #[test]
    fn keying_macht_den_grund_transparent_und_behaelt_das_motiv() {
        let ergebnis = freistellen(&motiv(), &keying(None), None).unwrap();
        assert_eq!(ergebnis.bild.dimensions(), (64, 64));
        assert_eq!(ergebnis.bild.get_pixel(2, 2)[3], 0, "Ecke ist Hintergrund");
        assert_eq!(ergebnis.bild.get_pixel(32, 32)[3], 255, "Mitte ist Motiv");
        assert!(ergebnis.transparent > 0.5, "mehr als die halbe Fläche ist Grund");
    }

    #[test]
    fn einpassen_schneidet_zu_und_liefert_exakt_die_zielgroesse() {
        let ergebnis = freistellen(&motiv(), &keying(Some((138, 244))), None).unwrap();
        assert_eq!(ergebnis.bild.dimensions(), (138, 244));
    }

    #[test]
    fn saliency_ohne_modelldatei_ist_ein_klarer_fehler() {
        let einstellung = Freistellen { keying: false, cutoff: 12, einpassen: None };
        let fehler = freistellen(&motiv(), &einstellung, None).err().unwrap().to_string();
        assert!(fehler.contains("u2netp"), "{fehler}");
    }

    #[test]
    fn einfarbiges_bild_ohne_motiv_scheitert_beim_einpassen() {
        let leer = RgbImage::from_pixel(32, 32, Rgb([128, 128, 128]));
        assert!(freistellen(&leer, &keying(Some((138, 244))), None).is_err());
    }
}
