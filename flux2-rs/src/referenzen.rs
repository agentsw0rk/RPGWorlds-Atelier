//! Referenzbilder: hochgeladene Bilder ablegen und für das Modell aufbereiten.

use crate::matting::auf_hintergrund;
use anyhow::{bail, Context, Result};
use std::io::Cursor;
use std::path::{Path, PathBuf};

/// Obergrenze pro Upload in Bytes.
pub const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024;
/// Obergrenze pro Kante in Pixeln — ein Foto in Kameraauflösung wäre für die
/// Referenz ohnehin Verschwendung, und das Dekodieren soll keinen Speicher sprengen.
const MAX_KANTE_PX: u32 = 8192;

/// Verzeichnis der hochgeladenen Referenzbilder.
///
/// Gespeichert wird immer als PNG, neu kodiert: so landet nur ein tatsächlich
/// dekodierbares Bild auf der Platte, keine beliebige Datei unter fremdem Namen.
pub struct Uploads {
    ordner: PathBuf,
}

impl Uploads {
    pub fn neu(ordner: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&ordner)
            .with_context(|| format!("{} nicht anlegbar", ordner.display()))?;
        Ok(Uploads { ordner })
    }

    /// Legt ein Bild ab und gibt seine ID zurück.
    pub fn speichern(&self, daten: &[u8]) -> Result<String> {
        if daten.is_empty() {
            bail!("Leerer Upload.");
        }
        if daten.len() > MAX_UPLOAD_BYTES {
            bail!("Upload ist größer als {} MB.", MAX_UPLOAD_BYTES / 1024 / 1024);
        }
        let mut leser = image::ImageReader::new(Cursor::new(daten))
            .with_guessed_format()
            .context("Format nicht erkennbar")?;
        let mut grenzen = image::Limits::default();
        grenzen.max_image_width = Some(MAX_KANTE_PX);
        grenzen.max_image_height = Some(MAX_KANTE_PX);
        leser.limits(grenzen);
        let bild = leser.decode().context("Das ist kein lesbares Bild (PNG, JPEG, WebP, …)")?;

        let id = crate::zufall::hex(8);
        bild.save_with_format(self.pfad_roh(&id), image::ImageFormat::Png)
            .context("Upload nicht speicherbar")?;
        Ok(id)
    }

    fn pfad_roh(&self, id: &str) -> PathBuf {
        self.ordner.join(format!("{id}.png"))
    }

    /// Pfad zu einer ID. Die ID muss genau das Format haben, das `speichern`
    /// vergibt — sonst könnte `../etc/passwd` als ID durchgehen.
    pub fn pfad(&self, id: &str) -> Result<PathBuf> {
        let gueltig = id.len() == 16 && id.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'));
        if !gueltig {
            bail!("Upload '{id}' gibt es nicht.");
        }
        let pfad = self.pfad_roh(id);
        if !pfad.is_file() {
            bail!("Upload '{id}' gibt es nicht.");
        }
        Ok(pfad)
    }
}

/// Zielmaße einer Referenz bei einer Obergrenze für die lange Kante.
///
/// Das Seitenverhältnis bleibt, es wird nie vergrößert, und beide Kanten werden
/// auf ein Vielfaches von 16 abgerundet (so groß ist ein Latent-Feld des VAE),
/// mindestens 16. Ganzzahlrechnung: mit Fließkomma würde 1402·(512/1402) leicht
/// unter 512 landen und eine Kachel verlieren.
pub fn ziel_masse(breite: u32, hoehe: u32, max_px: u32) -> (u32, u32) {
    let lang = u64::from(breite.max(hoehe));
    let max = u64::from(max_px).min(lang);
    let auf_16 = |kante: u32| ((u64::from(kante) * max / lang) as u32 / 16 * 16).max(16);
    (auf_16(breite), auf_16(hoehe))
}

