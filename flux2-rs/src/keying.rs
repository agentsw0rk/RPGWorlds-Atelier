//! Freistellen über die Hintergrundfarbe statt über Saliency.
//!
//! Für Bilder mit bewusst einfarbigem Hintergrund ist das u2netp überlegen:
//! u2netp sucht *ein* hervorstechendes Objekt und zählt Sockel, Podeste und
//! Schatten zum Hintergrund. Hier zählt stattdessen, was der Hintergrundfarbe
//! ähnelt **und vom Bildrand aus zusammenhängt** — damit bleiben graue Flächen
//! mitten in der Figur erhalten.

use image::{GrayImage, RgbImage, RgbaImage};

/// Farbabstände, ab denen ein Pixel als Hintergrund bzw. als Motiv gilt.
#[derive(Debug, Clone, Copy)]
pub struct Toleranzen {
    /// Bis hierhin gilt ein Pixel als reiner Hintergrund (alpha = 0).
    pub innen: f32,
    /// Ab hier gilt ein Pixel als Motiv (alpha = 255); dazwischen wird überblendet.
    pub aussen: f32,
    /// Eingeschlossene Flächen bis zu diesem Abstand ebenfalls entfernen.
    ///
    /// Nötig für Lücken zwischen den Beinen oder zwischen Arm und Rumpf: die
    /// sind vom Bildrand aus nicht erreichbar. Bewusst strenger als `innen` —
    /// eine graue Rüstung soll kein Loch bekommen. `<= 0` schaltet es ab.
    pub loch: f32,
    /// Mindestfläche einer eingeschlossenen Lücke in Pixeln.
    ///
    /// Glanzlichter auf Stahl treffen die Hintergrundfarbe zufällig genau, sind
    /// aber winzig. Ohne diese Schwelle löchert die Lochsuche jede Rüstung.
    pub loch_min: usize,
}

impl Default for Toleranzen {
    fn default() -> Self {
        // Gemessen an generierten Tokens auf mittelgrauem Grund. `loch` ist
        // bewusst eng: eine echte Lücke zeigt den Hintergrund selbst und trifft
        // dessen Farbe fast exakt, während auf den Sockel gemalte Schatten nur
        // in der Nähe liegen. Bei 30 fraß die Lochsuche diese Schatten weg.
        Self { innen: 70.0, aussen: 95.0, loch: 12.0, loch_min: 500 }
    }
}

