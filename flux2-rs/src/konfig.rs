//! Konfiguration des API-Servers aus Umgebungsvariablen.
//!
//! Wie im übrigen Projekt gibt es keinen Argument-Parser: jede Einstellung ist
//! eine Umgebungsvariable.
//!
//! | Variable | Default | Bedeutung |
//! |---|---|---|
//! | `FLUX2_BIND` | `127.0.0.1:8080` | Adresse, auf der der Server lauscht |
//! | `FLUX2_API_TOKEN` | – | erwartetes Bearer-Token; Pflicht bei nicht-lokaler Adresse |
//! | `FLUX2_DATA` | `./api-daten` | Jobs, Uploads, Batch-Ausgabeordner |
//! | `FLUX2_PROJECT` | `.` | Projektverzeichnis mit den Listen (`fullbody.txt`, …) |
//! | `MODELS_DIR` | `<projekt>/../models` bzw. `<projekt>/models` | Modellgewichte |
//! | `THREADS` | Anzahl der Kerne | Rechenthreads |
//! | `MMAP` | `0` | Gewichte per mmap laden (auf Metal/CUDA aus lassen) |
//! | `FLASH_ATTENTION` | `0` | Flash-Attention |
//! | `VAE_TILING` | `0` | gekacheltes VAE-Decoding (spart Speicher) |
//! | `HF_TOKEN` | – | Hugging-Face-Token für gated Repos |
//! | `REF_BG` | `ffffff` | Farbe für transparente Referenzbilder |
//! | `REF_MAX_PX` | – (aus) | Obergrenze für die lange Kante der Referenzbilder in Pixeln (64–2048); ein Auftrag kann sie mit `ref_max_px` überschreiben |
//! | `LOG` | `1` | sd.cpp-Log: 0 aus, 1 INFO, 2 mit DEBUG |
//! | `METRICS_MS` | `1000` | Abstand der Speicher-Messungen pro Job in ms (`jobs/<id>/metrics.jsonl`); `0` = aus |

use crate::dienst::Konfig;
use crate::lauf::Host;
use crate::modelle;
use crate::params::{flag_wert, hex_farbe};
use anyhow::{bail, Context, Result};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ServerKonfig {
    pub bind: SocketAddr,
    pub token: Option<String>,
    pub dienst: Konfig,
    pub log: usize,
    /// Abstand der Speicher-Messungen; `None` = Beobachtung aus.
    pub metriken_abstand: Option<std::time::Duration>,
}

