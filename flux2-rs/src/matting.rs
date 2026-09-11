//! Freistellen: aus einem RGB-Bild plus Saliency-Maske ein RGBA-Bild mit
//! transparentem Hintergrund machen.

use image::{RgbImage, RgbaImage};

/// Wendet eine Graustufen-Maske als Alphakanal auf ein RGB-Bild an.
///
/// Maske und Bild müssen dieselben Abmessungen haben. Maskenwert 0 wird zu
/// `alpha = 0` (voll transparent), 255 zu `alpha = 255` (voll deckend).
pub fn apply_mask_as_alpha(rgb: &RgbImage, mask: &image::GrayImage) -> RgbaImage {
    debug_assert_eq!(rgb.dimensions(), mask.dimensions());
    let (w, h) = rgb.dimensions();
    let mut out = RgbaImage::new(w, h);
    for (x, y, px) in rgb.enumerate_pixels() {
        let a = mask.get_pixel(x, y)[0];
        out.put_pixel(x, y, image::Rgba([px[0], px[1], px[2], a]));
    }
    out
}

/// Schneidet auf die Bounding-Box aller Pixel mit `alpha > threshold` zu.
///
/// Gibt `None` zurück, wenn kein Pixel den Schwellwert überschreitet (das Bild
/// also komplett transparent ist).
pub fn crop_to_content(rgba: &RgbaImage, threshold: u8) -> Option<RgbaImage> {
    let (mut min_x, mut min_y) = (u32::MAX, u32::MAX);
    let (mut max_x, mut max_y) = (0u32, 0u32);
    for (x, y, px) in rgba.enumerate_pixels() {
        if px[3] > threshold {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    if min_x == u32::MAX {
        return None;
    }
    let (w, h) = (max_x - min_x + 1, max_y - min_y + 1);
    Some(image::imageops::crop_imm(rgba, min_x, min_y, w, h).to_image())
}

/// Skaliert das Bild seitenverhältniswahrend so, dass es vollständig in
/// `target_w` x `target_h` passt, und zentriert es auf einer **komplett
/// transparenten** Fläche genau dieser Größe.
///
/// Das Ergebnis hat immer exakt die Zielabmessungen; überschüssiger Rand
/// bleibt bei `alpha = 0`.
pub fn fit_into(rgba: &RgbaImage, target_w: u32, target_h: u32) -> RgbaImage {
    let (w, h) = rgba.dimensions();
    // Kleinerer der beiden Faktoren gewinnt, damit nichts abgeschnitten wird.
    let scale = (target_w as f64 / w as f64).min(target_h as f64 / h as f64);
    let new_w = ((w as f64 * scale).round() as u32).clamp(1, target_w);
    let new_h = ((h as f64 * scale).round() as u32).clamp(1, target_h);

    let scaled = image::imageops::resize(rgba, new_w, new_h, image::imageops::FilterType::Lanczos3);

    // Leinwand ist [0,0,0,0] — der Rand bleibt damit garantiert voll transparent.
    let mut canvas = RgbaImage::new(target_w, target_h);
    let off_x = ((target_w - new_w) / 2) as i64;
    let off_y = ((target_h - new_h) / 2) as i64;
    image::imageops::replace(&mut canvas, &scaled, off_x, off_y);
    canvas
}

/// Legt ein RGBA-Bild auf eine einfarbige Fläche und gibt RGB zurück.
///
/// Nötig für Referenzbilder: `diffusion-rs` wirft den Alphakanal einfach weg
/// (`api.rs`: `img.to_rgb8()`), wodurch transparente Flächen als **Schwarz** im
/// Modell landen. Das Referenzbild sagt dann "schwarzer Hintergrund", obwohl es
/// gar keinen hat.
pub fn auf_hintergrund(rgba: &RgbaImage, hintergrund: image::Rgb<u8>) -> RgbImage {
    let (w, h) = rgba.dimensions();
    let mut out = RgbImage::new(w, h);
    for (x, y, px) in rgba.enumerate_pixels() {
        let a = px[3] as f32 / 255.0;
        let mix = |vorne: u8, hinten: u8| (vorne as f32 * a + hinten as f32 * (1.0 - a)).round() as u8;
        out.put_pixel(
            x,
            y,
            image::Rgb([
                mix(px[0], hintergrund[0]),
                mix(px[1], hintergrund[1]),
                mix(px[2], hintergrund[2]),
            ]),
        );
    }
    out
}

/// Anteil der voll transparenten Pixel (`alpha == 0`) am gesamten Bild.
///
/// Kontrollwert nach dem Freistellen: nahe 0 heißt, dass die Maske kaum etwas
/// als Hintergrund erkannt hat — dann stimmt mit dem Motiv oder dem Prompt
/// ("plain solid background") etwas nicht.
pub fn transparenz_anteil(rgba: &RgbaImage) -> f32 {
    let gesamt = rgba.pixels().count();
    if gesamt == 0 {
        return 0.0;
    }
    let transparent = rgba.pixels().filter(|px| px[3] == 0).count();
    transparent as f32 / gesamt as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GrayImage, Luma, Rgb, Rgba};

    #[test]
    fn maske_wird_zum_alphakanal() {
        // 2x1-Bild: linkes Pixel soll transparent werden, rechtes deckend.
        let mut rgb = RgbImage::new(2, 1);
        rgb.put_pixel(0, 0, Rgb([10, 20, 30]));
        rgb.put_pixel(1, 0, Rgb([40, 50, 60]));

        let mut mask = GrayImage::new(2, 1);
        mask.put_pixel(0, 0, Luma([0]));
        mask.put_pixel(1, 0, Luma([255]));

        let out = apply_mask_as_alpha(&rgb, &mask);

        assert_eq!(out.dimensions(), (2, 1), "Abmessungen bleiben erhalten");
        assert_eq!(out.get_pixel(0, 0)[3], 0, "Maske 0 muss alpha 0 ergeben");
        assert_eq!(out.get_pixel(1, 0)[3], 255, "Maske 255 muss alpha 255 ergeben");
        // Farbkanäle bleiben unangetastet.
        assert_eq!(&out.get_pixel(1, 0).0[..3], &[40, 50, 60]);
    }

    /// 4x4 transparent, darin ein 2x2-Block bei (1,1) — erwartet wird ein 2x2-Ausschnitt.
    fn bild_mit_block() -> RgbaImage {
        let mut img = RgbaImage::new(4, 4);
        for y in 1..3 {
            for x in 1..3 {
                img.put_pixel(x, y, Rgba([9, 9, 9, 255]));
            }
        }
        img
    }

    #[test]
    fn zuschnitt_findet_die_bounding_box() {
        let out = crop_to_content(&bild_mit_block(), 0).expect("Inhalt vorhanden");
        assert_eq!(out.dimensions(), (2, 2));
        assert_eq!(out.get_pixel(0, 0), &Rgba([9, 9, 9, 255]));
    }

    #[test]
    fn zuschnitt_ignoriert_pixel_unter_schwellwert() {
        let mut img = bild_mit_block();
        // Schwaches Rauschen in der Ecke darf die Box nicht aufziehen.
        img.put_pixel(0, 0, Rgba([9, 9, 9, 5]));
        let out = crop_to_content(&img, 10).expect("Inhalt vorhanden");
        assert_eq!(out.dimensions(), (2, 2), "alpha 5 liegt unter Schwelle 10");
    }

    #[test]
    fn zuschnitt_gibt_none_bei_leerem_bild() {
        assert!(crop_to_content(&RgbaImage::new(4, 4), 0).is_none());
    }

    fn deckend(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([200, 100, 50, 255]))
    }

    #[test]
    fn einpassen_liefert_exakt_die_zielgroesse() {
        // Genau der Fall aus der Aufgabe: 288x512 -> 138x244.
        let out = fit_into(&deckend(288, 512), 138, 244);
        assert_eq!(out.dimensions(), (138, 244));
    }

    #[test]
    fn einpassen_fuellt_rand_mit_alpha_null() {
        // Breites Bild in quadratisches Ziel: oben und unten muss Rand entstehen.
        let out = fit_into(&deckend(4, 1), 8, 8);
        assert_eq!(out.dimensions(), (8, 8));
        assert_eq!(out.get_pixel(0, 0)[3], 0, "Ecke oben links muss transparent sein");
        assert_eq!(out.get_pixel(7, 7)[3], 0, "Ecke unten rechts muss transparent sein");
        assert_eq!(out.get_pixel(4, 4)[3], 255, "Mitte muss deckend sein");
    }

    #[test]
    fn einpassen_erhaelt_das_seitenverhaeltnis() {
        // 4:1 in ein 8x8-Ziel -> Inhalt wird 8x2, also 3 Zeilen Rand oben.
        let out = fit_into(&deckend(4, 1), 8, 8);
        let deckende_zeilen = (0..8).filter(|&y| out.get_pixel(4, y)[3] > 0).count();
        assert_eq!(deckende_zeilen, 2, "4:1 in 8x8 ergibt 2 deckende Zeilen");
    }

    #[test]
    fn transparenzanteil_zaehlt_nur_volle_transparenz() {
        // 2x2: ein Pixel deckend, einer fast transparent (alpha 7, aber nicht 0),
        // zwei bleiben auf [0,0,0,0]. Erwartet: genau die Hälfte.
        let mut img = RgbaImage::new(2, 2);
        img.put_pixel(0, 0, Rgba([9, 9, 9, 255]));
        img.put_pixel(1, 0, Rgba([9, 9, 9, 7]));

        assert_eq!(transparenz_anteil(&img), 0.5);
    }

    #[test]
    fn leeres_bild_liefert_anteil_null_statt_nan() {
        // Division durch 0 wäre NaN und würde in der Ausgabe als "NaN %" landen.
        assert_eq!(transparenz_anteil(&RgbaImage::new(0, 0)), 0.0);
    }

    #[test]
    fn alpha_wird_auf_den_hintergrund_gerechnet() {
        // 3x1: deckend, halbtransparent, voll transparent — auf Weiß gelegt.
        let mut img = RgbaImage::new(3, 1);
        img.put_pixel(0, 0, Rgba([200, 0, 0, 255]));
        img.put_pixel(1, 0, Rgba([200, 0, 0, 128]));
        img.put_pixel(2, 0, Rgba([200, 0, 0, 0]));

        let out = auf_hintergrund(&img, Rgb([255, 255, 255]));

        assert_eq!(out.get_pixel(0, 0), &Rgb([200, 0, 0]), "deckend bleibt unverändert");
        assert_eq!(out.get_pixel(2, 0), &Rgb([255, 255, 255]), "transparent wird zum Hintergrund");
        // 200*0.502 + 255*0.498 = 227.4
        assert_eq!(out.get_pixel(1, 0), &Rgb([227, 127, 127]), "halbtransparent mischt beide");
    }
}