/// Schätzt die Hintergrundfarbe aus dem Bildrand.
///
/// Der Median je Kanal, nicht der Mittelwert: ragt die Figur bis an den Rand,
/// zieht ein Mittelwert die Farbe in Richtung Figur, der Median nicht.
pub fn hintergrundfarbe(rgb: &RgbImage) -> image::Rgb<u8> {
    let (w, h) = rgb.dimensions();
    let mut kanaele: [Vec<u8>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut sammle = |px: &image::Rgb<u8>| {
        for k in 0..3 {
            kanaele[k].push(px[k]);
        }
    };
    for x in 0..w {
        sammle(rgb.get_pixel(x, 0));
        sammle(rgb.get_pixel(x, h - 1));
    }
    for y in 0..h {
        sammle(rgb.get_pixel(0, y));
        sammle(rgb.get_pixel(w - 1, y));
    }
    let mut farbe = [0u8; 3];
    for k in 0..3 {
        kanaele[k].sort_unstable();
        farbe[k] = kanaele[k][kanaele[k].len() / 2];
    }
    image::Rgb(farbe)
}

/// Baut die Alphamaske: 0 für Hintergrund, 255 für alles andere.
///
/// `innen` und `aussen` sind Farbabstände zur Hintergrundfarbe (euklidisch über
/// RGB, 0–441). Bis `innen` gilt ein Pixel als reiner Hintergrund, ab `aussen`
/// als Motiv, dazwischen wird weich übergeblendet — sonst bekämen die Kanten
/// Treppen, weil das Modell den Rand weich malt.
///
/// Entscheidend ist die Flutfüllung vom Bildrand: nur was **vom Rand aus
/// erreichbar** ist, wird entfernt. Eine graue Rüstung mitten in der Figur hat
/// denselben Farbabstand wie der Hintergrund, ist aber nicht erreichbar.
pub fn hintergrund_maske(rgb: &RgbImage, key: image::Rgb<u8>, tol: Toleranzen) -> GrayImage {
    let Toleranzen { innen, aussen, loch, loch_min } = tol;
    let (w, h) = rgb.dimensions();
    let mut maske = GrayImage::from_pixel(w, h, image::Luma([255]));
    let mut besucht = vec![false; (w as usize) * (h as usize)];
    let mut warteschlange = std::collections::VecDeque::new();

    let index = |x: u32, y: u32| (y as usize) * (w as usize) + (x as usize);
    let abstand = |x: u32, y: u32| {
        let px = rgb.get_pixel(x, y);
        let d = |k: usize| (px[k] as f32 - key[k] as f32).powi(2);
        (d(0) + d(1) + d(2)).sqrt()
    };

    let einreihen = |x: u32,
                     y: u32,
                     besucht: &mut Vec<bool>,
                     q: &mut std::collections::VecDeque<(u32, u32)>| {
        if !besucht[index(x, y)] && abstand(x, y) < aussen {
            besucht[index(x, y)] = true;
            q.push_back((x, y));
        }
    };

    // Erste Saat: der komplette Bildrand.
    for x in 0..w {
        einreihen(x, 0, &mut besucht, &mut warteschlange);
        einreihen(x, h - 1, &mut besucht, &mut warteschlange);
    }
    for y in 0..h {
        einreihen(0, y, &mut besucht, &mut warteschlange);
        einreihen(w - 1, y, &mut besucht, &mut warteschlange);
    }

    // Der Scan nach eingeschlossenen Flächen läuft nur einmal übers Bild:
    // der Cursor merkt sich, wie weit er gekommen ist.
    let mut cursor: usize = 0;
    loop {
        while let Some((x, y)) = warteschlange.pop_front() {
            let d = abstand(x, y);
            let alpha = if d <= innen {
                0.0
            } else {
                ((d - innen) / (aussen - innen)).clamp(0.0, 1.0) * 255.0
            };
            maske.put_pixel(x, y, image::Luma([alpha.round() as u8]));

            if x > 0 {
                einreihen(x - 1, y, &mut besucht, &mut warteschlange);
            }
            if y > 0 {
                einreihen(x, y - 1, &mut besucht, &mut warteschlange);
            }
            if x + 1 < w {
                einreihen(x + 1, y, &mut besucht, &mut warteschlange);
            }
            if y + 1 < h {
                einreihen(x, y + 1, &mut besucht, &mut warteschlange);
            }
        }

        if loch <= 0.0 {
            break;
        }
        // Nächste eingeschlossene Fläche suchen: Lücke zwischen den Beinen,
        // Zwickel zwischen Arm und Rumpf. Zwei Bedingungen, weil eine nicht
        // reicht: nahezu exakte Hintergrundfarbe (sonst bekämen dunkle Flächen
        // Löcher) *und* eine Mindestgröße (sonst die Glanzlichter auf Stahl,
        // die dieselbe Farbe haben).
        let mut gefunden = false;
        while cursor < (w as usize) * (h as usize) {
            let x = (cursor % w as usize) as u32;
            let y = (cursor / w as usize) as u32;
            cursor += 1;
            if besucht[index(x, y)] || abstand(x, y) > loch {
                continue;
            }

            // Zusammenhängende Fläche einsammeln, strikt in Hintergrundfarbe.
            let mut flaeche = Vec::new();
            let mut lokal = std::collections::VecDeque::new();
            besucht[index(x, y)] = true;
            lokal.push_back((x, y));
            while let Some((lx, ly)) = lokal.pop_front() {
                flaeche.push((lx, ly));
                let nachbar = |nx: u32, ny: u32, lokal: &mut std::collections::VecDeque<(u32, u32)>, besucht: &mut Vec<bool>| {
                    if !besucht[index(nx, ny)] && abstand(nx, ny) <= loch {
                        besucht[index(nx, ny)] = true;
                        lokal.push_back((nx, ny));
                    }
                };
                if lx > 0 {
                    nachbar(lx - 1, ly, &mut lokal, &mut besucht);
                }
                if ly > 0 {
                    nachbar(lx, ly - 1, &mut lokal, &mut besucht);
                }
                if lx + 1 < w {
                    nachbar(lx + 1, ly, &mut lokal, &mut besucht);
                }
                if ly + 1 < h {
                    nachbar(lx, ly + 1, &mut lokal, &mut besucht);
                }
            }

            if flaeche.len() < loch_min {
                continue; // Glanzlicht, kein Loch — bleibt deckend.
            }
            for &(fx, fy) in &flaeche {
                maske.put_pixel(fx, fy, image::Luma([0]));
            }
            // Einen Schritt nach außen weich auslaufen lassen, sonst hätte die
            // Lücke eine Treppenkante.
            for &(fx, fy) in &flaeche {
                for (nx, ny) in [
                    (fx.wrapping_sub(1), fy),
                    (fx, fy.wrapping_sub(1)),
                    (fx + 1, fy),
                    (fx, fy + 1),
                ] {
                    if nx >= w || ny >= h || besucht[index(nx, ny)] {
                        continue;
                    }
                    let d = abstand(nx, ny);
                    if d < aussen {
                        besucht[index(nx, ny)] = true;
                        let alpha = ((d - innen) / (aussen - innen)).clamp(0.0, 1.0) * 255.0;
                        maske.put_pixel(nx, ny, image::Luma([alpha.round() as u8]));
                    }
                }
            }
            gefunden = true;
            break;
        }
        if !gefunden {
            break;
        }
    }
    maske
}

/// Rechnet die Hintergrundfarbe aus halbtransparenten Randpixeln heraus.
///
/// Das Modell malt weiche Kanten, deshalb ist jedes Randpixel eine Mischung
/// `c = a·Motiv + (1-a)·Hintergrund`. Bleibt sie stehen, zieht sich ein Saum in
/// Hintergrundfarbe um die Figur — bei Grau unauffällig, bei einem gesättigten
/// Keying-Hintergrund ein deutlich sichtbarer Rand. Umgestellt nach dem Motiv:
/// `Motiv = (c - (1-a)·Hintergrund) / a`.
pub fn despill(rgba: &RgbaImage, key: image::Rgb<u8>) -> RgbaImage {
    let mut out = rgba.clone();
    for px in out.pixels_mut() {
        let a = px[3] as f32 / 255.0;
        // Bei sehr kleinem Alpha verstärkt die Division nur noch Rauschen, und
        // sichtbar ist das Pixel ohnehin nicht.
        if a <= 0.03 || a >= 1.0 {
            continue;
        }
        for k in 0..3 {
            let entmischt = (px[k] as f32 - (1.0 - a) * key[k] as f32) / a;
            px[k] = entmischt.round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, Rgba};

    /// 6x6: Rand durchgehend grau, in der Mitte ein 2x2-Block in Rot.
    fn bild_mit_motiv() -> RgbImage {
        let mut img = RgbImage::from_pixel(6, 6, Rgb([128, 128, 128]));
        for y in 2..4 {
            for x in 2..4 {
                img.put_pixel(x, y, Rgb([200, 30, 30]));
            }
        }
        img
    }

    #[test]
    fn hintergrundfarbe_kommt_vom_bildrand() {
        assert_eq!(hintergrundfarbe(&bild_mit_motiv()), Rgb([128, 128, 128]));
    }

    #[test]
    fn hintergrund_wird_transparent_motiv_bleibt_deckend() {
        let maske = hintergrund_maske(&bild_mit_motiv(), Rgb([128, 128, 128]), Toleranzen { innen: 30.0, aussen: 90.0, loch: 0.0, loch_min: 0 });

        assert_eq!(maske.get_pixel(0, 0)[0], 0, "Ecke ist Hintergrund");
        assert_eq!(maske.get_pixel(5, 5)[0], 0, "gegenüberliegende Ecke auch");
        assert_eq!(maske.get_pixel(2, 2)[0], 255, "das Motiv bleibt vollständig deckend");
        assert_eq!(maske.get_pixel(3, 3)[0], 255);
    }

    #[test]
    fn eingeschlossene_hintergrundfarbe_bleibt_erhalten() {
        // Graue Rüstung mitten in der Figur hat denselben Farbwert wie der
        // Hintergrund. Ohne Flutfüllung vom Rand bekäme die Figur hier ein Loch.
        let mut img = RgbImage::from_pixel(7, 7, Rgb([128, 128, 128]));
        for y in 1..6 {
            for x in 1..6 {
                img.put_pixel(x, y, Rgb([200, 30, 30]));
            }
        }
        img.put_pixel(3, 3, Rgb([128, 128, 128]));

        let maske = hintergrund_maske(&img, Rgb([128, 128, 128]), Toleranzen { innen: 30.0, aussen: 90.0, loch: 0.0, loch_min: 0 });

        assert_eq!(maske.get_pixel(0, 0)[0], 0, "Rand ist Hintergrund");
        assert_eq!(maske.get_pixel(3, 3)[0], 255, "eingeschlossenes Grau bleibt deckend");
    }

    #[test]
    fn kanten_werden_weich_ueberblendet() {
        // Abstand 60 liegt genau in der Mitte zwischen innen=30 und aussen=90,
        // also halbdeckend. Ein harter Schwellwert gäbe hier 0 oder 255 und
        // damit Treppenkanten an jeder weich gemalten Silhouette.
        let mut img = RgbImage::from_pixel(3, 3, Rgb([128, 128, 128]));
        img.put_pixel(1, 1, Rgb([188, 128, 128]));

        let maske = hintergrund_maske(&img, Rgb([128, 128, 128]), Toleranzen { innen: 30.0, aussen: 90.0, loch: 0.0, loch_min: 0 });

        assert_eq!(maske.get_pixel(1, 1)[0], 128, "halber Abstand, halbes Alpha");
    }

    #[test]
    fn eingeschlossene_hintergrundflaeche_wird_entfernt() {
        // Die Lücke zwischen den Beinen: unten vom Sockel geschlossen, vom
        // Bildrand aus also nicht erreichbar — bleibt ohne Lochsuche grau stehen.
        let mut img = RgbImage::from_pixel(7, 7, Rgb([128, 128, 128]));
        for y in 1..6 {
            for x in 1..6 {
                img.put_pixel(x, y, Rgb([200, 30, 30]));
            }
        }
        img.put_pixel(3, 3, Rgb([128, 128, 128]));

        let tol = Toleranzen { innen: 30.0, aussen: 90.0, loch: 30.0, loch_min: 1 };
        let maske = hintergrund_maske(&img, Rgb([128, 128, 128]), tol);

        assert_eq!(maske.get_pixel(3, 3)[0], 0, "eingeschlossene Hintergrundfarbe wird transparent");
        assert_eq!(maske.get_pixel(2, 2)[0], 255, "das Motiv drumherum bleibt deckend");
    }

    #[test]
    fn eingeschlossenes_graublech_bleibt_deckend() {
        // Rüstung in Grau, aber nicht in Hintergrundgrau: Abstand 60 liegt über
        // loch=30. Genau diese Unterscheidung trägt die Lochsuche — wäre die
        // Schwelle so weit wie `innen`, hätte die Figur hier ein Loch.
        let mut img = RgbImage::from_pixel(7, 7, Rgb([128, 128, 128]));
        for y in 1..6 {
            for x in 1..6 {
                img.put_pixel(x, y, Rgb([200, 30, 30]));
            }
        }
        img.put_pixel(3, 3, Rgb([188, 128, 128])); // Abstand 60 zum Hintergrund

        let tol = Toleranzen { innen: 70.0, aussen: 95.0, loch: 30.0, loch_min: 1 };
        let maske = hintergrund_maske(&img, Rgb([128, 128, 128]), tol);

        assert_eq!(maske.get_pixel(3, 3)[0], 255, "graue Rüstung bekommt kein Loch");
    }

    #[test]
    fn kleine_eingeschlossene_flecken_bleiben_deckend() {
        // Ein Glanzlicht auf Stahl hat zufällig Hintergrundfarbe. Es ist aber
        // winzig — anders als die Lücke zwischen den Beinen. Ohne Mindestfläche
        // löchert die Lochsuche jede Rüstung.
        let mut img = RgbImage::from_pixel(9, 9, Rgb([128, 128, 128]));
        for y in 1..8 {
            for x in 1..8 {
                img.put_pixel(x, y, Rgb([200, 30, 30]));
            }
        }
        // Ein einzelnes Pixel in Hintergrundfarbe = Glanzlicht.
        img.put_pixel(3, 3, Rgb([128, 128, 128]));
        // Ein 2x2-Block = die Lücke.
        for y in 5..7 {
            for x in 5..7 {
                img.put_pixel(x, y, Rgb([128, 128, 128]));
            }
        }

        let tol = Toleranzen { innen: 70.0, aussen: 95.0, loch: 30.0, loch_min: 4 };
        let maske = hintergrund_maske(&img, Rgb([128, 128, 128]), tol);

        assert_eq!(maske.get_pixel(3, 3)[0], 255, "Einzelpixel bleibt deckend");
        assert_eq!(maske.get_pixel(5, 5)[0], 0, "die 2x2-Lücke wird transparent");
    }

    #[test]
    fn gemalter_schatten_auf_dem_motiv_wird_kein_loch() {
        // Auf dem Sockel gemalte graue Schatten liegen nah an der
        // Hintergrundfarbe, sind aber Teil der Figur. Ein echtes Loch zeigt
        // dagegen den Hintergrund selbst und trifft dessen Farbe exakt.
        // Gemessen an einem 1024er Token: der gemalte Schatten lag ~25 daneben,
        // die Lücke zwischen den Beinen bei 0.
        let key = Rgb([164, 164, 162]);
        let mut img = RgbImage::from_pixel(60, 60, key);
        for y in 5..55 {
            for x in 5..55 {
                img.put_pixel(x, y, Rgb([232, 222, 208])); // heller Sockel
            }
        }
        // Eingeschlossener gemalter Schatten: 25 vom Hintergrund entfernt und mit
        // 900 Pixeln deutlich über der Mindestfläche — nur die Farbschwelle
        // kann ihn noch retten.
        for y in 15..45 {
            for x in 15..45 {
                img.put_pixel(x, y, Rgb([178, 178, 176]));
            }
        }

        let maske = hintergrund_maske(&img, key, Toleranzen::default());

        assert_eq!(maske.get_pixel(30, 30)[0], 255, "gemalter Schatten bleibt deckend");
        assert_eq!(maske.get_pixel(0, 0)[0], 0, "der echte Hintergrund geht weg");
    }

    #[test]
    fn echte_luecke_in_hintergrundfarbe_verschwindet_weiterhin() {
        let key = Rgb([164, 164, 162]);
        let mut img = RgbImage::from_pixel(60, 60, key);
        for y in 5..55 {
            for x in 5..55 {
                img.put_pixel(x, y, Rgb([232, 222, 208]));
            }
        }
        // Eingeschlossene Lücke, exakt Hintergrundfarbe, groß genug.
        for y in 20..45 {
            for x in 20..45 {
                img.put_pixel(x, y, key);
            }
        }

        let maske = hintergrund_maske(&img, key, Toleranzen::default());

        assert_eq!(maske.get_pixel(32, 32)[0], 0, "die Lücke wird transparent");
    }

    #[test]
    fn halbtransparente_kante_verliert_die_hintergrundfarbe() {
        // Randpixel sind Mischungen: c = a*Motiv + (1-a)*Hintergrund. Bei 50 %
        // Rot auf Grau steht also (228, 79, 79) im Bild — despill muss daraus
        // wieder das reine Rot (200, 30, 30) machen.
        let mut img = RgbaImage::new(1, 1);
        img.put_pixel(0, 0, Rgba([228, 79, 79, 128]));

        let out = despill(&img, Rgb([255, 128, 128]));

        let px = out.get_pixel(0, 0);
        assert!((px[0] as i32 - 200).abs() <= 2, "Rot: {px:?}");
        assert!((px[1] as i32 - 30).abs() <= 2, "Grün: {px:?}");
        assert_eq!(px[3], 128, "Alpha bleibt unangetastet");
    }

    #[test]
    fn deckende_und_leere_pixel_bleiben_unberuehrt() {
        let mut img = RgbaImage::new(3, 1);
        img.put_pixel(0, 0, Rgba([200, 30, 30, 255])); // voll deckend
        img.put_pixel(1, 0, Rgba([255, 128, 128, 0])); // voll transparent
        img.put_pixel(2, 0, Rgba([250, 126, 126, 2])); // fast transparent

        let out = despill(&img, Rgb([255, 128, 128]));

        assert_eq!(out.get_pixel(0, 0), &Rgba([200, 30, 30, 255]), "deckend bleibt");
        assert_eq!(out.get_pixel(1, 0), &Rgba([255, 128, 128, 0]), "keine Division durch 0");
        // Bei alpha 2/255 würde die Division die Farbe ins Absurde ziehen.
        assert_eq!(out.get_pixel(2, 0), &Rgba([250, 126, 126, 2]), "unter der Schwelle unberührt");
    }

    /// Rundlauf mit bekannter Wahrheit: ein Motiv mit weicher Kante auf Magenta
    /// legen, wieder freistellen — und prüfen, dass die Randfarbe zurückkommt.
    /// Genau dieser Fall ist der Grund für Despill: ohne ihn bliebe der rosa
    /// Saum in den halbtransparenten Pixeln stehen.
    #[test]
    fn rundlauf_ueber_magenta_stellt_die_randfarbe_wieder_her() {
        let magenta = Rgb([255, 0, 255]);

        // 9x9: deckender roter Block, außen herum ein halbtransparenter Rand.
        let mut original = RgbaImage::new(9, 9);
        for y in 2..7 {
            for x in 2..7 {
                let rand = x == 2 || x == 6 || y == 2 || y == 6;
                original.put_pixel(x, y, Rgba([200, 30, 30, if rand { 128 } else { 255 }]));
            }
        }

        let auf_magenta = crate::matting::auf_hintergrund(&original, magenta);
        let tol = Toleranzen { innen: 20.0, aussen: 200.0, loch: 0.0, loch_min: 0 };
        let maske = hintergrund_maske(&auf_magenta, magenta, tol);
        let freigestellt = crate::matting::apply_mask_as_alpha(&auf_magenta, &maske);
        let entfaerbt = despill(&freigestellt, magenta);

        // Kantenpixel: ohne Despill stünde hier die Mischung mit Magenta.
        let kante = entfaerbt.get_pixel(2, 4);
        assert!(kante[3] > 0 && kante[3] < 255, "Kante ist halbtransparent: {kante:?}");
        assert!((kante[0] as i32 - 200).abs() <= 6, "Rot zurückgewonnen: {kante:?}");
        assert!(kante[2] < 60, "kein Magenta-Saum mehr: {kante:?}");

        // Die Mitte war nie gemischt und muss unverändert sein.
        assert_eq!(entfaerbt.get_pixel(4, 4), &Rgba([200, 30, 30, 255]));
    }
}
