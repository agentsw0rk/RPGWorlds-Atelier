//! Demo-Engine: erzeugt ohne Modelle bunte Platzhalterbilder.
//!
//! Zum Ausprobieren der Weboberfläche, ohne mehrere GB Gewichte und ohne
//! Rechenzeit (`FLUX2_DEMO=1`). Das Bild ist auf mittelgrauem Grund eine
//! farbige Figur, deren Farbe und Form aus Seed und Prompt entstehen — gleicher
//! Auftrag, gleiches Bild. Der graue Grund lässt sich per Keying freistellen,
//! sodass auch Tokens in der Demo ihren Weg durchlaufen.

use crate::lauf::{Engine, Erzeugung, Host};
use crate::modelle::Modellsatz;
use anyhow::Result;
use image::{Rgb, RgbImage};
use std::time::Duration;

pub struct DemoEngine {
    /// Künstliche Wartezeit je Bild, damit Warteschlange und Fortschritt sichtbar werden.
    pub pause: Duration,
}

impl DemoEngine {
    pub fn neu() -> Self {
        DemoEngine { pause: Duration::from_millis(1500) }
    }
}

/// Einfacher, stabiler Hash (FNV-1a) — `DefaultHasher` ist zwischen Versionen nicht festgelegt.
fn hash(text: &str, seed: i64) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325 ^ seed as u64;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn farbe(h: u64, versatz: u32) -> Rgb<u8> {
    let r = ((h >> versatz) & 0xff) as u8;
    let g = ((h >> (versatz + 8)) & 0xff) as u8;
    let b = ((h >> (versatz + 16)) & 0xff) as u8;
    // Weg vom mittleren Grau (128), damit sich die Figur vom Grund abhebt.
    let weg = |c: u8| if c.abs_diff(128) < 60 { c ^ 0xC0 } else { c };
    Rgb([weg(r), weg(g), weg(b)])
}

/// Das Platzhalterbild: grauer Grund, farbige Ellipse (Körper) mit Kreis (Kopf).
pub fn platzhalter(prompt: &str, seed: i64, breite: u32, hoehe: u32) -> RgbImage {
    let h = hash(prompt, seed);
    let (koerper, kopf, band) = (farbe(h, 0), farbe(h, 16), farbe(h, 32));
    let (w, hh) = (breite as f32, hoehe as f32);
    let (mx, my) = (w / 2.0, hh * 0.58);
    let (rx, ry) = (w * 0.22, hh * 0.28);
    let (kx, ky, kr) = (mx, hh * 0.24, w.min(hh) * 0.11);
    RgbImage::from_fn(breite, hoehe, |x, y| {
        let (x, y) = (x as f32, y as f32);
        if ((x - kx).powi(2) + (y - ky).powi(2)).sqrt() < kr {
            kopf
        } else if ((x - mx) / rx).powi(2) + ((y - my) / ry).powi(2) < 1.0 {
            // Ein Band quer über den Körper, damit nicht jedes Bild ein Einheitsklecks ist.
            if ((y - my) / ry).abs() < 0.12 { band } else { koerper }
        } else {
            Rgb([128, 128, 128])
        }
    })
}

impl Engine for DemoEngine {
    fn braucht_modelle(&self) -> bool {
        false
    }

    fn erzeuge(&mut self, _: &Modellsatz, _: &Host, e: &Erzeugung) -> Result<()> {
        std::thread::sleep(self.pause);
        platzhalter(e.prompt, e.seed, e.width, e.height).save(e.ziel)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gleicher_auftrag_gleiches_bild_anderer_seed_anderes_bild() {
        let a = platzhalter("dwarf", 1, 64, 64);
        assert_eq!(a, platzhalter("dwarf", 1, 64, 64));
        assert_ne!(a, platzhalter("dwarf", 2, 64, 64));
        assert_ne!(a, platzhalter("elf", 1, 64, 64));
    }

    #[test]
    fn bild_hat_die_verlangte_groesse_und_grauen_grund_in_der_ecke() {
        let b = platzhalter("x", 5, 96, 160);
        assert_eq!(b.dimensions(), (96, 160));
        assert_eq!(b.get_pixel(0, 0), &Rgb([128, 128, 128]));
        assert_ne!(b.get_pixel(48, 100), &Rgb([128, 128, 128]), "in der Mitte steht die Figur");
    }

    #[test]
    fn figur_hebt_sich_vom_grund_ab() {
        for seed in 0..200 {
            let b = platzhalter("x", seed, 64, 64);
            let mitte = b.get_pixel(32, 40);
            let abstand: i32 = (0..3).map(|i| (mitte[i] as i32 - 128).abs()).max().unwrap();
            assert!(abstand >= 60, "Seed {seed}: Figur zu nah am Grund ({mitte:?})");
        }
    }

    #[test]
    fn demo_braucht_keine_modelle() {
        assert!(!DemoEngine::neu().braucht_modelle());
    }
}