/// Liest die Konfiguration. `env` liefert den Wert einer Variablen — so lässt
/// sich das ohne Eingriff in die echte Umgebung testen.
pub fn aus_umgebung(env: &dyn Fn(&str) -> Option<String>) -> Result<ServerKonfig> {
    // Leere Variablen zählen wie nicht gesetzt.
    let wert = |name: &str| env(name).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let flag = |name: &str| flag_wert(wert(name).as_deref(), false);

    let bind_text = wert("FLUX2_BIND").unwrap_or_else(|| "127.0.0.1:8080".into());
    let bind: SocketAddr = bind_text
        .parse()
        .with_context(|| format!("FLUX2_BIND='{bind_text}' ist keine Adresse wie 127.0.0.1:8080"))?;
    let token = wert("FLUX2_API_TOKEN");
    // Wer den Server ins Netz stellt, braucht einen Schlüssel: ohne ihn könnte
    // jeder im Netz Rechenzeit verbrauchen und Dateien hochladen.
    if !bind.ip().is_loopback() && token.is_none() {
        bail!("FLUX2_BIND={bind} ist von außen erreichbar — dafür FLUX2_API_TOKEN setzen.");
    }

    let projekt = PathBuf::from(wert("FLUX2_PROJECT").unwrap_or_else(|| ".".into()));
    let daten = PathBuf::from(wert("FLUX2_DATA").unwrap_or_else(|| "api-daten".into()));
    let modelle = wert("MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| modelle::standard_verzeichnis(&projekt));

    let threads: i32 = match wert("THREADS") {
        Some(t) => t.parse().ok().filter(|&n| n > 0).with_context(|| format!("THREADS='{t}' ist keine positive Zahl"))?,
        None => std::thread::available_parallelism().map(|n| n.get() as i32).unwrap_or(4),
    };
    let ref_bg = hex_farbe(&wert("REF_BG").unwrap_or_else(|| "ffffff".into()))?;
    let ref_max_px = match wert("REF_MAX_PX") {
        Some(t) => {
            let px: u32 = t.parse().with_context(|| format!("REF_MAX_PX='{t}' ist keine Zahl (Pixel an der langen Kante, 0 = aus)"))?;
            if px == 0 {
                None
            } else if (64..=2048).contains(&px) {
                Some(px)
            } else {
                bail!("REF_MAX_PX={px} liegt außerhalb von 64..2048.");
            }
        }
        None => None,
    };
    let log = match wert("LOG") {
        Some(l) => l.parse().with_context(|| format!("LOG='{l}' ist keine Zahl"))?,
        None => 1,
    };

    let metriken_abstand = match wert("METRICS_MS") {
        Some(t) => {
            let ms: u64 = t.parse().with_context(|| format!("METRICS_MS='{t}' ist keine Zahl (Millisekunden, 0 = aus)"))?;
            (ms > 0).then(|| std::time::Duration::from_millis(ms))
        }
        None => Some(std::time::Duration::from_millis(1000)),
    };

    Ok(ServerKonfig {
        bind,
        token,
        log,
        metriken_abstand,
        dienst: Konfig {
            daten,
            projekt,
            host: Host {
                modelle,
                threads,
                mmap: flag("MMAP"),
                flash_attention: flag("FLASH_ATTENTION"),
                vae_tiling: flag("VAE_TILING"),
                hf_token: wert("HF_TOKEN"),
                ref_bg,
                ref_max_px,
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn konfig(paare: &[(&str, &str)]) -> Result<ServerKonfig> {
        let karte: HashMap<String, String> = paare.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        aus_umgebung(&|name| karte.get(name).cloned())
    }

    #[test]
    fn ohne_variablen_gelten_die_sicheren_defaults() {
        let k = konfig(&[]).unwrap();
        assert_eq!(k.bind.to_string(), "127.0.0.1:8080");
        assert_eq!(k.token, None);
        assert!(!k.dienst.host.mmap && !k.dienst.host.flash_attention && !k.dienst.host.vae_tiling);
        assert_eq!(k.dienst.host.ref_bg, [255, 255, 255]);
        assert!(k.dienst.host.threads >= 1);
    }

    #[test]
    fn schalter_werden_wie_in_den_skripten_gelesen() {
        let k = konfig(&[("MMAP", "1"), ("FLASH_ATTENTION", "true"), ("VAE_TILING", "0")]).unwrap();
        assert!(k.dienst.host.mmap && k.dienst.host.flash_attention && !k.dienst.host.vae_tiling);
    }

    #[test]
    fn nicht_lokale_adresse_ohne_token_wird_abgelehnt() {
        let fehler = konfig(&[("FLUX2_BIND", "0.0.0.0:8080")]).unwrap_err().to_string();
        assert!(fehler.contains("FLUX2_API_TOKEN"), "{fehler}");
        assert!(konfig(&[("FLUX2_BIND", "0.0.0.0:8080"), ("FLUX2_API_TOKEN", "geheim")]).is_ok());
        assert!(konfig(&[("FLUX2_BIND", "[::1]:9000")]).is_ok(), "IPv6-Loopback ist lokal");
    }

    #[test]
    fn leere_variablen_zaehlen_als_nicht_gesetzt() {
        let k = konfig(&[("FLUX2_API_TOKEN", "  "), ("HF_TOKEN", "")]).unwrap();
        assert_eq!(k.token, None);
        assert_eq!(k.dienst.host.hf_token, None);
    }

    #[test]
    fn ungueltige_werte_nennen_die_variable() {
        for (name, wert) in [("FLUX2_BIND", "überall"), ("THREADS", "0"), ("THREADS", "viele"), ("REF_BG", "weiss"), ("LOG", "laut")] {
            let fehler = konfig(&[(name, wert)]).unwrap_err().to_string();
            assert!(fehler.contains(name) || fehler.contains("Farbe"), "{name}: {fehler}");
        }
    }

    #[test]
    fn modellverzeichnis_laesst_sich_setzen() {
        let k = konfig(&[("MODELS_DIR", "/ssd/modelle"), ("HF_TOKEN", "hf_x")]).unwrap();
        assert_eq!(k.dienst.host.modelle, PathBuf::from("/ssd/modelle"));
        assert_eq!(k.dienst.host.hf_token.as_deref(), Some("hf_x"));
    }

    #[test]
    fn die_beobachtung_misst_ohne_angabe_jede_sekunde() {
        let k = konfig(&[]).unwrap();
        assert_eq!(k.metriken_abstand, Some(std::time::Duration::from_millis(1000)));
    }

    #[test]
    fn metrics_ms_setzt_den_abstand_null_schaltet_aus_unsinn_nennt_die_variable() {
        let ms = |w: &str| konfig(&[("METRICS_MS", w)]);
        assert_eq!(ms("250").unwrap().metriken_abstand, Some(std::time::Duration::from_millis(250)));
        assert_eq!(ms("0").unwrap().metriken_abstand, None);
        let fehler = ms("oft").unwrap_err().to_string();
        assert!(fehler.contains("METRICS_MS"), "{fehler}");
    }

    #[test]
    fn ref_max_px_ist_ohne_angabe_und_bei_null_aus_sonst_zwischen_64_und_2048() {
        let px = |w: &str| konfig(&[("REF_MAX_PX", w)]).map(|k| k.dienst.host.ref_max_px);
        assert_eq!(konfig(&[]).unwrap().dienst.host.ref_max_px, None);
        assert_eq!(px("0").unwrap(), None);
        assert_eq!(px("512").unwrap(), Some(512));
        for unsinn in ["gross", "8", "9000"] {
            let fehler = px(unsinn).unwrap_err().to_string();
            assert!(fehler.contains("REF_MAX_PX"), "{unsinn}: {fehler}");
        }
    }
}
