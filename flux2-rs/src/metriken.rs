//! Beobachtbarkeit: Messwerte und Phasenmarken eines Laufs als JSON-Zeilen.
//!
//! Eine Zeile pro Ereignis (`metrics.jsonl`), sofort geschrieben — wenn der
//! Rechner mitten im Lauf einfriert, bleibt die letzte Zeile davor erhalten.

use serde::Serialize;

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
}
