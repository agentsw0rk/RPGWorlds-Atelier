//! Modellsätze (`--preset` in `generate-macos.sh`) und ihre Dateien.
//!
//! Ein Satz legt Diffusionsmodell **und** Text-Encoder zusammen fest: die
//! 9B-Checkpoints bringen einen rund 8B großen Encoder mit (Qwen3-8B), der 4B
//! einen 4B großen. Ein 9B-Modell mit dem 4B-Encoder ist eine stille
//! Fehlkombination.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const HF: &str = "https://huggingface.co";
const U2NETP_URL: &str = "https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2netp.onnx";
const VAE_URL_PFAD: &str = "unsloth/FLUX.2-VAE/resolve/main/split_files/vae/flux2-vae.safetensors";

/// Die drei Modellsätze.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Preset {
    /// Distilliert, GGUF, Qwen3-4B. Kleiner und schneller.
    #[serde(rename = "klein-4b")]
    Klein4b,
    /// Distilliert, GGUF, Qwen3-8B. Der Default.
    #[serde(rename = "klein-9b")]
    Klein9b,
    /// Nicht distilliert, fp8-Safetensors, Qwen3-8B. Repo gated, braucht `HF_TOKEN`.
    #[serde(rename = "klein-base-9b")]
    KleinBase9b,
}

impl Default for Preset {
    fn default() -> Self {
        Preset::Klein9b
    }
}

impl Preset {
    pub fn name(self) -> &'static str {
        match self {
            Preset::Klein4b => "klein-4b",
            Preset::Klein9b => "klein-9b",
            Preset::KleinBase9b => "klein-base-9b",
        }
    }
}

impl std::str::FromStr for Preset {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "klein-4b" => Ok(Preset::Klein4b),
            "klein-9b" => Ok(Preset::Klein9b),
            "klein-base-9b" => Ok(Preset::KleinBase9b),
            andere => Err(format!(
                "Unbekannter Modellsatz: '{andere}'. Gültig: klein-4b, klein-9b, klein-base-9b"
            )),
        }
    }
}

/// Eine Modelldatei: wo sie liegt und woher sie kommt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Datei {
    pub pfad: PathBuf,
    /// `None` bei einer selbst mitgebrachten Datei (z. B. eigener Encoder).
    pub url: Option<String>,
    pub label: String,
}

/// Alles, was ein Lauf an Dateien und Voreinstellungen vom Modellsatz braucht.
#[derive(Debug, Clone, PartialEq)]
pub struct Modellsatz {
    pub preset: Preset,
    pub dit: Datei,
    pub llm: Datei,
    pub vae: Datei,
    pub u2netp: Datei,
    /// Schrittzahl, die der Satz ohne Vorgabe braucht (distilliert 4, base 20).
    pub standard_steps: u32,
    pub standard_cfg: f32,
    /// Gewichte beim Laden umwandeln. Bei base ist q8_0 gesetzt: sd.cpp rechnet
    /// fp8 beim Einlesen auf f16 hoch, aus 9,5 GB würden rund 19 GB im Speicher.
    pub standard_wtype: Option<&'static str>,
}

/// Standard-Quantisierung der GGUF-Dateien.
pub const STANDARD_QUANT: &str = "Q5_K_M";

/// Modellverzeichnis wie `standard_modelle` in `umgebung.sh`: `<projekt>/models`
/// neben dem Repo, wenn es existiert, sonst `<repo>/models`.
///
/// Alle Einstiegspunkte müssen sich hier einig sein — sonst landen dieselben
/// mehrere GB je nach Aufrufer in zwei Verzeichnissen.
pub fn standard_verzeichnis(repo: &Path) -> PathBuf {
    let nachbar = repo.join("..").join("models");
    match nachbar.canonicalize() {
        Ok(pfad) if pfad.is_dir() => pfad,
        _ => repo.join("models"),
    }
}

