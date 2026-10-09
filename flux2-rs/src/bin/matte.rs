//! Freistellen: RGB-Bild rein, RGBA mit transparentem Hintergrund raus.
//!
//! Die Maske liefert u2netp (Salient Object Detection) über tract-onnx —
//! reines Rust, keine C++-Laufzeit nötig.
//!
//! Ohne OUT_W/OUT_H behält das Ergebnis die Originalabmessungen — nur der
//! Hintergrund wird alpha = 0. Mit OUT_W/OUT_H wird zusätzlich auf das Motiv
//! zugeschnitten und in eine Fläche dieser Größe eingepasst (Token-Variante).
//!
//! ```sh
//! matte eingabe.png ausgabe.png                       # nur Hintergrund raus
//! OUT_W=138 OUT_H=244 matte eingabe.png token.png      # freigestellt + eingepasst
//! ```
use anyhow::{bail, Context, Result};
use flux2_rs::matting::{apply_mask_as_alpha, crop_to_content, fit_into, transparenz_anteil};
use flux2_rs::keying::{despill, hintergrund_maske, hintergrundfarbe, Toleranzen};
use flux2_rs::params::{canvas, env_flag, hex_farbe, Canvas};
use flux2_rs::saliency;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let (src, dst) = match (args.next(), args.next()) {
        (Some(s), Some(d)) => (s, d),
        _ => bail!("Aufruf: matte <eingabe.png> <ausgabe.png>"),
    };
    let model_path = std::env::var("MODEL")
        .unwrap_or_else(|_| format!("{}/matting/u2netp.onnx", std::env::var("MODELS_DIR").unwrap_or_else(|_| "/workspace/models".into())));
    let canvas_mode = canvas(env_num("OUT_W"), env_num("OUT_H"))?;
    // Alpha unter diesem Wert gilt als Hintergrund und wird hart auf 0 gezogen.
    let cutoff: u8 = env_num("CUTOFF").unwrap_or(12);

    let rgb = image::open(&src)
        .with_context(|| format!("{src} konnte nicht gelesen werden"))?
        .to_rgb8();
    let (w, h) = rgb.dimensions();
    println!("Eingabe: {src} ({w}x{h})");

    // BG_KEY schaltet auf Farb-Keying um: `auto` liest die Hintergrundfarbe vom
    // Bildrand, `rrggbb` gibt sie vor. Ohne BG_KEY bleibt es bei u2netp.
    //
    // Für Bilder mit bewusst einfarbigem Hintergrund ist Keying die bessere
    // Wahl: u2netp sucht *ein* hervorstechendes Objekt und lässt Sockel,
    // Podeste und Schatten weg, auch wenn sie zum Motiv gehören sollen.
    // Leer gesetzt ist wie nicht gesetzt — das Script übergibt BG_KEY="" für
    // den Saliency-Weg.
    // Die Key-Farbe wird später noch gebraucht: nur mit ihr lässt sich der
    // Farbsaum aus den Randpixeln herausrechnen.
    let (mask, key_farbe) = match std::env::var("BG_KEY").ok().filter(|v| !v.trim().is_empty()) {
        None => (saliency::maske(std::path::Path::new(&model_path), &rgb, cutoff)?, None),
        Some(wert) => {
            let key = if wert.trim().eq_ignore_ascii_case("auto") {
                let erkannt = hintergrundfarbe(&rgb);
                println!(
                    "Hintergrundfarbe vom Rand gelesen: #{:02x}{:02x}{:02x}",
                    erkannt[0], erkannt[1], erkannt[2]
                );
                erkannt
            } else {
                let [r, g, b] = hex_farbe(&wert)?;
                image::Rgb([r, g, b])
            };
            // Gemessen an generierten Tokens auf mittelgrauem Grund: unter 70 bleibt
            // der weiche Schlagschatten stehen, über ~98 läuft die Füllung durch
            // helle Hauttöne und schlägt Löcher ins Gesicht.
            let standard = Toleranzen::default();
            let tol = Toleranzen {
                innen: env_num("KEY_INNEN").unwrap_or(standard.innen),
                aussen: env_num("KEY_AUSSEN").unwrap_or(standard.aussen),
                loch: env_num("KEY_LOCH").unwrap_or(standard.loch),
                loch_min: env_num("KEY_LOCH_MIN").unwrap_or(standard.loch_min),
            };
            if tol.innen >= tol.aussen {
                bail!(
                    "KEY_INNEN ({}) muss kleiner als KEY_AUSSEN ({}) sein.",
                    tol.innen,
                    tol.aussen
                );
            }
            (hintergrund_maske(&rgb, key, tol), Some(key))
        }
    };
    // MASK_OUT schreibt die rohe Maske als Graustufenbild. Wenn beim Freistellen
    // etwas fehlt, sieht man hier sofort, ob u2netp es gar nicht erkannt hat
    // (schwarz) oder ob es nur knapp unter CUTOFF liegt (dunkelgrau).
    if let Ok(pfad) = std::env::var("MASK_OUT") {
        mask.save(&pfad)
            .with_context(|| format!("Maske {pfad} konnte nicht geschrieben werden"))?;
        println!("Maske: {pfad}");
    }

    let rgba = apply_mask_as_alpha(&rgb, &mask);
    // Despill geht nur beim Keying: bei u2netp kennt niemand die Hintergrundfarbe.
    let rgba = match key_farbe {
        Some(key) if env_flag("DESPILL", true) => despill(&rgba, key),
        _ => rgba,
    };

    let final_img = match canvas_mode {
        Canvas::Original => rgba,
        Canvas::Fit(target_w, target_h) => {
            let cropped = crop_to_content(&rgba, cutoff)
                .context("Maske ist komplett leer — kein Motiv erkannt")?;
            let (cw, ch) = cropped.dimensions();
            println!("Freigestellt und zugeschnitten: {cw}x{ch}");
            fit_into(&cropped, target_w, target_h)
        }
    };
    let (fw, fh) = final_img.dimensions();
    final_img
        .save(&dst)
        .with_context(|| format!("{dst} konnte nicht geschrieben werden"))?;
    // Kontrollausgabe: ohne transparente Fläche ist das Freistellen fehlgeschlagen,
    // auch wenn die Datei formal ein gültiges RGBA-PNG ist.
    let transparent = transparenz_anteil(&final_img) * 100.0;
    println!("Fertig: {dst} ({fw}x{fh}, RGBA, {transparent:.0} % der Fläche bei alpha = 0)");
    if transparent < 1.0 {
        eprintln!(
            "Warnung: fast nichts wurde transparent — u2netp hat kein klares Motiv gefunden. \
             Ein einfarbiger Hintergrund im Ausgangsbild hilft."
        );
    }
    Ok(())
}

fn env_num<T: std::str::FromStr>(key: &str) -> Option<T> {
    std::env::var(key).ok().and_then(|v| v.parse().ok())
}
