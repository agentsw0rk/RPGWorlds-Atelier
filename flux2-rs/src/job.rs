//! Zustand eines Jobs: was die API über einen Auftrag und seine Bilder meldet
//! und auf der Platte festhält.

use crate::auftrag::{Auftrag, Plan};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Wartend,
    Laeuft,
    /// Alle Bilder sind fertig (oder waren schon vorhanden).
    Fertig,
    /// Mindestens ein Bild ist fehlgeschlagen, oder der Job konnte nicht starten.
    Fehlgeschlagen,
    Abgebrochen,
}

impl JobStatus {
    pub fn ist_beendet(self) -> bool {
        matches!(self, JobStatus::Fertig | JobStatus::Fehlgeschlagen | JobStatus::Abgebrochen)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BildStatus {
    Wartend,
    Laeuft,
    Fertig,
    Fehlgeschlagen,
    /// Lag schon vor (Batch ohne `force`).
    Vorhanden,
    Abgebrochen,
}

/// Ein Bild eines Jobs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bild {
    pub stamm: String,
    pub seed: i64,
    pub status: BildStatus,
    /// Dateiname des fertigen Bildes im Ausgabeordner (`<stamm>.png`).
    pub datei: Option<String>,
    /// Das Rohbild vor dem Freistellen, falls freigestellt wurde.
    pub roh: Option<String>,
    pub fehler: Option<String>,
    pub dauer_s: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub auftrag: Auftrag,
    pub plan: Plan,
    pub status: JobStatus,
    /// Unix-Sekunden.
    pub erstellt: u64,
    pub gestartet: Option<u64>,
    pub beendet: Option<u64>,
    /// Ausgabeordner relativ zum Datenverzeichnis, mit `/` getrennt.
    pub ausgabe: String,
    pub bilder: Vec<Bild>,
    /// Grund, wenn der Job gar nicht erst laufen konnte (z. B. Download gescheitert).
    pub fehler: Option<String>,
    /// Lesbare Zeile zum aktuellen Schritt ("lade Modell …", "Bild 2/4").
    pub meldung: String,
    #[serde(default)]
    pub abbruch_angefordert: bool,
}

impl Job {
    /// Neuer, wartender Job aus einem geplanten Auftrag.
    pub fn neu(id: String, auftrag: Auftrag, plan: Plan, ausgabe: String, jetzt: u64) -> Job {
        let bilder = plan
            .aufgaben
            .iter()
            .map(|a| Bild {
                stamm: a.stamm.clone(),
                seed: a.seed,
                status: if a.uebersprungen { BildStatus::Vorhanden } else { BildStatus::Wartend },
                datei: a.uebersprungen.then(|| format!("{}.png", a.stamm)),
                roh: None,
                fehler: None,
                dauer_s: None,
            })
            .collect();
        Job {
            id,
            auftrag,
            plan,
            status: JobStatus::Wartend,
            erstellt: jetzt,
            gestartet: None,
            beendet: None,
            ausgabe,
            bilder,
            fehler: None,
            meldung: "wartet".into(),
            abbruch_angefordert: false,
        }
    }

    /// Endstatus aus den Bildern: ein fehlgeschlagenes Bild macht den Job
    /// fehlgeschlagen, die übrigen Bilder bleiben trotzdem erhalten.
    pub fn endstatus(&self) -> JobStatus {
        if self.fehler.is_some() || self.bilder.iter().any(|b| b.status == BildStatus::Fehlgeschlagen) {
            JobStatus::Fehlgeschlagen
        } else if self.bilder.iter().any(|b| b.status == BildStatus::Abgebrochen) {
            JobStatus::Abgebrochen
        } else {
            JobStatus::Fertig
        }
    }
}