/// Leitet Dateien und Voreinstellungen eines Modellsatzes ab.
///
/// `llm_datei` ersetzt den zum Preset gehörenden Encoder — für den Fall, dass
/// sd.cpp im Log `Version: Flux.2` statt `Flux.2 klein` meldet und damit
/// Mistral Small 3.2 erwartet.
pub fn aufloesen(
    modelle: &Path,
    preset: Preset,
    quant: &str,
    llm_datei: Option<&Path>,
) -> Result<Modellsatz> {
    if quant.is_empty() || !quant.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!("Ungültige Quantisierung '{quant}' — erwartet etwas wie Q5_K_M.");
    }
    let diffusion = modelle.join("diffusion");
    let (dit, llm_name, llm_repo, steps, cfg, wtype) = match preset {
        Preset::Klein4b => (
            Datei {
                pfad: diffusion.join(format!("flux-2-klein-4b-{quant}.gguf")),
                url: Some(format!(
                    "{HF}/unsloth/FLUX.2-klein-4B-GGUF/resolve/main/flux-2-klein-4b-{quant}.gguf"
                )),
                label: format!("Diffusion-Transformer klein 4B ({quant})"),
            },
            "Qwen3-4B",
            "unsloth/Qwen3-4B-GGUF",
            4,
            1.0,
            None,
        ),
        Preset::Klein9b => (
            Datei {
                pfad: diffusion.join(format!("flux-2-klein-9b-{quant}.gguf")),
                url: Some(format!(
                    "{HF}/unsloth/FLUX.2-klein-9B-GGUF/resolve/main/flux-2-klein-9b-{quant}.gguf"
                )),
                label: format!("Diffusion-Transformer klein 9B ({quant})"),
            },
            "Qwen3-8B",
            "unsloth/Qwen3-8B-GGUF",
            4,
            1.0,
            None,
        ),
        Preset::KleinBase9b => (
            Datei {
                pfad: diffusion.join("flux-2-klein-base-9b-fp8.safetensors"),
                url: Some(format!(
                    "{HF}/black-forest-labs/FLUX.2-klein-base-9b-fp8/resolve/main/flux-2-klein-base-9b-fp8.safetensors"
                )),
                label: "Diffusion-Transformer klein-base 9B (fp8, 9,5 GB)".into(),
            },
            "Qwen3-8B",
            "unsloth/Qwen3-8B-GGUF",
            20,
            4.0,
            Some("q8_0"),
        ),
    };

    let llm = match llm_datei {
        Some(pfad) => {
            if !pfad.is_file() {
                bail!("Encoder-Datei nicht gefunden: {}", pfad.display());
            }
            Datei { pfad: pfad.to_path_buf(), url: None, label: "Text-Encoder (eigene Datei)".into() }
        }
        None => Datei {
            pfad: modelle.join("text_encoder").join(format!("{llm_name}-{quant}.gguf")),
            url: Some(format!("{HF}/{llm_repo}/resolve/main/{llm_name}-{quant}.gguf")),
            label: format!("Text-Encoder {llm_name} ({quant})"),
        },
    };

    Ok(Modellsatz {
        preset,
        dit,
        llm,
        vae: Datei {
            pfad: modelle.join("vae").join("flux2-vae.safetensors"),
            url: Some(format!("{HF}/{VAE_URL_PFAD}")),
            label: "VAE".into(),
        },
        u2netp: Datei {
            pfad: modelle.join("matting").join("u2netp.onnx"),
            url: Some(U2NETP_URL.into()),
            label: "u2netp (Freistellen)".into(),
        },
        standard_steps: steps,
        standard_cfg: cfg,
        standard_wtype: wtype,
    })
}

impl Modellsatz {
    /// Dateien, die für einen Lauf vorhanden sein müssen. u2netp nur, wenn mit
    /// dem Saliency-Weg freigestellt wird — beim Farb-Keying wird es nie geladen.
    pub fn benoetigt(&self, u2netp: bool) -> Vec<&Datei> {
        let mut dateien = vec![&self.dit, &self.llm, &self.vae];
        if u2netp {
            dateien.push(&self.u2netp);
        }
        dateien
    }
}

