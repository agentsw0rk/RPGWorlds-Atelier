//! Beobachtbarkeit: Messwerte und Phasenmarken eines Laufs als JSON-Zeilen.
//!
//! Eine Zeile pro Ereignis (`metrics.jsonl`), sofort geschrieben — wenn der
//! Rechner mitten im Lauf einfriert, bleibt die letzte Zeile davor erhalten.

use anyhow::{Context, Result};
use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

/// Ein Ereignis im Metrikstrom. `t_ms` zählt ab Beginn des Jobs.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "art", rename_all = "snake_case")]
pub enum Eintrag {
    /// Ein Abschnitt des Ablaufs beginnt (z. B. `sampling`, `vae_decode`).
    Phase { t_ms: u64, name: String },
    /// Messwert des Samplers. Speicher in MiB; `cpu_pct` über alle Kerne
    /// (400 = vier Kerne voll ausgelastet).
    Probe {
        t_ms: u64,
        rss_mb: u64,
        frei_mb: u64,
        swap_mb: u64,
        cpu_pct: f32,
    },
}

impl Eintrag {
    /// Die Zeile für `metrics.jsonl`, ohne Zeilenumbruch.
    pub fn zeile(&self) -> String {
        serde_json::to_string(self).expect("Eintrag ist immer serialisierbar")
    }
}

/// Hängt Einträge zeilenweise an `metrics.jsonl` an.
///
/// Jede Zeile geht unmittelbar auf die Platte (kein Puffer, `sync_data`): der
/// Fall, für den das gebaut ist, ist ein Rechner, der mitten im Lauf einfriert.
pub struct MetrikSchreiber {
    datei: File,
}

impl MetrikSchreiber {
    pub fn oeffnen(pfad: &Path) -> Result<Self> {
        let datei = OpenOptions::new()
            .create(true)
            .append(true)
            .open(pfad)
            .with_context(|| format!("{} nicht öffnbar", pfad.display()))?;
        Ok(MetrikSchreiber { datei })
    }

    pub fn schreibe(&mut self, eintrag: &Eintrag) -> Result<()> {
        // Eine Zeile in einem einzigen write: so mischen sich Zeilen zweier
        // Schreiber (Sampler und Ablauf) nicht mitten im Text.
        let mut zeile = eintrag.zeile();
        zeile.push('\n');
        self.datei
            .write_all(zeile.as_bytes())
            .context("Metrik nicht schreibbar")?;
        self.datei
            .sync_data()
            .context("Metrik nicht auf die Platte gebracht")?;
        Ok(())
    }
}

/// Liest aus einer Logzeile von sd.cpp, welche Stufe fertig ist und wie lange
/// sie gedauert hat. sd.cpp meldet das als `<stufe> completed, taking 1.23s`.
pub fn stufe_aus_logzeile(zeile: &str) -> Option<(&'static str, f32)> {
    /// Text in sd.cpp → Name in den Metriken.
    const STUFEN: [(&str, &str); 5] = [
        ("loading tensors", "laden"),
        ("get_learned_condition", "text_encoder"),
        ("encode_first_stage", "vae_encode"),
        ("sampling", "sampling"),
        ("decode_first_stage", "vae_decode"),
    ];
    let (name, rest) = STUFEN.iter().find_map(|(sd_name, name)| {
        let rest = zeile
            .split_once(&format!("{sd_name} completed, taking "))?
            .1;
        Some((*name, rest))
    })?;
    // Die Zahl endet am `s`; bei `loading tensors` folgt noch eine Klammer.
    let sekunden = rest.split_once('s')?.0.parse().ok()?;
    Some((name, sekunden))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phasenmarke_ist_genau_eine_json_zeile_mit_zeit_art_und_name() {
        let zeile = Eintrag::Phase {
            t_ms: 1500,
            name: "sampling".into(),
        }
        .zeile();
        assert!(!zeile.contains('\n'), "{zeile:?}");
        let wert: serde_json::Value = serde_json::from_str(&zeile).expect("gültiges JSON");
        assert_eq!(wert["t_ms"], 1500);
        assert_eq!(wert["art"], "phase");
        assert_eq!(wert["name"], "sampling");
    }

    #[test]
    fn messwert_traegt_prozess_speicher_systemspeicher_swap_und_cpu() {
        let zeile = Eintrag::Probe {
            t_ms: 2000,
            rss_mb: 7300,
            frei_mb: 410,
            swap_mb: 2048,
            cpu_pct: 380.5,
        }
        .zeile();
        let wert: serde_json::Value = serde_json::from_str(&zeile).expect("gültiges JSON");
        assert_eq!(wert["art"], "probe");
        assert_eq!(wert["rss_mb"], 7300);
        assert_eq!(wert["frei_mb"], 410);
        assert_eq!(wert["swap_mb"], 2048);
        assert_eq!(wert["cpu_pct"], 380.5);
    }

    #[test]
    fn geschriebene_eintraege_sind_sofort_lesbar_auch_bei_offenem_schreiber() {
        let dir = tempfile::tempdir().unwrap();
        let pfad = dir.path().join("metrics.jsonl");
        let mut schreiber = MetrikSchreiber::oeffnen(&pfad).unwrap();
        schreiber
            .schreibe(&Eintrag::Phase {
                t_ms: 0,
                name: "laden".into(),
            })
            .unwrap();
        schreiber
            .schreibe(&Eintrag::Phase {
                t_ms: 10,
                name: "sampling".into(),
            })
            .unwrap();
        // Der Schreiber lebt noch: ein Absturz des Rechners käme genauso.
        let inhalt = std::fs::read_to_string(&pfad).unwrap();
        let zeilen: Vec<&str> = inhalt.lines().collect();
        assert_eq!(zeilen.len(), 2, "{inhalt:?}");
        assert!(zeilen[1].contains("sampling"));
    }

    #[test]
    fn logzeile_mit_sampling_dauer_ergibt_stufe_und_sekunden() {
        let zeile = "stable-diffusion.cpp:4389 - sampling completed, taking 123.45s\n";
        assert_eq!(stufe_aus_logzeile(zeile), Some(("sampling", 123.45)));
    }

    #[test]
    fn die_uebrigen_stufen_werden_erkannt_auch_mit_text_hinter_der_zahl() {
        let faelle = [
            (
                "x.cpp:1 - decode_first_stage completed, taking 50.20s",
                "vae_decode",
                50.2,
            ),
            (
                "x.cpp:1 - encode_first_stage completed, taking 3.10s",
                "vae_encode",
                3.1,
            ),
            (
                "x.cpp:1 - get_learned_condition completed, taking 4.00s",
                "text_encoder",
                4.0,
            ),
            (
                "x.cpp:1 - loading tensors completed, taking 12.30s (read: 8.00s, memcpy: 1.00s)",
                "laden",
                12.3,
            ),
        ];
        for (zeile, name, sekunden) in faelle {
            assert_eq!(stufe_aus_logzeile(zeile), Some((name, sekunden)), "{zeile}");
        }
        assert_eq!(stufe_aus_logzeile("x.cpp:1 - Version: Flux.2 klein"), None);
    }
}
