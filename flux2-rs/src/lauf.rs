//! Führt einen Plan aus: Modelle beschaffen, je Bild erzeugen, freistellen.
//!
//! Das ist der Ablauf von `generate-macos.sh` ohne die Shell: Download →
//! Generierung → Freistellen, ein Bild nach dem anderen. Die eigentliche
//! Bilderzeugung steckt hinter dem Trait `Engine`, damit dieser Ablauf ohne
//! Modellgewichte getestet werden kann (und nie ein Bild erzeugt, wo es nicht
//! soll).
//!
//! Ein fehlgeschlagenes Bild bricht den Lauf nicht ab: bei mehreren Seeds soll
//! ein Fehler nicht einen ganzen Abend wegwerfen — wie in `generate-macos.sh`.

use crate::auftrag::{Aufgabe, Plan};
use crate::metriken::{Kontext, Metriken};
use crate::modelle::{self, Modellsatz};
use crate::nachbearbeitung;
use crate::referenzen;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// Einstellungen des Rechners, nicht des Auftrags.
#[derive(Debug, Clone)]
pub struct Host {
    pub modelle: PathBuf,
    pub threads: i32,
    /// `enable_mmap` bindet die Gewichte an einen CPU-Buffer; auf Metal/CUDA
    /// führt das zu einem grauen Bild ohne Fehlermeldung. Deshalb Default aus.
    pub mmap: bool,
    pub flash_attention: bool,
    pub vae_tiling: bool,
    /// Hugging-Face-Token für gated Repos.
    pub hf_token: Option<String>,
    /// Farbe, auf die transparente Referenzbilder gelegt werden.
    pub ref_bg: [u8; 3],
    /// Vorgabe für die Obergrenze der Referenzgröße (lange Kante in Pixeln);
    /// ein Auftrag mit eigenem `ref_max_px` geht vor. `None`: sd.cpps Automatik (1 MP).
    pub ref_max_px: Option<u32>,
}

/// Ein einzelner Erzeugungsschritt, wie ihn die Engine bekommt.
#[derive(Debug, Clone)]
pub struct Erzeugung<'a> {
    pub prompt: &'a str,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub cfg: f32,
    pub guidance: f32,
    pub seed: i64,
    /// Bereits deckend gemachte Referenzbilder.
    pub refs: &'a [PathBuf],
    pub wtype: Option<&'a str>,
    /// Die Referenzen sind schon auf ihre Zielgröße gebracht: die Engine muss
    /// sd.cpps eigene Skalierung abschalten, die sie sonst wieder auf 1 MP
    /// aufblasen würde.
    pub ref_selbst_skaliert: bool,
    /// Metrikstrom des Jobs, wenn beobachtet wird. Die echte Engine leitet
    /// damit das sd.cpp-Log hinein.
    pub metriken: Option<&'a Metriken>,
    /// Hierhin schreibt die Engine das Rohbild (PNG).
    pub ziel: &'a Path,
}

/// Die Bilderzeugung. Die echte Fassung (`engine::SdEngine`) ruft
/// stable-diffusion.cpp; Tests setzen eine Attrappe ein.
pub trait Engine {
    /// Ob die Engine Modellgewichte braucht. Nur die Demo-Engine verneint das;
    /// dann wird nichts heruntergeladen.
    fn braucht_modelle(&self) -> bool {
        true
    }

    fn erzeuge(&mut self, satz: &Modellsatz, host: &Host, e: &Erzeugung) -> Result<()>;
}

/// Was während des Laufs passiert. Der Aufrufer macht daraus Zustand und
/// Fortschrittsmeldungen.
#[derive(Debug, Clone, PartialEq)]
pub enum Ereignis {
    /// Eine Meldung wie "lade Text-Encoder …".
    Meldung(String),
    BildStart(usize),
    BildFertig { index: usize, datei: String, roh: Option<String>, dauer_s: u64 },
    BildFehler { index: usize, fehler: String },
    BildAbgebrochen(usize),
}

fn bild_fehler(index: usize, fehler: impl std::fmt::Display) -> Ereignis {
    Ereignis::BildFehler { index, fehler: format!("{fehler:#}") }
}