/// Ersetzt Referenzbilder mit Alphakanal durch eine deckende Fassung in `ordner`.
///
/// `diffusion-rs` reicht Referenzbilder mit `to_rgb8()` weiter und wirft Alpha
/// dabei weg — transparente Flächen kämen im Modell als **Schwarz** an. Ein
/// bereits freigestelltes Token als Vorlage würde also "schwarzer Hintergrund"
/// sagen. Deshalb wird es vorher auf `hintergrund` gelegt.
pub fn deckend_machen(refs: &[PathBuf], hintergrund: [u8; 3], ordner: &Path) -> Result<Vec<PathBuf>> {
    let mut ergebnis = Vec::with_capacity(refs.len());
    for (i, pfad) in refs.iter().enumerate() {
        let bild = image::open(pfad)
            .with_context(|| format!("Referenzbild {} nicht lesbar", pfad.display()))?;
        let rgba = bild.to_rgba8();
        if rgba.pixels().all(|px| px[3] == 255) {
            ergebnis.push(pfad.clone());
            continue;
        }
        let deckend = auf_hintergrund(&rgba, image::Rgb(hintergrund));
        let ziel = ordner.join(format!("ref-{i}-deckend.png"));
        deckend
            .save(&ziel)
            .with_context(|| format!("{} nicht schreibbar", ziel.display()))?;
        ergebnis.push(ziel);
    }
    Ok(ergebnis)
}

