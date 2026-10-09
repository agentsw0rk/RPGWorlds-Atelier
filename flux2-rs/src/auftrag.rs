//! Ein Auftrag von außen (JSON) und der daraus abgeleitete Plan.
//!
//! Der Auftrag ersetzt die Kommandozeile der vier Skripte: dieselben Stile und
//! Vorgaben, dieselbe Seed-Logik von `generate-macos.sh`, aber als Daten statt
//! als Argumentliste. `planen` ist reine Logik — es liest keine Dateien und
//! erzeugt kein Bild; Listen, Uploads und Würfel kommen über `Umgebung`.

use crate::kind::{self, Kind, STYLE_REF_HINT, TOKEN_OUT_H, TOKEN_OUT_W};
use crate::listen::{self, Eintrag};
use crate::modelle::{Preset, STANDARD_QUANT};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Obergrenze für Bilder pro Auftrag: ein Lauf dauert Minuten, ein Tippfehler
/// ("seeds": 10000) soll nicht die Warteschlange tagelang blockieren.
pub const MAX_BILDER: usize = 64;
const MAX_PROMPT_ZEICHEN: usize = 4000;
const MIN_KANTE: u32 = 256;
const MAX_KANTE: u32 = 2048;
const MAX_STEPS: u32 = 100;
/// Grenzen für `ref_max_px`: darunter ist eine Referenz nur noch Farbfleck, darüber
/// gilt ohnehin sd.cpps eigene Grenze von 1 MP.
const MIN_REF_PX: u32 = 64;
const MAX_REF_PX: u32 = 2048;

/// Welche Seeds ein Auftrag erzeugen soll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Seeds {
    /// `"seeds": 4` — vier Varianten mit fortlaufenden Seeds ab `seed`.
    Anzahl(u32),
    /// `"seeds": [7, 42, 99]` — genau diese Seeds.
    Liste(Vec<i64>),
}

/// Auftrag, wie die API ihn annimmt. Unbekannte Felder sind ein Fehler: ein
/// Tippfehler wie `"seed_count"` soll nicht stillschweigend ignoriert werden.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Auftrag {
    /// Welches Skript nachgebaut wird: token, location, portrait, fullbody.
    pub kind: Option<Kind>,
    /// Charakter- bzw. Ortsbeschreibung (englisch, fürs Modell).
    pub prompt: Option<String>,
    /// Statt eines Prompts: die Standardliste dieser Art abarbeiten.
    #[serde(default)]
    pub liste: Option<ListenAuswahl>,
    /// Dateiname-Stamm; Default wird aus dem Prompt abgeleitet.
    pub name: Option<String>,

    pub preset: Option<Preset>,
    pub quant: Option<String>,
    pub wtype: Option<String>,

    pub width: Option<u32>,
    pub height: Option<u32>,
    /// `location.sh --large`: 1536x768 statt 1024x512.
    #[serde(default)]
    pub large: bool,
    pub steps: Option<u32>,
    pub cfg: Option<f32>,
    pub guidance: Option<f32>,

    /// Startseed (Default 42). `-1` würfelt einmal; der Seed steht danach im Ergebnis.
    pub seed: Option<i64>,
    pub seeds: Option<Seeds>,

    /// Stiltext ersetzen — nur für Experimente, sonst bricht die Reihe.
    pub style: Option<String>,
    /// Pose ersetzen (nur Tokens).
    pub pose: Option<String>,
    /// Upload-ID eines Bildes, das nur Stil liefern soll (`--style-ref`).
    pub style_ref: Option<String>,
    /// Upload-IDs von Referenzbildern, die inhaltlich übernommen werden.
    #[serde(default)]
    pub refs: Vec<String>,
    /// Obergrenze für die lange Kante der Referenzbilder in Pixeln. Weniger
    /// Referenz-Pixel heißen weniger Tokens und damit deutlich schnelleres Sampling.
    /// Ohne Angabe gilt die Vorgabe des Servers (`REF_MAX_PX`), sonst sd.cpps 1 MP.
    pub ref_max_px: Option<u32>,

    /// Freistellen erzwingen oder abschalten. Default: nur Tokens werden freigestellt.
    pub freistellen: Option<bool>,
    /// Farb-Keying statt u2netp (Default bei Tokens: ja).
    pub key: Option<bool>,
    pub out_w: Option<u32>,
    pub out_h: Option<u32>,
}

