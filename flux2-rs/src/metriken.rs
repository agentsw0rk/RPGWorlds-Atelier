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
}