/// Beschafft die Modelle, die dieser Plan braucht. Ein Fehler hier betrifft
/// den ganzen Job — ohne Gewichte gibt es kein einziges Bild.
fn modelle_beschaffen(
    satz: &Modellsatz,
    plan: &Plan,
    host: &Host,
    bericht: &mut dyn FnMut(Ereignis),
) -> Result<()> {
    // u2netp nur, wenn tatsächlich über Saliency freigestellt wird.
    let braucht_u2netp = plan
        .aufgaben
        .iter()
        .any(|a| !a.uebersprungen && a.freistellen.as_ref().is_some_and(|f| !f.keying));
    for datei in satz.benoetigt(braucht_u2netp) {
        modelle::beschaffen(datei, host.hf_token.as_deref(), &mut |m| bericht(Ereignis::Meldung(m.into())))?;
    }
    Ok(())
}

/// Erzeugt ein Bild und stellt es frei. Gibt Dateiname, Rohbild und Dauer zurück.
fn bild_erzeugen(
    aufgabe: &Aufgabe,
    plan: &Plan,
    satz: &Modellsatz,
    host: &Host,
    engine: &mut dyn Engine,
    ausgabe: &Path,
    arbeit: &Path,
    metriken: Option<&Metriken>,
) -> Result<(String, Option<String>, u64)> {
    let start = Instant::now();
    let endgueltig = format!("{}.png", aufgabe.stamm);
    // Das Rohbild bleibt neben dem freigestellten liegen — praktisch zum Vergleichen.
    let roh_name = format!("{}.raw.png", aufgabe.stamm);
    let (roh_pfad, roh) = match aufgabe.freistellen {
        Some(_) => (ausgabe.join(&roh_name), Some(roh_name)),
        None => (ausgabe.join(&endgueltig), None),
    };

    let phase = |name: &str| {
        if let Some(m) = metriken {
            m.phase(name);
        }
    };
    phase("referenz");
    let refs = referenzen::deckend_machen(&aufgabe.refs, host.ref_bg, arbeit)?;
    let ref_max_px = plan.ref_max_px.or(host.ref_max_px);
    let refs = match ref_max_px {
        Some(px) => referenzen::begrenzen(&refs, px, arbeit)?,
        None => refs,
    };
    if let Some(m) = metriken {
        m.kontext(Kontext {
            preset: satz.preset.name().into(),
            wtype: plan.wtype.clone(),
            breite: plan.width,
            hoehe: plan.height,
            steps: plan.steps,
            referenzen: refs.iter().filter_map(|p| image::image_dimensions(p).ok()).collect(),
            mmap: host.mmap,
            flash_attention: host.flash_attention,
            vae_tiling: host.vae_tiling,
            threads: host.threads,
            ref_max_px: ref_max_px.filter(|_| !refs.is_empty()),
        });
    }
    let erzeugung = Erzeugung {
        prompt: &aufgabe.prompt,
        width: plan.width,
        height: plan.height,
        steps: plan.steps,
        cfg: plan.cfg,
        guidance: plan.guidance,
        seed: aufgabe.seed,
        refs: &refs,
        wtype: plan.wtype.as_deref(),
        ref_selbst_skaliert: ref_max_px.is_some() && !refs.is_empty(),
        metriken,
        ziel: &roh_pfad,
    };
    phase("erzeugen");
    engine.erzeuge(satz, host, &erzeugung).context("Bildgenerierung fehlgeschlagen")?;

    if let Some(einstellung) = &aufgabe.freistellen {
        phase("freistellen");
        let rgb = image::open(&roh_pfad)
            .with_context(|| format!("Rohbild {} nicht lesbar", roh_pfad.display()))?
            .to_rgb8();
        let u2netp = (!einstellung.keying).then_some(satz.u2netp.pfad.as_path());
        let ergebnis = nachbearbeitung::freistellen(&rgb, einstellung, u2netp)?;
        ergebnis
            .bild
            .save(ausgabe.join(&endgueltig))
            .context("Freigestelltes Bild nicht schreibbar")?;
    }
    phase("bild_fertig");
    Ok((endgueltig, roh, start.elapsed().as_secs()))
}