/// `{"liste": {"from": "...", "only": "...", "force": true}}` — wie die
/// Batch-Skripte. Leer (`{}`) heißt: alles, was noch fehlt.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListenAuswahl {
    pub from: Option<String>,
    pub only: Option<String>,
    /// Auch vorhandene Bilder neu erzeugen.
    #[serde(default)]
    pub force: bool,
}

/// Was `planen` von außen braucht, damit es selbst reine Logik bleibt.
pub trait Umgebung {
    /// Die Standardliste dieser Art.
    fn liste(&self, kind: Kind) -> Result<Vec<Eintrag>>;
    /// Pfad eines hochgeladenen Bildes; Fehler, wenn es die ID nicht gibt.
    fn upload(&self, id: &str) -> Result<PathBuf>;
    /// Ein zufälliger Startseed.
    fn wuerfeln(&mut self) -> i64;
    /// Ob unter diesem Pfad schon ein fertiges Bild liegt (Batch-Fortsetzen).
    fn vorhanden(&self, datei: &str) -> bool;
}

/// Wie freigestellt wird.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Freistellen {
    /// `true`: Farb-Keying (Hintergrundfarbe vom Rand), `false`: u2netp.
    pub keying: bool,
    pub cutoff: u8,
    /// Auf das Motiv zuschneiden und in diese Fläche einpassen.
    pub einpassen: Option<(u32, u32)>,
}

/// Ein einzelnes Bild.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Aufgabe {
    /// Dateiname ohne Endung, bei mehreren Seeds mit `-s<seed>`.
    pub stamm: String,
    /// Der Listen-slug, falls das Bild aus einer Liste stammt.
    pub eintrag: Option<String>,
    pub seed: i64,
    pub prompt: String,
    pub refs: Vec<PathBuf>,
    pub freistellen: Option<Freistellen>,
    /// Schon vorhanden und nicht neu zu erzeugen (nur Batch ohne `force`).
    pub uebersprungen: bool,
}

/// Alles, was für den ganzen Auftrag gleich ist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub kind: Kind,
    pub preset: Preset,
    pub quant: String,
    pub wtype: Option<String>,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub cfg: f32,
    pub guidance: f32,
    /// Siehe `Auftrag::ref_max_px`. Fehlt in älteren `job.json`.
    #[serde(default)]
    pub ref_max_px: Option<u32>,
    /// `true`: die Bilder gehören in den dauerhaften Ordner der Art (Batch),
    /// `false`: in den Ordner dieses Auftrags.
    pub dauerhaft: bool,
    pub aufgaben: Vec<Aufgabe>,
}

fn pruefe_kante(name: &str, wert: u32) -> Result<()> {
    if wert % 16 != 0 {
        bail!("{name}={wert} ist nicht durch 16 teilbar — das Modell verlangt das.");
    }
    if !(MIN_KANTE..=MAX_KANTE).contains(&wert) {
        bail!("{name}={wert} liegt außerhalb von {MIN_KANTE}..{MAX_KANTE}.");
    }
    Ok(())
}

/// Ein Dateiname-Stamm darf nirgends hinzeigen außer in den Ausgabeordner.
fn pruefe_stamm(stamm: &str) -> Result<()> {
    let gueltig = !stamm.is_empty()
        && stamm.len() <= 100
        && !stamm.starts_with('.')
        && stamm.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !gueltig {
        bail!("name '{stamm}' ist ungültig — erlaubt sind Buchstaben, Ziffern, - _ . (nicht am Anfang).");
    }
    Ok(())
}

fn seeds_festlegen(auftrag: &Auftrag, umgebung: &mut dyn Umgebung) -> Result<Vec<i64>> {
    let start = auftrag.seed.unwrap_or(42);
    match &auftrag.seeds {
        Some(Seeds::Liste(liste)) => {
            if auftrag.seed.is_some() {
                bail!("seed und eine seeds-Liste schließen sich aus — entweder Startseed oder feste Seeds.");
            }
            if liste.is_empty() {
                bail!("seeds braucht mindestens einen Eintrag.");
            }
            if liste.iter().any(|&s| s < 0) {
                bail!("In einer seeds-Liste sind nur Seeds ab 0 erlaubt; -1 geht nur als Startseed.");
            }
            Ok(liste.clone())
        }
        anzahl => {
            let n = match anzahl {
                Some(Seeds::Anzahl(n)) => *n as usize,
                _ => 1,
            };
            if n < 1 {
                bail!("seeds braucht mindestens 1.");
            }
            // -1 heißt "würfle". Wir würfeln selbst und zählen von dort hoch,
            // damit jeder Treffer reproduzierbar ist und im Ergebnis steht.
            let start = if start < 0 { umgebung.wuerfeln().abs() } else { start };
            Ok((0..n as i64).map(|i| start + i).collect())
        }
    }
}