/// Lädt eine Datei, falls sie fehlt. Gibt `true` zurück, wenn heruntergeladen wurde.
///
/// Schreibt erst nach `<datei>.part` und benennt am Ende um, damit ein
/// abgebrochener Lauf keine halbe Datei als fertig zurücklässt. Eine vorhandene
/// `.part`-Datei wird per Range-Request fortgesetzt.
///
/// `token` ist der Hugging-Face-Zugriffstoken (`HF_TOKEN`): gated Repos liefern
/// ohne ihn eine HTML-Fehlerseite statt der Gewichte.
pub fn beschaffen(datei: &Datei, token: Option<&str>, fortschritt: &mut dyn FnMut(&str)) -> Result<bool> {
    if std::fs::metadata(&datei.pfad).is_ok_and(|m| m.len() > 0) {
        return Ok(false);
    }
    let Some(url) = &datei.url else {
        bail!("{} fehlt: {}", datei.label, datei.pfad.display());
    };
    if let Some(ordner) = datei.pfad.parent() {
        std::fs::create_dir_all(ordner)
            .with_context(|| format!("{} nicht anlegbar", ordner.display()))?;
    }
    fortschritt(&format!("lade {} ...", datei.label));

    let mut teil = datei.pfad.clone().into_os_string();
    teil.push(".part");
    let teil = PathBuf::from(teil);
    let bisher = std::fs::metadata(&teil).map(|m| m.len()).unwrap_or(0);

    let mut anfrage = ureq::get(url);
    if let Some(token) = token.filter(|t| !t.is_empty()) {
        anfrage = anfrage.set("Authorization", &format!("Bearer {token}"));
    }
    if bisher > 0 {
        anfrage = anfrage.set("Range", &format!("bytes={bisher}-"));
    }
    let antwort = anfrage.call().map_err(|e| {
        let gated = if url.contains("black-forest-labs") {
            " Dieses Repo ist gated: Lizenz auf huggingface.co bestätigen, dann HF_TOKEN setzen."
        } else {
            ""
        };
        anyhow::anyhow!("Download fehlgeschlagen: {} ({e}).{gated}", datei.label)
    })?;

    // 206 = der Server setzt fort, 200 = er schickt alles von vorn.
    let fortsetzen = bisher > 0 && antwort.status() == 206;
    let mut ziel = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(fortsetzen)
        .truncate(!fortsetzen)
        .open(&teil)
        .with_context(|| format!("{} nicht schreibbar", teil.display()))?;
    let mut leser = antwort.into_reader();
    let mut puffer = vec![0u8; 1 << 20];
    loop {
        let n = leser.read(&mut puffer).context("Download unterbrochen")?;
        if n == 0 {
            break;
        }
        ziel.write_all(&puffer[..n])?;
    }
    ziel.flush()?;
    drop(ziel);
    std::fs::rename(&teil, &datei.pfad)
        .with_context(|| format!("{} nicht umbenennbar", teil.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;

    fn satz(preset: Preset) -> Modellsatz {
        aufloesen(Path::new("/m"), preset, "Q5_K_M", None).unwrap()
    }

    #[test]
    fn der_default_ist_klein_9b() {
        assert_eq!(Preset::default(), Preset::Klein9b);
    }

    #[test]
    fn preset_wird_aus_text_gelesen() {
        assert_eq!("klein-9b".parse::<Preset>(), Ok(Preset::Klein9b));
        assert_eq!("klein-base-9b".parse::<Preset>(), Ok(Preset::KleinBase9b));
        let fehler = "klein-7b".parse::<Preset>().unwrap_err();
        assert!(fehler.contains("klein-7b") && fehler.contains("klein-4b"), "{fehler}");
    }

    #[test]
    fn preset_hat_die_namen_der_skripte_im_json() {
        assert_eq!(serde_json::to_string(&Preset::KleinBase9b).unwrap(), "\"klein-base-9b\"");
        assert_eq!(serde_json::from_str::<Preset>("\"klein-4b\"").unwrap(), Preset::Klein4b);
    }

    #[test]
    fn klein_9b_paart_das_modell_mit_dem_8b_encoder() {
        let s = satz(Preset::Klein9b);
        assert_eq!(s.dit.pfad, Path::new("/m/diffusion/flux-2-klein-9b-Q5_K_M.gguf"));
        assert_eq!(s.llm.pfad, Path::new("/m/text_encoder/Qwen3-8B-Q5_K_M.gguf"));
        assert!(s.llm.url.as_deref().unwrap().contains("Qwen3-8B-GGUF"));
    }

    #[test]
    fn klein_4b_paart_das_modell_mit_dem_4b_encoder() {
        let s = satz(Preset::Klein4b);
        assert_eq!(s.dit.pfad, Path::new("/m/diffusion/flux-2-klein-4b-Q5_K_M.gguf"));
        assert_eq!(s.llm.pfad, Path::new("/m/text_encoder/Qwen3-4B-Q5_K_M.gguf"));
    }

    #[test]
    fn base_braucht_cfg_4_zwanzig_steps_und_q8_umwandlung() {
        let s = satz(Preset::KleinBase9b);
        assert_eq!((s.standard_steps, s.standard_cfg), (20, 4.0));
        assert_eq!(s.standard_wtype, Some("q8_0"));
        assert!(s.dit.url.as_deref().unwrap().contains("black-forest-labs"));
    }

    #[test]
    fn distillierte_saetze_laufen_mit_cfg_1_und_4_steps() {
        for preset in [Preset::Klein4b, Preset::Klein9b] {
            let s = satz(preset);
            assert_eq!((s.standard_steps, s.standard_cfg, s.standard_wtype), (4, 1.0, None));
        }
    }

    #[test]
    fn quantisierung_wirkt_auf_modell_und_encoder() {
        let s = aufloesen(Path::new("/m"), Preset::Klein9b, "Q4_K_M", None).unwrap();
        assert!(s.dit.pfad.ends_with("flux-2-klein-9b-Q4_K_M.gguf"));
        assert!(s.llm.pfad.ends_with("Qwen3-8B-Q4_K_M.gguf"));
    }

    #[test]
    fn quantisierung_mit_pfadzeichen_wird_abgelehnt() {
        assert!(aufloesen(Path::new("/m"), Preset::Klein9b, "../../etc", None).is_err());
        assert!(aufloesen(Path::new("/m"), Preset::Klein9b, "", None).is_err());
    }

    #[test]
    fn eigener_encoder_ersetzt_den_des_presets() {
        let dir = tempfile::tempdir().unwrap();
        let eigener = dir.path().join("mistral.gguf");
        std::fs::write(&eigener, b"x").unwrap();
        let s = aufloesen(Path::new("/m"), Preset::Klein9b, "Q5_K_M", Some(&eigener)).unwrap();
        assert_eq!(s.llm.pfad, eigener);
        assert_eq!(s.llm.url, None, "eine eigene Datei wird nie heruntergeladen");
    }

    #[test]
    fn fehlender_eigener_encoder_ist_ein_fehler() {
        let fehler = aufloesen(Path::new("/m"), Preset::Klein9b, "Q5_K_M", Some(Path::new("/nix.gguf")))
            .unwrap_err()
            .to_string();
        assert!(fehler.contains("/nix.gguf"), "{fehler}");
    }

    #[test]
    fn u2netp_wird_nur_fuer_den_saliency_weg_verlangt() {
        let s = satz(Preset::Klein9b);
        assert_eq!(s.benoetigt(false).len(), 3);
        assert_eq!(s.benoetigt(true).len(), 4);
    }

    #[test]
    fn modellverzeichnis_bevorzugt_den_nachbarordner() {
        let wurzel = tempfile::tempdir().unwrap();
        let repo = wurzel.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        assert_eq!(standard_verzeichnis(&repo), repo.join("models"), "ohne Nachbarordner");
        std::fs::create_dir_all(wurzel.path().join("models")).unwrap();
        assert_eq!(
            standard_verzeichnis(&repo),
            wurzel.path().join("models").canonicalize().unwrap()
        );
    }

    /// Startet einen Server, der genau eine Anfrage beantwortet, und gibt
    /// Adresse sowie die empfangenen Header zurück.
    fn einmal_server(status: u16, body: &'static [u8]) -> (String, mpsc::Receiver<Vec<(String, String)>>) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let adresse = format!("http://{}", server.server_addr().to_ip().unwrap());
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            if let Ok(anfrage) = server.recv() {
                let header = anfrage
                    .headers()
                    .iter()
                    .map(|h| (h.field.to_string().to_ascii_lowercase(), h.value.to_string()))
                    .collect();
                tx.send(header).ok();
                let antwort = tiny_http::Response::from_data(body).with_status_code(status);
                anfrage.respond(antwort).ok();
            }
        });
        (adresse, rx)
    }

    fn datei(dir: &Path, url: Option<String>) -> Datei {
        Datei { pfad: dir.join("sub").join("modell.gguf"), url, label: "Testmodell".into() }
    }

    #[test]
    fn vorhandene_datei_wird_nicht_erneut_geladen() {
        let dir = tempfile::tempdir().unwrap();
        let d = datei(dir.path(), Some("http://127.0.0.1:1/nie".into()));
        std::fs::create_dir_all(d.pfad.parent().unwrap()).unwrap();
        std::fs::write(&d.pfad, b"gewichte").unwrap();
        assert!(!beschaffen(&d, None, &mut |_| {}).unwrap());
    }

    #[test]
    fn fehlende_datei_wird_geladen_und_ohne_part_hinterlassen() {
        let dir = tempfile::tempdir().unwrap();
        let (adresse, _) = einmal_server(200, b"gewichte");
        let d = datei(dir.path(), Some(format!("{adresse}/modell.gguf")));
        assert!(beschaffen(&d, None, &mut |_| {}).unwrap());
        assert_eq!(std::fs::read(&d.pfad).unwrap(), b"gewichte");
        assert!(!d.pfad.with_extension("gguf.part").exists());
    }

    #[test]
    fn hf_token_geht_als_bearer_header_mit() {
        let dir = tempfile::tempdir().unwrap();
        let (adresse, header) = einmal_server(200, b"x");
        let d = datei(dir.path(), Some(format!("{adresse}/m")));
        beschaffen(&d, Some("hf_geheim"), &mut |_| {}).unwrap();
        let header = header.recv().unwrap();
        assert!(
            header.iter().any(|(k, v)| k == "authorization" && v == "Bearer hf_geheim"),
            "{header:?}"
        );
    }

    #[test]
    fn fehlgeschlagener_download_hinterlaesst_keine_datei() {
        let dir = tempfile::tempdir().unwrap();
        let (adresse, _) = einmal_server(404, b"nix");
        let d = datei(dir.path(), Some(format!("{adresse}/m")));
        let fehler = beschaffen(&d, None, &mut |_| {}).unwrap_err().to_string();
        assert!(fehler.contains("Testmodell"), "{fehler}");
        assert!(!d.pfad.exists());
    }

    #[test]
    fn gated_repo_bekommt_einen_hinweis_auf_hf_token() {
        let dir = tempfile::tempdir().unwrap();
        let (adresse, _) = einmal_server(401, b"gated");
        let d = Datei {
            pfad: dir.path().join("m.safetensors"),
            url: Some(format!("{adresse}/black-forest-labs/x")),
            label: "base".into(),
        };
        let fehler = beschaffen(&d, None, &mut |_| {}).unwrap_err().to_string();
        assert!(fehler.contains("HF_TOKEN"), "{fehler}");
    }

    #[test]
    fn eigene_datei_ohne_url_wird_nicht_beschafft() {
        let dir = tempfile::tempdir().unwrap();
        let d = datei(dir.path(), None);
        assert!(beschaffen(&d, None, &mut |_| {}).is_err());
    }

    #[test]
    fn abgebrochener_download_wird_per_range_fortgesetzt() {
        let dir = tempfile::tempdir().unwrap();
        let (adresse, header) = einmal_server(206, b"REST");
        let d = datei(dir.path(), Some(format!("{adresse}/m")));
        std::fs::create_dir_all(d.pfad.parent().unwrap()).unwrap();
        let mut teil = d.pfad.clone().into_os_string();
        teil.push(".part");
        std::fs::write(&teil, b"ANFANG").unwrap();
        beschaffen(&d, None, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(&d.pfad).unwrap(), b"ANFANGREST");
        let header = header.recv().unwrap();
        assert!(header.iter().any(|(k, v)| k == "range" && v == "bytes=6-"), "{header:?}");
    }
}