/// Verkleinert Referenzbilder auf höchstens `max_px` an der langen Kante.
///
/// Weniger Referenz-Pixel heißen weniger Tokens im Transformer, und die
/// Attention wächst quadratisch mit der Tokenzahl: das ist der größte Hebel für
/// die Laufzeit. Bilder, die schon passen (und durch 16 teilbar sind), bleiben
/// unberührt; die übrigen landen als `ref-<i>-klein.png` in `ordner`.
pub fn begrenzen(refs: &[PathBuf], max_px: u32, ordner: &Path) -> Result<Vec<PathBuf>> {
    let mut ergebnis = Vec::with_capacity(refs.len());
    for (i, pfad) in refs.iter().enumerate() {
        let (b, h) = image::image_dimensions(pfad)
            .with_context(|| format!("Referenzbild {} nicht lesbar", pfad.display()))?;
        let ziel = ziel_masse(b, h, max_px);
        if ziel == (b, h) {
            ergebnis.push(pfad.clone());
            continue;
        }
        let klein = image::open(pfad)
            .with_context(|| format!("Referenzbild {} nicht lesbar", pfad.display()))?
            .resize_exact(ziel.0, ziel.1, image::imageops::FilterType::Lanczos3);
        let datei = ordner.join(format!("ref-{i}-klein.png"));
        klein.save(&datei).with_context(|| format!("{} nicht schreibbar", datei.display()))?;
        ergebnis.push(datei);
    }
    Ok(ergebnis)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, Rgb, Rgba, RgbaImage};

    fn png_bytes(bild: &image::DynamicImage) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        bild.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn uploads() -> (tempfile::TempDir, Uploads) {
        let dir = tempfile::tempdir().unwrap();
        let u = Uploads::neu(dir.path().join("uploads")).unwrap();
        (dir, u)
    }

    #[test]
    fn ein_gueltiges_bild_wird_gespeichert_und_ist_ueber_die_id_auffindbar() {
        let (_dir, u) = uploads();
        let bild = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(8, 8, Rgb([1, 2, 3])));
        let id = u.speichern(&png_bytes(&bild)).unwrap();
        assert_eq!(id.len(), 16);
        let pfad = u.pfad(&id).unwrap();
        assert_eq!(image::open(pfad).unwrap().to_rgb8().get_pixel(0, 0), &Rgb([1, 2, 3]));
    }

    #[test]
    fn jpeg_wird_als_png_abgelegt() {
        let (_dir, u) = uploads();
        let bild = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(8, 8, Rgb([9, 9, 9])));
        let mut jpg = Cursor::new(Vec::new());
        bild.write_to(&mut jpg, ImageFormat::Jpeg).unwrap();
        let id = u.speichern(&jpg.into_inner()).unwrap();
        assert!(u.pfad(&id).unwrap().extension().is_some_and(|e| e == "png"));
    }

    #[test]
    fn kein_bild_wird_abgelehnt() {
        let (_dir, u) = uploads();
        assert!(u.speichern(b"das ist kein bild").is_err());
        assert!(u.speichern(b"").is_err());
    }

    #[test]
    fn zu_grosser_upload_wird_abgelehnt() {
        let (_dir, u) = uploads();
        let fehler = u.speichern(&vec![0u8; MAX_UPLOAD_BYTES + 1]).unwrap_err().to_string();
        assert!(fehler.contains("MB"), "{fehler}");
    }

    #[test]
    fn pfadtricks_in_der_id_werden_abgelehnt() {
        let (_dir, u) = uploads();
        for boese in ["../etc/passwd", "..", "abc", "ABCDEF0123456789", "0123456789abcdeg", ""] {
            assert!(u.pfad(boese).is_err(), "{boese:?}");
        }
    }

    #[test]
    fn unbekannte_id_im_richtigen_format_ist_ein_fehler() {
        let (_dir, u) = uploads();
        assert!(u.pfad("0123456789abcdef").is_err());
    }

    #[test]
    fn deckende_referenz_bleibt_unveraendert() {
        let dir = tempfile::tempdir().unwrap();
        let pfad = dir.path().join("a.png");
        image::RgbImage::from_pixel(4, 4, Rgb([5, 5, 5])).save(&pfad).unwrap();
        let aus = deckend_machen(&[pfad.clone()], [255, 255, 255], dir.path()).unwrap();
        assert_eq!(aus, vec![pfad]);
    }

    #[test]
    fn transparente_referenz_wird_auf_den_hintergrund_gelegt() {
        let dir = tempfile::tempdir().unwrap();
        let pfad = dir.path().join("t.png");
        RgbaImage::from_pixel(4, 4, Rgba([0, 0, 0, 0])).save(&pfad).unwrap();
        let aus = deckend_machen(&[pfad.clone()], [255, 255, 255], dir.path()).unwrap();
        assert_ne!(aus[0], pfad);
        let bild = image::open(&aus[0]).unwrap().to_rgb8();
        assert_eq!(bild.get_pixel(0, 0), &Rgb([255, 255, 255]), "transparent wird weiß, nicht schwarz");
    }

    #[test]
    fn fehlende_referenz_ist_ein_fehler_mit_pfad() {
        let dir = tempfile::tempdir().unwrap();
        let fehler = deckend_machen(&[dir.path().join("nix.png")], [0; 3], dir.path()).unwrap_err().to_string();
        assert!(fehler.contains("nix.png"), "{fehler}");
    }

    #[test]
    fn zielmasse_begrenzen_die_lange_kante_ohne_hochzuskalieren_und_runden_auf_16_ab() {
        // 1122x1402 auf lange Kante 512: Faktor 0,365 → 409,7 x 512 → 400 x 512.
        assert_eq!(ziel_masse(1122, 1402, 512), (400, 512));
        // Querformat genauso.
        assert_eq!(ziel_masse(1402, 1122, 512), (512, 400));
        // Kleiner als die Grenze: nicht vergrößern, nur auf 16 abrunden.
        assert_eq!(ziel_masse(300, 200, 512), (288, 192));
        // Extrem schmal: mindestens 16 Pixel.
        assert_eq!(ziel_masse(4000, 20, 512), (512, 16));
    }

    #[test]
    fn begrenzen_verkleinert_zu_grosse_referenzen_und_laesst_passende_unveraendert() {
        let dir = tempfile::tempdir().unwrap();
        let gross = dir.path().join("gross.png");
        image::RgbImage::from_pixel(128, 96, Rgb([9, 9, 9])).save(&gross).unwrap();
        let passt = dir.path().join("passt.png");
        image::RgbImage::from_pixel(48, 32, Rgb([1, 1, 1])).save(&passt).unwrap();

        let aus = begrenzen(&[gross.clone(), passt.clone()], 64, dir.path()).unwrap();

        assert_ne!(aus[0], gross, "die große bekommt eine verkleinerte Kopie");
        assert_eq!(image::image_dimensions(&aus[0]).unwrap(), ziel_masse(128, 96, 64));
        assert_eq!(aus[1], passt, "48x32 ist schon klein genug und durch 16 teilbar");
    }
}