/// Leitet aus einem Auftrag den Plan ab: Prompt je Eintrag, Seeds, Dateinamen,
/// Größe und Freistellen. Fehler gibt es hier, vor dem ersten Bild — nicht
/// nach einer halben Stunde Rechnen.
pub fn planen(auftrag: &Auftrag, umgebung: &mut dyn Umgebung) -> Result<Plan> {
    let Some(kind) = auftrag.kind else {
        bail!("kind fehlt — erwartet: token, location, portrait oder fullbody.");
    };
    if auftrag.prompt.is_some() == auftrag.liste.is_some() {
        bail!("Entweder prompt oder liste angeben — genau eins von beiden.");
    }

    let preset = auftrag.preset.unwrap_or_default();
    let satz_standard = crate::modelle::aufloesen(std::path::Path::new("."), preset, STANDARD_QUANT, None)?;
    let vorgaben = kind.vorgaben(auftrag.large);

    let width = auftrag.width.unwrap_or(vorgaben.width);
    let height = auftrag.height.unwrap_or(vorgaben.height);
    pruefe_kante("width", width)?;
    pruefe_kante("height", height)?;

    // Die Skripte übergeben ihre Schrittzahl immer ausdrücklich; hier gilt für
    // den nicht distillierten Satz stattdessen dessen eigener Wert, sonst käme
    // bei base mit 8 statt 20 Steps Matsch heraus.
    let steps = auftrag.steps.unwrap_or(if preset == Preset::KleinBase9b {
        satz_standard.standard_steps
    } else {
        vorgaben.steps
    });
    if !(1..=MAX_STEPS).contains(&steps) {
        bail!("steps={steps} liegt außerhalb von 1..{MAX_STEPS}.");
    }
    let cfg = auftrag.cfg.unwrap_or(satz_standard.standard_cfg);
    let guidance = auftrag.guidance.unwrap_or(3.5);
    if !(0.0..=20.0).contains(&cfg) || !(0.0..=20.0).contains(&guidance) {
        bail!("cfg und guidance müssen zwischen 0 und 20 liegen.");
    }

    if let Some(px) = auftrag.ref_max_px {
        if !(MIN_REF_PX..=MAX_REF_PX).contains(&px) {
            bail!("ref_max_px={px} liegt außerhalb von {MIN_REF_PX}..{MAX_REF_PX}.");
        }
    }

    // Freistellen: Default wie die Skripte — nur Tokens. location/portrait/
    // fullbody behalten ihren Hintergrund.
    let freistellen_an = auftrag.freistellen.unwrap_or(!vorgaben.keep_bg);
    if !freistellen_an && (auftrag.out_w.is_some() || auftrag.out_h.is_some() || auftrag.key == Some(true)) {
        bail!("out_w/out_h und key wirken nur beim Freistellen, nicht ohne.");
    }
    let einpassen = match (auftrag.out_w, auftrag.out_h) {
        (None, None) if kind == Kind::Token && freistellen_an => Some((TOKEN_OUT_W, TOKEN_OUT_H)),
        (None, None) => None,
        (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
        _ => bail!("out_w und out_h nur gemeinsam und größer als 0 angeben."),
    };
    let freistellen = freistellen_an.then(|| Freistellen {
        // Ein Token hat einen festen mittelgrauen Grund — Keying behält den
        // Sockel, den u2netp als Beiwerk wegschneiden würde.
        keying: auftrag.key.unwrap_or(kind == Kind::Token),
        cutoff: 12,
        einpassen,
    });

    // Referenzen: eine Stilvorlage bekommt die Anweisung vor den Prompt, plain
    // `refs` werden inhaltlich übernommen.
    let mut refs = Vec::new();
    if let Some(id) = &auftrag.style_ref {
        refs.push(umgebung.upload(id)?);
    }
    for id in &auftrag.refs {
        refs.push(umgebung.upload(id)?);
    }

    let seeds = seeds_festlegen(auftrag, umgebung)?;

    // Die Einträge: ein Prompt oder die Liste.
    let (eintraege, dauerhaft, force): (Vec<(Option<String>, String)>, bool, bool) =
        match (&auftrag.prompt, &auftrag.liste) {
            (Some(prompt), None) => {
                let prompt = prompt.trim();
                if prompt.is_empty() {
                    bail!("prompt ist leer.");
                }
                if prompt.chars().count() > MAX_PROMPT_ZEICHEN {
                    bail!("prompt ist länger als {MAX_PROMPT_ZEICHEN} Zeichen.");
                }
                (vec![(None, prompt.to_string())], false, false)
            }
            (None, Some(auswahl)) => {
                if auftrag.name.is_some() {
                    bail!("name gilt für einen einzelnen Prompt; Listeneinträge heißen wie ihr slug.");
                }
                let liste = listen::eingrenzen(
                    umgebung.liste(kind)?,
                    auswahl.from.as_deref(),
                    auswahl.only.as_deref(),
                )?;
                (
                    liste.into_iter().map(|e| (Some(e.slug), e.prompt)).collect(),
                    true,
                    auswahl.force,
                )
            }
            _ => unreachable!("oben geprüft: genau eins von beiden"),
        };

    let mut aufgaben = Vec::new();
    for (slug, beschreibung) in eintraege {
        let stamm_basis = match (&slug, &auftrag.name) {
            (Some(slug), _) => slug.clone(),
            (None, Some(name)) => {
                pruefe_stamm(name)?;
                name.clone()
            }
            (None, None) => kind::slug(&beschreibung).ok_or_else(|| {
                anyhow::anyhow!("Aus dem Prompt lässt sich kein Dateiname ableiten — bitte name angeben.")
            })?,
        };
        // Mit Szenenreferenz (`refs`) sagt der Prompt dem Modell ausdrücklich, was mit
        // dem Bild geschehen soll, und der Stil verzichtet auf Umgebungsangaben.
        let mut prompt = if auftrag.refs.is_empty() {
            kind.prompt(&beschreibung, auftrag.style.as_deref(), auftrag.pose.as_deref())
        } else {
            kind.prompt_mit_szene(&beschreibung, auftrag.style.as_deref(), auftrag.pose.as_deref())
        };
        if auftrag.style_ref.is_some() {
            prompt = format!("{STYLE_REF_HINT} {prompt}");
        }
        for &seed in &seeds {
            // Bei mehreren Seeds trägt jede Datei ihren Seed im Namen — sonst
            // überschreiben sich die Läufe gegenseitig.
            let stamm = if seeds.len() > 1 { format!("{stamm_basis}-s{seed}") } else { stamm_basis.clone() };
            let uebersprungen = dauerhaft && !force && umgebung.vorhanden(&format!("{stamm}.png"));
            aufgaben.push(Aufgabe {
                stamm,
                eintrag: slug.clone(),
                seed,
                prompt: prompt.clone(),
                refs: refs.clone(),
                freistellen: freistellen.clone(),
                uebersprungen,
            });
        }
    }
    if aufgaben.is_empty() {
        bail!("Der Auftrag ergibt kein einziges Bild.");
    }
    if aufgaben.len() > MAX_BILDER {
        bail!("Der Auftrag ergibt {} Bilder — erlaubt sind höchstens {MAX_BILDER} pro Auftrag.", aufgaben.len());
    }

    Ok(Plan {
        kind,
        preset,
        quant: auftrag.quant.clone().unwrap_or_else(|| STANDARD_QUANT.into()),
        wtype: auftrag.wtype.clone().or_else(|| satz_standard.standard_wtype.map(String::from)),
        width,
        height,
        steps,
        cfg,
        guidance,
        ref_max_px: auftrag.ref_max_px,
        dauerhaft,
        aufgaben,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::FULLBODY_STYLE;

    #[derive(Default)]
    struct Test {
        liste: Vec<Eintrag>,
        uploads: Vec<&'static str>,
        vorhanden: Vec<String>,
        wuerfel: i64,
    }

    impl Umgebung for Test {
        fn liste(&self, _: Kind) -> Result<Vec<Eintrag>> {
            Ok(self.liste.clone())
        }
        fn upload(&self, id: &str) -> Result<PathBuf> {
            if self.uploads.contains(&id) {
                Ok(PathBuf::from(format!("/up/{id}.png")))
            } else {
                bail!("Upload '{id}' gibt es nicht.")
            }
        }
        fn wuerfeln(&mut self) -> i64 {
            self.wuerfel
        }
        fn vorhanden(&self, datei: &str) -> bool {
            self.vorhanden.iter().any(|v| v == datei)
        }
    }

    fn eintrag(slug: &str, prompt: &str) -> Eintrag {
        Eintrag { slug: slug.into(), name: slug.into(), beschreibung: String::new(), prompt: prompt.into() }
    }

    fn auftrag(json: &str) -> Auftrag {
        serde_json::from_str(json).unwrap_or_else(|e| panic!("{e}: {json}"))
    }

    fn plan(json: &str) -> Plan {
        planen(&auftrag(json), &mut Test::default()).unwrap()
    }

    fn fehler(json: &str) -> String {
        planen(&auftrag(json), &mut Test::default()).unwrap_err().to_string()
    }

    #[test]
    fn einfacher_fullbody_auftrag_nutzt_die_skriptvorgaben() {
        let p = plan(r#"{"kind":"fullbody","prompt":"orphan girl, torn dress"}"#);
        assert_eq!((p.width, p.height, p.steps), (768, 1536, 8));
        assert_eq!(p.preset, Preset::Klein9b);
        assert_eq!(p.aufgaben.len(), 1);
        let a = &p.aufgaben[0];
        assert_eq!(a.stamm, "orphan-girl");
        assert_eq!(a.seed, 42);
        assert_eq!(a.prompt, format!("orphan girl, torn dress, {FULLBODY_STYLE}"));
        assert_eq!(a.freistellen, None, "fullbody behält den Hintergrund");
        assert!(!p.dauerhaft);
    }

    #[test]
    fn token_wird_per_keying_freigestellt_und_auf_token_groesse_eingepasst() {
        let p = plan(r#"{"kind":"token","prompt":"dwarf cleric"}"#);
        let f = p.aufgaben[0].freistellen.as_ref().unwrap();
        assert!(f.keying);
        assert_eq!(f.einpassen, Some((138, 244)));
        assert!(p.aufgaben[0].prompt.contains("calm upright pose"));
    }

    #[test]
    fn freistellen_laesst_sich_bei_tokens_abschalten() {
        let p = plan(r#"{"kind":"token","prompt":"dwarf","freistellen":false}"#);
        assert_eq!(p.aufgaben[0].freistellen, None);
    }

    #[test]
    fn freistellen_laesst_sich_bei_porträts_einschalten_und_nutzt_dann_saliency() {
        let p = plan(r#"{"kind":"portrait","prompt":"elf","freistellen":true}"#);
        let f = p.aufgaben[0].freistellen.as_ref().unwrap();
        assert!(!f.keying);
        assert_eq!(f.einpassen, None);
    }

    #[test]
    fn mehrere_seeds_zaehlen_ab_dem_startseed_hoch_und_stehen_im_dateinamen() {
        let p = plan(r#"{"kind":"fullbody","prompt":"girl","seed":100,"seeds":3}"#);
        let seeds: Vec<_> = p.aufgaben.iter().map(|a| a.seed).collect();
        assert_eq!(seeds, [100, 101, 102]);
        assert_eq!(p.aufgaben[1].stamm, "girl-s101");
    }

    #[test]
    fn eine_seedliste_nimmt_genau_diese_seeds() {
        let p = plan(r#"{"kind":"fullbody","prompt":"girl","seeds":[7,42,99]}"#);
        let seeds: Vec<_> = p.aufgaben.iter().map(|a| a.seed).collect();
        assert_eq!(seeds, [7, 42, 99]);
    }

    #[test]
    fn seed_minus_eins_wuerfelt_einmal_und_zaehlt_hoch() {
        let mut umgebung = Test { wuerfel: 5000, ..Default::default() };
        let p = planen(&auftrag(r#"{"kind":"fullbody","prompt":"girl","seed":-1,"seeds":2}"#), &mut umgebung)
            .unwrap();
        let seeds: Vec<_> = p.aufgaben.iter().map(|a| a.seed).collect();
        assert_eq!(seeds, [5000, 5001]);
    }

    #[test]
    fn auch_ein_einzelbild_mit_seed_minus_eins_bekommt_einen_konkreten_seed() {
        let mut umgebung = Test { wuerfel: 777, ..Default::default() };
        let p = planen(&auftrag(r#"{"kind":"fullbody","prompt":"girl","seed":-1}"#), &mut umgebung).unwrap();
        assert_eq!(p.aufgaben[0].seed, 777, "der Seed muss im Ergebnis stehen, sonst ist er nicht reproduzierbar");
        assert_eq!(p.aufgaben[0].stamm, "girl", "ein einzelnes Bild trägt keinen Seed im Namen");
    }

    #[test]
    fn seed_und_seedliste_zusammen_sind_ein_fehler() {
        assert!(fehler(r#"{"kind":"fullbody","prompt":"x","seed":1,"seeds":[1,2]}"#).contains("schließen sich aus"));
    }

    #[test]
    fn negative_seeds_in_der_liste_sind_ein_fehler() {
        assert!(fehler(r#"{"kind":"fullbody","prompt":"x","seeds":[-1]}"#).contains("ab 0"));
    }

    #[test]
    fn zu_viele_bilder_werden_abgelehnt() {
        assert!(fehler(r#"{"kind":"fullbody","prompt":"x","seeds":65}"#).contains("höchstens 64"));
        assert!(fehler(r#"{"kind":"fullbody","prompt":"x","seeds":0}"#).contains("mindestens 1"));
    }

    #[test]
    fn unbekannte_felder_sind_ein_fehler() {
        let ergebnis = serde_json::from_str::<Auftrag>(r#"{"kind":"fullbody","prompt":"x","seed_count":3}"#);
        assert!(ergebnis.unwrap_err().to_string().contains("seed_count"));
    }

    #[test]
    fn prompt_und_liste_zusammen_oder_keins_von_beiden_ist_ein_fehler() {
        assert!(fehler(r#"{"kind":"fullbody"}"#).contains("genau eins"));
        assert!(fehler(r#"{"kind":"fullbody","prompt":"x","liste":{}}"#).contains("genau eins"));
        assert!(fehler(r#"{"prompt":"x"}"#).contains("kind fehlt"));
    }

    #[test]
    fn leerer_oder_endloser_prompt_wird_abgelehnt() {
        assert!(fehler(r#"{"kind":"fullbody","prompt":"   "}"#).contains("leer"));
        let lang = "a".repeat(5000);
        assert!(fehler(&format!(r#"{{"kind":"fullbody","prompt":"{lang}"}}"#)).contains("länger"));
    }

    #[test]
    fn kanten_muessen_durch_16_teilbar_und_im_bereich_sein() {
        assert!(fehler(r#"{"kind":"fullbody","prompt":"x","width":300}"#).contains("durch 16"));
        assert!(fehler(r#"{"kind":"fullbody","prompt":"x","height":8192}"#).contains("außerhalb"));
        let p = plan(r#"{"kind":"fullbody","prompt":"x","width":1024,"height":2048}"#);
        assert_eq!((p.width, p.height), (1024, 2048));
    }

    #[test]
    fn large_schaltet_orte_auf_die_groessere_kante() {
        let p = plan(r#"{"kind":"location","prompt":"village","large":true}"#);
        assert_eq!((p.width, p.height), (1536, 768));
    }

    #[test]
    fn preset_default_ist_klein_9b_und_base_bringt_eigene_steps_cfg_und_wtype_mit() {
        let p = plan(r#"{"kind":"fullbody","prompt":"x","preset":"klein-base-9b"}"#);
        assert_eq!((p.steps, p.cfg), (20, 4.0));
        assert_eq!(p.wtype.as_deref(), Some("q8_0"));
        let ausdruecklich = plan(r#"{"kind":"fullbody","prompt":"x","preset":"klein-base-9b","steps":12,"cfg":3.0}"#);
        assert_eq!((ausdruecklich.steps, ausdruecklich.cfg), (12, 3.0));
    }

    #[test]
    fn style_ref_stellt_die_anweisung_voran_und_reicht_das_bild_als_referenz_durch() {
        let mut umgebung = Test { uploads: vec!["abc123"], ..Default::default() };
        let p = planen(
            &auftrag(r#"{"kind":"fullbody","prompt":"dwarf smith","style_ref":"abc123"}"#),
            &mut umgebung,
        )
        .unwrap();
        let a = &p.aufgaben[0];
        assert!(a.prompt.starts_with(STYLE_REF_HINT), "{}", a.prompt);
        assert!(a.prompt.contains("dwarf smith"));
        assert_eq!(a.refs, vec![PathBuf::from("/up/abc123.png")]);
    }

    #[test]
    fn plain_refs_bekommen_keine_stilanweisung() {
        let mut umgebung = Test { uploads: vec!["r1"], ..Default::default() };
        let p = planen(&auftrag(r#"{"kind":"token","prompt":"x","refs":["r1"]}"#), &mut umgebung).unwrap();
        assert!(!p.aufgaben[0].prompt.contains(STYLE_REF_HINT));
        assert_eq!(p.aufgaben[0].refs.len(), 1);
    }

    #[test]
    fn szenenreferenz_bekommt_die_anweisung_und_bei_ganzkoerper_keinen_umgebungsstil() {
        let mut umgebung = Test { uploads: vec!["r1"], ..Default::default() };
        let p = planen(&auftrag(r#"{"kind":"fullbody","prompt":"elven girl","refs":["r1"]}"#), &mut umgebung).unwrap();
        let prompt = &p.aufgaben[0].prompt;
        assert!(prompt.starts_with(crate::kind::REF_SZENE_HINT), "{prompt}");
        assert!(prompt.contains("elven girl, full-body fantasy character illustration"));
        assert!(!prompt.contains("softly blurred background"), "der Stil darf der Referenz nicht widersprechen: {prompt}");
        assert_eq!(p.aufgaben[0].refs.len(), 1);
    }

    #[test]
    fn stilreferenz_allein_bekommt_den_normalen_stil_ohne_szenenanweisung() {
        let mut umgebung = Test { uploads: vec!["r1"], ..Default::default() };
        let p = planen(&auftrag(r#"{"kind":"fullbody","prompt":"girl","style_ref":"r1"}"#), &mut umgebung).unwrap();
        assert!(!p.aufgaben[0].prompt.contains(crate::kind::REF_SZENE_HINT));
        assert!(p.aufgaben[0].prompt.contains("softly blurred background"), "Stilvorlage behält den vollen Stil");
    }

    #[test]
    fn stil_und_szenenreferenz_zusammen_tragen_beide_anweisungen() {
        let mut umgebung = Test { uploads: vec!["s", "r"], ..Default::default() };
        let p = planen(&auftrag(r#"{"kind":"fullbody","prompt":"girl","style_ref":"s","refs":["r"]}"#), &mut umgebung).unwrap();
        let prompt = &p.aufgaben[0].prompt;
        assert!(prompt.starts_with(STYLE_REF_HINT) && prompt.contains(crate::kind::REF_SZENE_HINT), "{prompt}");
        assert_eq!(p.aufgaben[0].refs.len(), 2, "erst Stil-, dann Szenenreferenz");
    }

    #[test]
    fn unbekannter_upload_ist_ein_fehler() {
        assert!(fehler(r#"{"kind":"fullbody","prompt":"x","style_ref":"nix"}"#).contains("nix"));
    }

    #[test]
    fn name_wird_als_dateiname_geprueft() {
        assert_eq!(plan(r#"{"kind":"fullbody","prompt":"x","name":"mein-bild_1"}"#).aufgaben[0].stamm, "mein-bild_1");
        for boese in ["../x", "a/b", ".versteckt", "", "a b"] {
            let json = serde_json::json!({"kind":"fullbody","prompt":"x","name":boese}).to_string();
            assert!(fehler(&json).contains("ungültig"), "{boese:?} muss abgelehnt werden");
        }
    }

    #[test]
    fn prompt_ohne_ableitbaren_dateinamen_verlangt_einen_namen() {
        assert!(fehler(r#"{"kind":"fullbody","prompt":"!!!"}"#).contains("name angeben"));
    }

    #[test]
    fn freistellgroesse_nur_mit_freistellen_und_nur_paarweise() {
        assert!(fehler(r#"{"kind":"fullbody","prompt":"x","out_w":100,"out_h":100}"#).contains("nur beim Freistellen"));
        assert!(fehler(r#"{"kind":"token","prompt":"x","out_w":100}"#).contains("gemeinsam"));
        let p = plan(r#"{"kind":"token","prompt":"x","out_w":200,"out_h":300}"#);
        assert_eq!(p.aufgaben[0].freistellen.as_ref().unwrap().einpassen, Some((200, 300)));
    }

    #[test]
    fn liste_ergibt_ein_bild_je_eintrag_mit_dem_slug_als_dateiname() {
        let mut umgebung = Test {
            liste: vec![eintrag("mira", "orphan girl"), eintrag("jonas", "orphan boy")],
            ..Default::default()
        };
        let p = planen(&auftrag(r#"{"kind":"fullbody","liste":{}}"#), &mut umgebung).unwrap();
        let stamme: Vec<_> = p.aufgaben.iter().map(|a| a.stamm.as_str()).collect();
        assert_eq!(stamme, ["mira", "jonas"]);
        assert!(p.dauerhaft);
        assert_eq!(p.aufgaben[0].eintrag.as_deref(), Some("mira"));
        assert!(p.aufgaben[1].prompt.starts_with("orphan boy, "));
    }

    #[test]
    fn liste_mit_mehreren_seeds_erzeugt_varianten_je_eintrag() {
        let mut umgebung = Test { liste: vec![eintrag("mira", "girl")], ..Default::default() };
        let p = planen(&auftrag(r#"{"kind":"fullbody","liste":{},"seeds":2}"#), &mut umgebung).unwrap();
        let stamme: Vec<_> = p.aufgaben.iter().map(|a| a.stamm.as_str()).collect();
        assert_eq!(stamme, ["mira-s42", "mira-s43"]);
    }

    #[test]
    fn liste_ueberspringt_vorhandene_bilder_ausser_mit_force() {
        let liste = vec![eintrag("mira", "girl"), eintrag("jonas", "boy")];
        let mut umgebung = Test { liste: liste.clone(), vorhanden: vec!["mira.png".into()], ..Default::default() };
        let p = planen(&auftrag(r#"{"kind":"fullbody","liste":{}}"#), &mut umgebung).unwrap();
        assert!(p.aufgaben[0].uebersprungen);
        assert!(!p.aufgaben[1].uebersprungen);

        let mut umgebung = Test { liste, vorhanden: vec!["mira.png".into()], ..Default::default() };
        let p = planen(&auftrag(r#"{"kind":"fullbody","liste":{"force":true}}"#), &mut umgebung).unwrap();
        assert!(p.aufgaben.iter().all(|a| !a.uebersprungen));
    }

    #[test]
    fn liste_laesst_sich_mit_from_und_only_eingrenzen() {
        let liste = vec![eintrag("a", "x"), eintrag("b", "y"), eintrag("c", "z")];
        let mut umgebung = Test { liste, ..Default::default() };
        let p = planen(&auftrag(r#"{"kind":"fullbody","liste":{"from":"b"}}"#), &mut umgebung).unwrap();
        assert_eq!(p.aufgaben.len(), 2);
        let p = planen(&auftrag(r#"{"kind":"fullbody","liste":{"only":"c"}}"#), &mut umgebung).unwrap();
        assert_eq!(p.aufgaben[0].stamm, "c");
        assert!(planen(&auftrag(r#"{"kind":"fullbody","liste":{"only":"q"}}"#), &mut umgebung).is_err());
    }

    #[test]
    fn name_bei_einer_liste_ist_ein_fehler() {
        let mut umgebung = Test { liste: vec![eintrag("a", "x")], ..Default::default() };
        let ergebnis = planen(&auftrag(r#"{"kind":"fullbody","liste":{},"name":"x"}"#), &mut umgebung);
        assert!(ergebnis.unwrap_err().to_string().contains("slug"));
    }

    #[test]
    fn eine_leere_oder_zu_grosse_liste_wird_abgelehnt() {
        let mut leer = Test::default();
        assert!(planen(&auftrag(r#"{"kind":"fullbody","liste":{}}"#), &mut leer).is_err());
        let gross = (0..70).map(|i| eintrag(&format!("e{i}"), "x")).collect();
        let mut gross = Test { liste: gross, ..Default::default() };
        let fehler = planen(&auftrag(r#"{"kind":"fullbody","liste":{}}"#), &mut gross).unwrap_err().to_string();
        assert!(fehler.contains("höchstens 64"), "{fehler}");
    }

    #[test]
    fn ref_max_px_kommt_in_den_plan_und_unsinnige_werte_nennen_das_feld() {
        assert_eq!(plan(r#"{"kind":"fullbody","prompt":"x","ref_max_px":512}"#).ref_max_px, Some(512));
        assert_eq!(plan(r#"{"kind":"fullbody","prompt":"x"}"#).ref_max_px, None);
        for unsinn in ["8", "5000", "0"] {
            let f = fehler(&format!(r#"{{"kind":"fullbody","prompt":"x","ref_max_px":{unsinn}}}"#));
            assert!(f.contains("ref_max_px"), "{unsinn}: {f}");
        }
    }
}