/// Führt alle Aufgaben des Plans nacheinander aus.
///
/// `abbruch` wird zwischen zwei Bildern geprüft; ein laufendes Bild lässt sich
/// nicht unterbrechen, weil `stable-diffusion.cpp` blockiert, bis es fertig ist.
///
/// `Err` nur, wenn der Job gar nicht starten konnte (Modelle, Ausgabeordner).
/// Fehler einzelner Bilder kommen als `Ereignis::BildFehler`.
pub fn ausfuehren(
    plan: &Plan,
    ausgabe: &Path,
    host: &Host,
    engine: &mut dyn Engine,
    abbruch: &AtomicBool,
    bericht: &mut dyn FnMut(Ereignis),
) -> Result<()> {
    ausfuehren_mit_metriken(plan, ausgabe, host, engine, abbruch, bericht, None)
}

/// Wie `ausfuehren`, schreibt aber Phasen und Kontext in `metriken`.
pub fn ausfuehren_mit_metriken(
    plan: &Plan,
    ausgabe: &Path,
    host: &Host,
    engine: &mut dyn Engine,
    abbruch: &AtomicBool,
    bericht: &mut dyn FnMut(Ereignis),
    metriken: Option<&Metriken>,
) -> Result<()> {
    std::fs::create_dir_all(ausgabe)
        .with_context(|| format!("Ausgabeordner {} nicht anlegbar", ausgabe.display()))?;
    let satz = modelle::aufloesen(&host.modelle, plan.preset, &plan.quant, None)?;
    if engine.braucht_modelle() {
        modelle_beschaffen(&satz, plan, host, bericht)?;
    }

    let arbeit = ausgabe.join(".arbeit");
    std::fs::create_dir_all(&arbeit)?;

    let offen = plan.aufgaben.iter().filter(|a| !a.uebersprungen).count();
    let mut nummer = 0;
    for (index, aufgabe) in plan.aufgaben.iter().enumerate() {
        if aufgabe.uebersprungen {
            continue;
        }
        if abbruch.load(Ordering::Relaxed) {
            bericht(Ereignis::BildAbgebrochen(index));
            continue;
        }
        nummer += 1;
        bericht(Ereignis::Meldung(format!("Bild {nummer}/{offen} — {} (Seed {})", aufgabe.stamm, aufgabe.seed)));
        bericht(Ereignis::BildStart(index));

        match bild_erzeugen(aufgabe, plan, &satz, host, engine, ausgabe, &arbeit, metriken) {
            Ok((datei, roh, dauer_s)) => bericht(Ereignis::BildFertig { index, datei, roh, dauer_s }),
            Err(fehler) => {
                // Halbfertige Dateien wegräumen: sonst gilt das Bild beim
                // nächsten Batch-Lauf als vorhanden und fehlt am Ende still.
                let _ = std::fs::remove_file(ausgabe.join(format!("{}.png", aufgabe.stamm)));
                let _ = std::fs::remove_file(ausgabe.join(format!("{}.raw.png", aufgabe.stamm)));
                bericht(bild_fehler(index, fehler));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&arbeit);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auftrag::{self, Auftrag, Umgebung};
    use crate::kind::Kind;
    use crate::listen::Eintrag;
    use image::{Rgb, RgbImage};

    struct Test;
    impl Umgebung for Test {
        fn liste(&self, _: Kind) -> Result<Vec<Eintrag>> {
            Ok(vec![
                Eintrag { slug: "mira".into(), name: "Mira".into(), beschreibung: String::new(), prompt: "girl".into() },
                Eintrag { slug: "jonas".into(), name: "Jonas".into(), beschreibung: String::new(), prompt: "boy".into() },
            ])
        }
        fn upload(&self, id: &str) -> Result<PathBuf> {
            anyhow::bail!("kein Upload {id}")
        }
        fn wuerfeln(&mut self) -> i64 {
            1
        }
        fn vorhanden(&self, _: &str) -> bool {
            false
        }
    }

    /// Schreibt ein mittelgraues Bild mit rotem Quadrat — ein freistellbares Motiv.
    fn motiv(ziel: &Path) {
        RgbImage::from_fn(64, 64, |x, y| {
            if (20..44).contains(&x) && (20..44).contains(&y) { Rgb([200, 30, 30]) } else { Rgb([128, 128, 128]) }
        })
        .save(ziel)
        .unwrap();
    }

    #[derive(Default)]
    struct Attrappe {
        aufrufe: Vec<(String, i64, u32, u32)>,
        scheitere_bei_seed: Option<i64>,
        refs: Vec<Vec<PathBuf>>,
        metriken_da: Vec<bool>,
        /// Pixelmaße der Referenzen, wie sie bei der Engine ankommen, und ob die
        /// Engine ihre eigene Skalierung abschalten soll.
        ref_masse: Vec<Vec<(u32, u32)>>,
        selbst_skaliert: Vec<bool>,
    }

    impl Engine for Attrappe {
        fn erzeuge(&mut self, _: &Modellsatz, _: &Host, e: &Erzeugung) -> Result<()> {
            self.aufrufe.push((e.prompt.to_string(), e.seed, e.width, e.height));
            self.refs.push(e.refs.to_vec());
            self.metriken_da.push(e.metriken.is_some());
            self.ref_masse.push(e.refs.iter().map(|p| image::image_dimensions(p).unwrap()).collect());
            self.selbst_skaliert.push(e.ref_selbst_skaliert);
            if self.scheitere_bei_seed == Some(e.seed) {
                std::fs::write(e.ziel, b"halb")?;
                anyhow::bail!("Speicher voll");
            }
            motiv(e.ziel);
            Ok(())
        }
    }

    /// Ein Modellverzeichnis, in dem alle Dateien schon liegen — kein Download nötig.
    fn host(dir: &Path) -> Host {
        let modelle = dir.join("models");
        for datei in [
            "diffusion/flux-2-klein-9b-Q5_K_M.gguf",
            "text_encoder/Qwen3-8B-Q5_K_M.gguf",
            "vae/flux2-vae.safetensors",
            "matting/u2netp.onnx",
        ] {
            let pfad = modelle.join(datei);
            std::fs::create_dir_all(pfad.parent().unwrap()).unwrap();
            std::fs::write(pfad, b"x").unwrap();
        }
        Host { modelle, threads: 2, mmap: false, flash_attention: false, vae_tiling: false, hf_token: None, ref_bg: [255; 3], ref_max_px: None }
    }

    fn plan(json: &str) -> Plan {
        auftrag::planen(&serde_json::from_str::<Auftrag>(json).unwrap(), &mut Test).unwrap()
    }

    fn lauf(plan: &Plan, dir: &Path, engine: &mut Attrappe, abbruch: &AtomicBool) -> (Result<()>, Vec<Ereignis>) {
        let mut ereignisse = Vec::new();
        let ergebnis = ausfuehren(plan, &dir.join("aus"), &host(dir), engine, abbruch, &mut |e| ereignisse.push(e));
        (ergebnis, ereignisse)
    }

    fn fertige(ereignisse: &[Ereignis]) -> Vec<&str> {
        ereignisse
            .iter()
            .filter_map(|e| match e {
                Ereignis::BildFertig { datei, .. } => Some(datei.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn jedes_bild_wird_erzeugt_und_gemeldet() {
        let dir = tempfile::tempdir().unwrap();
        let p = plan(r#"{"kind":"fullbody","prompt":"girl","seeds":[1,2]}"#);
        let mut engine = Attrappe::default();
        let (ergebnis, ereignisse) = lauf(&p, dir.path(), &mut engine, &AtomicBool::new(false));
        ergebnis.unwrap();
        assert_eq!(fertige(&ereignisse), ["girl-s1.png", "girl-s2.png"]);
        assert!(dir.path().join("aus/girl-s1.png").is_file());
        assert_eq!(engine.aufrufe.iter().map(|a| a.1).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn die_engine_bekommt_groesse_und_prompt_aus_dem_plan() {
        let dir = tempfile::tempdir().unwrap();
        let p = plan(r#"{"kind":"fullbody","prompt":"girl"}"#);
        let mut engine = Attrappe::default();
        lauf(&p, dir.path(), &mut engine, &AtomicBool::new(false)).0.unwrap();
        assert_eq!((engine.aufrufe[0].2, engine.aufrufe[0].3), (768, 1536));
        assert!(engine.aufrufe[0].0.starts_with("girl, "));
    }

    #[test]
    fn token_wird_freigestellt_und_das_rohbild_bleibt_liegen() {
        let dir = tempfile::tempdir().unwrap();
        let p = plan(r#"{"kind":"token","prompt":"dwarf"}"#);
        let mut engine = Attrappe::default();
        let (ergebnis, ereignisse) = lauf(&p, dir.path(), &mut engine, &AtomicBool::new(false));
        ergebnis.unwrap();
        let Ereignis::BildFertig { datei, roh, .. } = ereignisse.iter().find(|e| matches!(e, Ereignis::BildFertig { .. })).unwrap() else {
            unreachable!()
        };
        assert_eq!(datei, "dwarf.png");
        assert_eq!(roh.as_deref(), Some("dwarf.raw.png"));
        let bild = image::open(dir.path().join("aus/dwarf.png")).unwrap();
        assert_eq!((bild.width(), bild.height()), (138, 244), "auf Tokengröße eingepasst");
        assert!(bild.color().has_alpha());
        assert!(dir.path().join("aus/dwarf.raw.png").is_file());
    }

    #[test]
    fn ein_fehlgeschlagenes_bild_stoppt_die_uebrigen_nicht_und_hinterlaesst_keine_datei() {
        let dir = tempfile::tempdir().unwrap();
        let p = plan(r#"{"kind":"fullbody","prompt":"girl","seeds":[1,2,3]}"#);
        let mut engine = Attrappe { scheitere_bei_seed: Some(2), ..Default::default() };
        let (ergebnis, ereignisse) = lauf(&p, dir.path(), &mut engine, &AtomicBool::new(false));
        ergebnis.unwrap();
        assert_eq!(fertige(&ereignisse), ["girl-s1.png", "girl-s3.png"]);
        let fehler = ereignisse.iter().find_map(|e| match e {
            Ereignis::BildFehler { index, fehler } => Some((*index, fehler.clone())),
            _ => None,
        });
        let (index, text) = fehler.expect("Fehler gemeldet");
        assert_eq!(index, 1);
        assert!(text.contains("Speicher voll"), "{text}");
        assert!(!dir.path().join("aus/girl-s2.png").exists(), "halbe Datei muss weg");
    }

    #[test]
    fn abbruch_wird_zwischen_zwei_bildern_beachtet() {
        let dir = tempfile::tempdir().unwrap();
        let p = plan(r#"{"kind":"fullbody","prompt":"girl","seeds":2}"#);
        let mut engine = Attrappe::default();
        let (ergebnis, ereignisse) = lauf(&p, dir.path(), &mut engine, &AtomicBool::new(true));
        ergebnis.unwrap();
        assert!(engine.aufrufe.is_empty(), "nach dem Abbruch startet nichts mehr");
        assert_eq!(ereignisse.iter().filter(|e| matches!(e, Ereignis::BildAbgebrochen(_))).count(), 2);
    }

    #[test]
    fn vorhandene_listenbilder_werden_nicht_neu_erzeugt() {
        struct MiraDa;
        impl Umgebung for MiraDa {
            fn liste(&self, k: Kind) -> Result<Vec<Eintrag>> {
                Test.liste(k)
            }
            fn upload(&self, id: &str) -> Result<PathBuf> {
                Test.upload(id)
            }
            fn wuerfeln(&mut self) -> i64 {
                1
            }
            fn vorhanden(&self, datei: &str) -> bool {
                datei == "mira.png"
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let a: Auftrag = serde_json::from_str(r#"{"kind":"fullbody","liste":{}}"#).unwrap();
        let p = auftrag::planen(&a, &mut MiraDa).unwrap();
        let mut engine = Attrappe::default();
        let (ergebnis, ereignisse) = lauf(&p, dir.path(), &mut engine, &AtomicBool::new(false));
        ergebnis.unwrap();
        assert_eq!(fertige(&ereignisse), ["jonas.png"]);
        assert_eq!(engine.aufrufe.len(), 1);
    }

    #[test]
    fn engine_ohne_modelle_loest_keinen_download_aus() {
        struct Leer;
        impl Engine for Leer {
            fn braucht_modelle(&self) -> bool {
                false
            }
            fn erzeuge(&mut self, _: &Modellsatz, _: &Host, e: &Erzeugung) -> Result<()> {
                motiv(e.ziel);
                Ok(())
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let p = plan(r#"{"kind":"fullbody","prompt":"girl"}"#);
        // Ein leeres Modellverzeichnis: mit Download-Versuch käme hier ein Fehler.
        let mut h = host(dir.path());
        h.modelle = dir.path().join("gibt-es-nicht");
        let mut ereignisse = Vec::new();
        ausfuehren(&p, &dir.path().join("aus"), &h, &mut Leer, &AtomicBool::new(false), &mut |e| ereignisse.push(e)).unwrap();
        assert_eq!(fertige(&ereignisse), ["girl.png"]);
    }

    #[test]
    fn referenzen_kommen_deckend_bei_der_engine_an() {
        let dir = tempfile::tempdir().unwrap();
        let ref_pfad = dir.path().join("r.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 0])).save(&ref_pfad).unwrap();
        let mut p = plan(r#"{"kind":"fullbody","prompt":"girl"}"#);
        p.aufgaben[0].refs = vec![ref_pfad.clone()];
        let mut engine = Attrappe::default();
        lauf(&p, dir.path(), &mut engine, &AtomicBool::new(false)).0.unwrap();
        assert_ne!(engine.refs[0][0], ref_pfad, "transparente Referenz wird ersetzt");
    }

    #[test]
    fn der_arbeitsordner_wird_am_ende_weggeraeumt() {
        let dir = tempfile::tempdir().unwrap();
        let p = plan(r#"{"kind":"fullbody","prompt":"girl"}"#);
        lauf(&p, dir.path(), &mut Attrappe::default(), &AtomicBool::new(false)).0.unwrap();
        assert!(!dir.path().join("aus/.arbeit").exists());
    }

    #[test]
    fn metriken_halten_vor_dem_erzeugen_kontext_mit_referenzmassen_fest() {
        use crate::metriken::{self, Metriken};
        let dir = tempfile::tempdir().unwrap();
        let ref_pfad = dir.path().join("r.png");
        image::RgbImage::from_pixel(40, 30, Rgb([1, 2, 3])).save(&ref_pfad).unwrap();
        let mut p = plan(r#"{"kind":"fullbody","prompt":"girl"}"#);
        p.aufgaben[0].refs = vec![ref_pfad];
        let pfad = dir.path().join("metrics.jsonl");
        let m = Metriken::oeffnen(&pfad).unwrap();
        ausfuehren_mit_metriken(
            &p,
            &dir.path().join("aus"),
            &host(dir.path()),
            &mut Attrappe::default(),
            &AtomicBool::new(false),
            &mut |_| {},
            Some(&m),
        )
        .unwrap();
        let kontexte: Vec<_> = metriken::eintraege_lesen(&pfad)
            .unwrap()
            .into_iter()
            .filter_map(|e| match e {
                metriken::Eintrag::Kontext { kontext, .. } => Some(kontext),
                _ => None,
            })
            .collect();
        assert_eq!(kontexte.len(), 1);
        assert_eq!(kontexte[0].preset, "klein-9b");
        assert_eq!((kontexte[0].breite, kontexte[0].hoehe), (768, 1536));
        assert_eq!(kontexte[0].referenzen, vec![(40, 30)]);
    }

    #[test]
    fn metriken_markieren_die_phasen_eines_bildes_in_reihenfolge() {
        use crate::metriken::{self, Eintrag as M, Metriken};
        let dir = tempfile::tempdir().unwrap();
        let p = plan(r#"{"kind":"token","prompt":"dwarf"}"#);
        let pfad = dir.path().join("metrics.jsonl");
        let m = Metriken::oeffnen(&pfad).unwrap();
        ausfuehren_mit_metriken(
            &p,
            &dir.path().join("aus"),
            &host(dir.path()),
            &mut Attrappe::default(),
            &AtomicBool::new(false),
            &mut |_| {},
            Some(&m),
        )
        .unwrap();
        let phasen: Vec<String> = metriken::eintraege_lesen(&pfad)
            .unwrap()
            .into_iter()
            .filter_map(|e| match e {
                M::Phase { name, .. } => Some(name),
                _ => None,
            })
            .collect();
        assert_eq!(phasen, ["referenz", "erzeugen", "freistellen", "bild_fertig"]);
    }

    #[test]
    fn die_engine_bekommt_die_metriken_nur_wenn_beobachtet_wird() {
        use crate::metriken::Metriken;
        let dir = tempfile::tempdir().unwrap();
        let p = plan(r#"{"kind":"fullbody","prompt":"girl"}"#);
        let mut ohne = Attrappe::default();
        lauf(&p, dir.path(), &mut ohne, &AtomicBool::new(false)).0.unwrap();
        assert_eq!(ohne.metriken_da, [false]);

        let m = Metriken::oeffnen(&dir.path().join("m.jsonl")).unwrap();
        let mut mit = Attrappe::default();
        ausfuehren_mit_metriken(&p, &dir.path().join("aus"), &host(dir.path()), &mut mit, &AtomicBool::new(false), &mut |_| {}, Some(&m)).unwrap();
        assert_eq!(mit.metriken_da, [true]);
    }

    fn mit_grosser_referenz(dir: &Path, plan_px: Option<u32>, host_px: Option<u32>) -> Attrappe {
        let ref_pfad = dir.join("gross.png");
        image::RgbImage::from_pixel(128, 96, Rgb([9, 9, 9])).save(&ref_pfad).unwrap();
        let mut p = plan(r#"{"kind":"fullbody","prompt":"girl"}"#);
        p.aufgaben[0].refs = vec![ref_pfad];
        p.ref_max_px = plan_px;
        let mut h = host(dir);
        h.ref_max_px = host_px;
        let mut engine = Attrappe::default();
        ausfuehren(&p, &dir.join("aus"), &h, &mut engine, &AtomicBool::new(false), &mut |_| {}).unwrap();
        engine
    }

    #[test]
    fn mit_obergrenze_bekommt_die_engine_verkleinerte_referenzen_der_auftrag_geht_vor() {
        let dir = tempfile::tempdir().unwrap();
        // Nur der Server hat eine Grenze (32 px lange Kante).
        let e = mit_grosser_referenz(dir.path(), None, Some(32));
        assert_eq!(e.ref_masse[0], [crate::referenzen::ziel_masse(128, 96, 32)]);
        assert_eq!(e.selbst_skaliert, [true]);
        // Der Auftrag (64 px) schlägt die Server-Vorgabe.
        let e = mit_grosser_referenz(dir.path(), Some(64), Some(32));
        assert_eq!(e.ref_masse[0], [crate::referenzen::ziel_masse(128, 96, 64)]);
    }

    #[test]
    fn ohne_obergrenze_bleibt_die_referenz_wie_sie_ist_und_sd_cpp_skaliert() {
        let dir = tempfile::tempdir().unwrap();
        let e = mit_grosser_referenz(dir.path(), None, None);
        assert_eq!(e.ref_masse[0], [(128, 96)]);
        assert_eq!(e.selbst_skaliert, [false]);
    }

    #[test]
    fn der_kontext_haelt_die_wirksame_obergrenze_fest() {
        use crate::metriken::{self, Metriken};
        let dir = tempfile::tempdir().unwrap();
        let ref_pfad = dir.path().join("gross.png");
        image::RgbImage::from_pixel(128, 96, Rgb([9, 9, 9])).save(&ref_pfad).unwrap();
        let mut p = plan(r#"{"kind":"fullbody","prompt":"girl"}"#);
        p.aufgaben[0].refs = vec![ref_pfad];
        let mut h = host(dir.path());
        h.ref_max_px = Some(32);
        let pfad = dir.path().join("m.jsonl");
        let m = Metriken::oeffnen(&pfad).unwrap();
        ausfuehren_mit_metriken(&p, &dir.path().join("aus"), &h, &mut Attrappe::default(), &AtomicBool::new(false), &mut |_| {}, Some(&m)).unwrap();
        let kontext = metriken::eintraege_lesen(&pfad).unwrap().into_iter().find_map(|e| match e {
            metriken::Eintrag::Kontext { kontext, .. } => Some(kontext),
            _ => None,
        });
        let kontext = kontext.unwrap();
        assert_eq!(kontext.ref_max_px, Some(32));
        assert_eq!(kontext.referenzen, [crate::referenzen::ziel_masse(128, 96, 32)], "die Maße nach dem Verkleinern");
    }
}
