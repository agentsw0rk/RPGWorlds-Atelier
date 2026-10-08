//! Beobachtbarkeit: Messwerte und Phasenmarken eines Laufs als JSON-Zeilen.
//!
//! Eine Zeile pro Ereignis (`metrics.jsonl`), sofort geschrieben — wenn der
//! Rechner mitten im Lauf einfriert, bleibt die letzte Zeile davor erhalten.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Ein Ereignis im Metrikstrom. `t_ms` zählt ab Beginn des Jobs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "art", rename_all = "snake_case")]
pub enum Eintrag {
    /// Ein Abschnitt des Ablaufs beginnt (z. B. `sampling`, `vae_decode`).
    Phase { t_ms: u64, name: String },
    /// Eine Stufe von sd.cpp ist fertig (aus dem Log), mit ihrer Dauer.
    Stufe {
        t_ms: u64,
        name: String,
        dauer_s: f32,
    },
    /// Einstellungen des Bildes, das gleich erzeugt wird.
    Kontext { t_ms: u64, kontext: Kontext },
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

/// Was ein Bild ausmacht, soweit es Speicher und Dauer beeinflusst.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Kontext {
    pub preset: String,
    pub wtype: Option<String>,
    pub breite: u32,
    pub hoehe: u32,
    pub steps: u32,
    /// Pixelmaße (Breite, Höhe) jeder Referenz, so wie sie ans Modell gehen.
    pub referenzen: Vec<(u32, u32)>,
    pub mmap: bool,
    pub flash_attention: bool,
    pub vae_tiling: bool,
    pub threads: i32,
}

/// Gemeinsamer Zugang zum Metrikstrom eines Jobs. Klonbar: Sampler-Thread,
/// Log-Callback und Ablauf schreiben in dieselbe Datei.
///
/// Ein Fehler beim Schreiben bricht nie einen Lauf ab — Beobachtung darf das
/// Beobachtete nicht kaputt machen.
#[derive(Clone)]
pub struct Metriken {
    innen: Arc<Mutex<Innen>>,
}

struct Innen {
    schreiber: MetrikSchreiber,
    uhr: Box<dyn Fn() -> u64 + Send>,
}

impl Metriken {
    /// Zeit in Millisekunden ab dem Aufruf.
    pub fn oeffnen(pfad: &Path) -> Result<Self> {
        let start = std::time::Instant::now();
        Self::mit_uhr(pfad, Box::new(move || start.elapsed().as_millis() as u64))
    }

    pub fn mit_uhr(pfad: &Path, uhr: Box<dyn Fn() -> u64 + Send>) -> Result<Self> {
        let schreiber = MetrikSchreiber::oeffnen(pfad)?;
        Ok(Metriken {
            innen: Arc::new(Mutex::new(Innen { schreiber, uhr })),
        })
    }

    /// Schreibt einen Eintrag; `bauen` bekommt die aktuelle Jobzeit.
    pub fn eintrag(&self, bauen: impl FnOnce(u64) -> Eintrag) {
        let mut innen = self.innen.lock().unwrap_or_else(|e| e.into_inner());
        let eintrag = bauen((innen.uhr)());
        if let Err(fehler) = innen.schreiber.schreibe(&eintrag) {
            eprintln!("Metrik: {fehler:#}");
        }
    }

    /// Hält die Einstellungen des folgenden Bildes fest.
    pub fn kontext(&self, kontext: Kontext) {
        self.eintrag(|t_ms| Eintrag::Kontext { t_ms, kontext });
    }

    /// Ein Abschnitt des Ablaufs beginnt.
    pub fn phase(&self, name: &str) {
        self.eintrag(|t_ms| Eintrag::Phase {
            t_ms,
            name: name.into(),
        });
    }

    /// Eine Logzeile von sd.cpp; nur fertige Stufen werden zu Einträgen.
    pub fn sd_log(&self, zeile: &str) {
        if let Some((name, dauer_s)) = stufe_aus_logzeile(zeile) {
            self.eintrag(|t_ms| Eintrag::Stufe {
                t_ms,
                name: name.into(),
                dauer_s,
            });
        }
    }
}

/// Ein Messpunkt von Prozess und System.
#[derive(Debug, Clone, PartialEq)]
pub struct Messung {
    pub rss_mb: u64,
    pub frei_mb: u64,
    pub swap_mb: u64,
    pub cpu_pct: f32,
}

/// Woher die Messwerte kommen. Die echte Quelle fragt das Betriebssystem, Tests
/// setzen eine feste ein.
pub trait Messquelle: Send {
    fn messen(&mut self) -> Messung;
}

/// Hintergrund-Thread, der in festem Abstand eine Probe schreibt — auch während
/// sd.cpp rechnet und der Ablauf selbst nichts melden kann.
pub struct Sampler {
    halt: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<()>,
}

impl Sampler {
    /// Die erste Probe kommt sofort, dann alle `abstand`.
    pub fn starten(metriken: Metriken, mut quelle: Box<dyn Messquelle>, abstand: Duration) -> Self {
        let halt = Arc::new(AtomicBool::new(false));
        let thread = {
            let halt = halt.clone();
            std::thread::spawn(move || {
                while !halt.load(Ordering::Relaxed) {
                    let m = quelle.messen();
                    metriken.eintrag(|t_ms| Eintrag::Probe {
                        t_ms,
                        rss_mb: m.rss_mb,
                        frei_mb: m.frei_mb,
                        swap_mb: m.swap_mb,
                        cpu_pct: m.cpu_pct,
                    });
                    std::thread::park_timeout(abstand);
                }
            })
        };
        Sampler { halt, thread }
    }

    /// Hält den Thread an und wartet, bis er beendet ist.
    pub fn stoppen(self) {
        self.halt.store(true, Ordering::Relaxed);
        self.thread.thread().unpark();
        let _ = self.thread.join();
    }
}

/// Liest einen Metrikstrom zurück. Eine unvollständige letzte Zeile (Absturz
/// mitten im Schreiben) wird übergangen, nicht als Fehler gemeldet.
pub fn eintraege_lesen(pfad: &Path) -> Result<Vec<Eintrag>> {
    let text = std::fs::read_to_string(pfad)
        .with_context(|| format!("{} nicht lesbar", pfad.display()))?;
    Ok(text
        .lines()
        .filter_map(|zeile| serde_json::from_str(zeile).ok())
        .collect())
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

    fn feste_uhr(ms: u64) -> (tempfile::TempDir, std::path::PathBuf, Metriken) {
        let dir = tempfile::tempdir().unwrap();
        let pfad = dir.path().join("metrics.jsonl");
        let m = Metriken::mit_uhr(&pfad, Box::new(move || ms)).unwrap();
        (dir, pfad, m)
    }

    #[test]
    fn sd_logzeile_wird_zu_stufeneintrag_mit_jobzeit() {
        let (_dir, pfad, m) = feste_uhr(4200);
        m.sd_log("x.cpp:1 - sampling completed, taking 2.50s\n");
        assert_eq!(
            eintraege_lesen(&pfad).unwrap(),
            vec![Eintrag::Stufe {
                t_ms: 4200,
                name: "sampling".into(),
                dauer_s: 2.5
            }]
        );
    }

    #[test]
    fn phase_schreibt_marke_mit_jobzeit() {
        let (_dir, pfad, m) = feste_uhr(900);
        m.phase("referenz");
        assert_eq!(
            eintraege_lesen(&pfad).unwrap(),
            vec![Eintrag::Phase {
                t_ms: 900,
                name: "referenz".into()
            }]
        );
    }

    #[test]
    fn kontext_haelt_einstellungen_und_referenzmasse_fest() {
        let (_dir, pfad, m) = feste_uhr(0);
        let kontext = Kontext {
            preset: "klein-9b".into(),
            wtype: Some("q8_0".into()),
            breite: 768,
            hoehe: 1536,
            steps: 4,
            referenzen: vec![(1024, 1024)],
            mmap: false,
            flash_attention: false,
            vae_tiling: false,
            threads: 8,
        };
        m.kontext(kontext.clone());
        assert_eq!(
            eintraege_lesen(&pfad).unwrap(),
            vec![Eintrag::Kontext { t_ms: 0, kontext }]
        );
    }

    struct FesteQuelle;
    impl Messquelle for FesteQuelle {
        fn messen(&mut self) -> Messung {
            Messung {
                rss_mb: 7000,
                frei_mb: 300,
                swap_mb: 1024,
                cpu_pct: 250.0,
            }
        }
    }

    fn proben(pfad: &std::path::Path) -> usize {
        eintraege_lesen(pfad)
            .unwrap()
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    Eintrag::Probe {
                        rss_mb: 7000,
                        swap_mb: 1024,
                        ..
                    }
                )
            })
            .count()
    }

    #[test]
    fn sampler_schreibt_proben_bis_er_gestoppt_wird() {
        let (_dir, pfad, m) = feste_uhr(0);
        let sampler = Sampler::starten(
            m,
            Box::new(FesteQuelle),
            std::time::Duration::from_millis(5),
        );
        let frist = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while proben(&pfad) < 2 {
            assert!(
                std::time::Instant::now() < frist,
                "keine zwei Proben in 5 s"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        sampler.stoppen();
        let danach = proben(&pfad);
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(proben(&pfad), danach, "nach stoppen() kommt nichts mehr");
    }
}
