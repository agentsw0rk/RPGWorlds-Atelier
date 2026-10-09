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
        /// Bezugsgrößen für die Skala einer Anzeige.
        gesamt_mb: u64,
        swap_gesamt_mb: u64,
        kerne: u32,
        /// Seit Rechnerstart ein- und ausgelagerter Swap (kumuliert, MiB). Aus zwei
        /// Proben ergibt sich die Rate; Läufe ohne diese Felder lesen als 0.
        #[serde(default)]
        swap_ein_mb: u64,
        #[serde(default)]
        swap_aus_mb: u64,
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
    /// Obergrenze für die Referenzgröße (lange Kante), falls eine galt.
    #[serde(default)]
    pub ref_max_px: Option<u32>,
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

impl std::fmt::Debug for Metriken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Metriken")
    }
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

/// Der Metrikstrom, in den der globale sd.cpp-Log-Callback schreibt. Der Callback
/// ist ein C-Zeiger ohne Zustand und kennt keinen Job; der Dienst setzt das
/// Ziel für die Dauer eines Jobs.
static SD_LOG_ZIEL: Mutex<Option<Metriken>> = Mutex::new(None);

pub fn sd_log_ziel_setzen(ziel: Option<Metriken>) {
    *SD_LOG_ZIEL.lock().unwrap_or_else(|e| e.into_inner()) = ziel;
}

/// Reicht eine Logzeile von sd.cpp an das gesetzte Ziel weiter, falls es eins gibt.
pub fn sd_log_weiterleiten(zeile: &str) {
    let ziel = SD_LOG_ZIEL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    if let Some(m) = ziel {
        m.sd_log(zeile);
    }
}

/// Liest aus der Ausgabe von `vm_stat` (macOS), wie viel Swap seit dem Start des
/// Rechners ein- und ausgelagert wurde, in MiB: `(ein, aus)`.
///
/// Gezählt werden `Swapins`/`Swapouts`, nicht `Pageins`/`Pageouts`: die zweiten
/// enthalten auch jeden Dateizugriff und sagen nichts über Speichernot.
pub fn swap_zaehler_aus_vm_stat(text: &str) -> Option<(u64, u64)> {
    let seite: u64 = text
        .lines()
        .next()?
        .split_once("page size of ")?
        .1
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let zaehler = |name: &str| -> Option<u64> {
        let wert = text.lines().find_map(|z| z.strip_prefix(name))?;
        wert.trim().trim_end_matches('.').parse().ok()
    };
    let mib = |seiten: u64| seiten * seite / (1024 * 1024);
    Some((mib(zaehler("Swapins:")?), mib(zaehler("Swapouts:")?)))
}

/// Aktueller Swap-Zähler des Systems, `(ein, aus)` in MiB. Nur macOS; anderswo 0.
fn swap_zaehler() -> (u64, u64) {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("vm_stat")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|text| swap_zaehler_aus_vm_stat(&text))
            .unwrap_or((0, 0))
    }
    #[cfg(not(target_os = "macos"))]
    {
        (0, 0)
    }
}

/// Ein Messpunkt von Prozess und System.
#[derive(Debug, Clone, PartialEq)]
pub struct Messung {
    pub rss_mb: u64,
    pub frei_mb: u64,
    pub swap_mb: u64,
    pub cpu_pct: f32,
    pub gesamt_mb: u64,
    pub swap_gesamt_mb: u64,
    pub kerne: u32,
    pub swap_ein_mb: u64,
    pub swap_aus_mb: u64,
}

/// Woher die Messwerte kommen. Die echte Quelle fragt das Betriebssystem, Tests
/// setzen eine feste ein.
pub trait Messquelle: Send {
    fn messen(&mut self) -> Messung;
}

/// Die echte Messquelle: fragt über `sysinfo` das Betriebssystem.
///
/// Gemessen wird der eigene Prozess (RSS, CPU) und das System (verfügbarer
/// Speicher, belegter Swap). Auf Apple Silicon teilen sich CPU und GPU den
/// Speicher, deshalb zeigt der Systemwert auch, was Metal belegt.
pub struct SystemQuelle {
    system: sysinfo::System,
    pid: sysinfo::Pid,
}

impl SystemQuelle {
    pub fn neu() -> Self {
        SystemQuelle {
            system: sysinfo::System::new(),
            pid: sysinfo::get_current_pid().expect("eigene Prozess-ID unbekannt"),
        }
    }
}

impl Messquelle for SystemQuelle {
    fn messen(&mut self) -> Messung {
        const MIB: u64 = 1024 * 1024;
        let (ein, aus) = swap_zaehler();
        self.system.refresh_memory();
        self.system
            .refresh_processes(sysinfo::ProcessesToUpdate::Some(&[self.pid]), true);
        let (rss, cpu) = self
            .system
            .process(self.pid)
            .map_or((0, 0.0), |p| (p.memory(), p.cpu_usage()));
        Messung {
            rss_mb: rss / MIB,
            frei_mb: self.system.available_memory() / MIB,
            swap_mb: self.system.used_swap() / MIB,
            cpu_pct: cpu,
            gesamt_mb: self.system.total_memory() / MIB,
            swap_gesamt_mb: self.system.total_swap() / MIB,
            kerne: std::thread::available_parallelism().map_or(1, |n| n.get() as u32),
            swap_ein_mb: ein,
            swap_aus_mb: aus,
        }
    }
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
                        gesamt_mb: m.gesamt_mb,
                        swap_gesamt_mb: m.swap_gesamt_mb,
                        kerne: m.kerne,
                        swap_ein_mb: m.swap_ein_mb,
                        swap_aus_mb: m.swap_aus_mb,
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

/// Ein Messpunkt samt der Skala, an der man ihn misst (für Balkenanzeigen).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Messpunkt {
    pub t_ms: u64,
    pub rss_mb: u64,
    pub frei_mb: u64,
    pub swap_mb: u64,
    pub cpu_pct: f32,
    pub gesamt_mb: u64,
    pub swap_gesamt_mb: u64,
    pub kerne: u32,
    /// Swap-Rate seit der vorigen Probe in MB/s — zeigt, ob der Rechner gerade thrasht.
    pub swap_ein_mb_s: f32,
    pub swap_aus_mb_s: f32,
}

/// Die Antworten, auf die es bei einem Hänger ankommt.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Zusammenfassung {
    pub spitze_rss_mb: u64,
    pub spitze_rss_t_ms: u64,
    /// Wenig freier Speicher ist der Vorbote des Swappens und damit des Freezes.
    pub min_frei_mb: u64,
    pub max_swap_mb: u64,
    /// Phase, in der der Strom endet — bei einem Freeze die, in der es passiert ist.
    pub letzte_phase: Option<String>,
    /// Zeit des letzten Eintrags: das letzte Lebenszeichen des Prozesses.
    pub ende_t_ms: u64,
    /// Die jüngste Probe: der aktuelle Stand, solange der Job läuft.
    pub aktuell: Option<Messpunkt>,
    pub spitze_cpu_pct: f32,
    pub spitze_swap_ein_mb_s: f32,
    pub spitze_swap_aus_mb_s: f32,
    /// Im Lauf insgesamt ein- bzw. ausgelagert (MiB).
    pub swap_eingelagert_mb: u64,
    pub swap_ausgelagert_mb: u64,
    /// Werte je Phase, in der Reihenfolge ihres Auftretens.
    pub phasen: Vec<PhasenWerte>,
    /// Die sd.cpp-Stufen (laden, text_encoder, vae_encode, sampling, vae_decode),
    /// nach Beginn geordnet. Eine Phase wie `erzeugen` enthält sie alle.
    pub stufen: Vec<StufenWerte>,
}

/// Spitzen innerhalb einer Phase (von ihrer Marke bis zur nächsten bzw. zum Ende).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PhasenWerte {
    pub name: String,
    pub von_t_ms: u64,
    pub bis_t_ms: u64,
    /// Anzahl der Proben in dieser Phase; bei 0 sind die Werte unten bedeutungslos.
    pub proben: usize,
    pub spitze_rss_mb: u64,
    pub min_frei_mb: u64,
    pub max_swap_mb: u64,
}

/// Eine Stufe von sd.cpp, über alle ihre Meldungen zusammengefasst (`laden` meldet
/// sich mehrfach). Speicher und Swap gelten für die Zeitfenster, in denen sie lief.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StufenWerte {
    pub name: String,
    pub anzahl: usize,
    pub dauer_s: f32,
    pub von_t_ms: u64,
    pub bis_t_ms: u64,
    pub proben: usize,
    pub spitze_rss_mb: u64,
    pub min_frei_mb: u64,
    pub max_swap_mb: u64,
    pub swap_eingelagert_mb: u64,
    pub swap_ausgelagert_mb: u64,
}

fn stufen_werte(strom: &[Eintrag]) -> Vec<StufenWerte> {
    // (t_ms, rss, frei, swap, ein, aus) jeder Probe.
    let proben: Vec<(u64, u64, u64, u64, u64, u64)> = strom
        .iter()
        .filter_map(|e| match e {
            Eintrag::Probe {
                t_ms,
                rss_mb,
                frei_mb,
                swap_mb,
                swap_ein_mb,
                swap_aus_mb,
                ..
            } => Some((
                *t_ms,
                *rss_mb,
                *frei_mb,
                *swap_mb,
                *swap_ein_mb,
                *swap_aus_mb,
            )),
            _ => None,
        })
        .collect();
    let mut stufen: Vec<StufenWerte> = Vec::new();
    for eintrag in strom {
        let Eintrag::Stufe {
            t_ms: bis,
            name,
            dauer_s,
        } = eintrag
        else {
            continue;
        };
        let von = bis.saturating_sub((dauer_s * 1000.0).round() as u64);
        let idx = stufen
            .iter()
            .position(|s| &s.name == name)
            .unwrap_or_else(|| {
                stufen.push(StufenWerte {
                    name: name.clone(),
                    anzahl: 0,
                    dauer_s: 0.0,
                    von_t_ms: von,
                    bis_t_ms: *bis,
                    proben: 0,
                    spitze_rss_mb: 0,
                    min_frei_mb: u64::MAX,
                    max_swap_mb: 0,
                    swap_eingelagert_mb: 0,
                    swap_ausgelagert_mb: 0,
                });
                stufen.len() - 1
            });
        let s = &mut stufen[idx];
        s.anzahl += 1;
        s.dauer_s += dauer_s;
        s.von_t_ms = s.von_t_ms.min(von);
        s.bis_t_ms = s.bis_t_ms.max(*bis);
        let im_fenster: Vec<_> = proben
            .iter()
            .filter(|p| p.0 >= von && p.0 <= *bis)
            .collect();
        for p in &im_fenster {
            s.proben += 1;
            s.spitze_rss_mb = s.spitze_rss_mb.max(p.1);
            s.min_frei_mb = s.min_frei_mb.min(p.2);
            s.max_swap_mb = s.max_swap_mb.max(p.3);
        }
        if let (Some(erste), Some(letzte)) = (im_fenster.first(), im_fenster.last()) {
            s.swap_eingelagert_mb += letzte.4.saturating_sub(erste.4);
            s.swap_ausgelagert_mb += letzte.5.saturating_sub(erste.5);
        }
    }
    for s in &mut stufen {
        if s.proben == 0 {
            s.min_frei_mb = 0;
        }
    }
    stufen.sort_by_key(|s| s.von_t_ms);
    stufen
}

pub fn zusammenfassen(strom: &[Eintrag]) -> Zusammenfassung {
    let mut z = Zusammenfassung {
        spitze_rss_mb: 0,
        spitze_rss_t_ms: 0,
        min_frei_mb: u64::MAX,
        max_swap_mb: 0,
        letzte_phase: None,
        ende_t_ms: 0,
        aktuell: None,
        spitze_cpu_pct: 0.0,
        spitze_swap_ein_mb_s: 0.0,
        spitze_swap_aus_mb_s: 0.0,
        swap_eingelagert_mb: 0,
        swap_ausgelagert_mb: 0,
        phasen: Vec::new(),
        stufen: stufen_werte(strom),
    };
    // (t_ms, ein, aus) der ersten und der vorigen Probe, für Summe und Rate.
    let mut erste_probe: Option<(u64, u64)> = None;
    let mut vorige_probe: Option<(u64, u64, u64)> = None;
    for eintrag in strom {
        match eintrag {
            Eintrag::Phase { t_ms, name } => {
                if let Some(vorige) = z.phasen.last_mut() {
                    vorige.bis_t_ms = *t_ms;
                }
                z.phasen.push(PhasenWerte {
                    name: name.clone(),
                    von_t_ms: *t_ms,
                    bis_t_ms: *t_ms,
                    proben: 0,
                    spitze_rss_mb: 0,
                    min_frei_mb: u64::MAX,
                    max_swap_mb: 0,
                });
                z.letzte_phase = Some(name.clone());
                z.ende_t_ms = *t_ms;
            }
            Eintrag::Probe {
                t_ms,
                rss_mb,
                frei_mb,
                swap_mb,
                cpu_pct,
                gesamt_mb,
                swap_gesamt_mb,
                kerne,
                swap_ein_mb,
                swap_aus_mb,
            } => {
                let (ein_s, aus_s) = match vorige_probe {
                    Some((t0, ein0, aus0)) if *t_ms > t0 => {
                        let dt = (*t_ms - t0) as f32 / 1000.0;
                        (
                            swap_ein_mb.saturating_sub(ein0) as f32 / dt,
                            swap_aus_mb.saturating_sub(aus0) as f32 / dt,
                        )
                    }
                    _ => (0.0, 0.0),
                };
                let (ein0, aus0) = *erste_probe.get_or_insert((*swap_ein_mb, *swap_aus_mb));
                z.swap_eingelagert_mb = swap_ein_mb.saturating_sub(ein0);
                z.swap_ausgelagert_mb = swap_aus_mb.saturating_sub(aus0);
                z.spitze_swap_ein_mb_s = z.spitze_swap_ein_mb_s.max(ein_s);
                z.spitze_swap_aus_mb_s = z.spitze_swap_aus_mb_s.max(aus_s);
                vorige_probe = Some((*t_ms, *swap_ein_mb, *swap_aus_mb));
                z.aktuell = Some(Messpunkt {
                    t_ms: *t_ms,
                    rss_mb: *rss_mb,
                    frei_mb: *frei_mb,
                    swap_mb: *swap_mb,
                    cpu_pct: *cpu_pct,
                    gesamt_mb: *gesamt_mb,
                    swap_gesamt_mb: *swap_gesamt_mb,
                    kerne: *kerne,
                    swap_ein_mb_s: ein_s,
                    swap_aus_mb_s: aus_s,
                });
                z.spitze_cpu_pct = z.spitze_cpu_pct.max(*cpu_pct);
                if let Some(p) = z.phasen.last_mut() {
                    p.bis_t_ms = *t_ms;
                    p.proben += 1;
                    p.spitze_rss_mb = p.spitze_rss_mb.max(*rss_mb);
                    p.min_frei_mb = p.min_frei_mb.min(*frei_mb);
                    p.max_swap_mb = p.max_swap_mb.max(*swap_mb);
                }
                if *rss_mb > z.spitze_rss_mb {
                    z.spitze_rss_mb = *rss_mb;
                    z.spitze_rss_t_ms = *t_ms;
                }
                z.min_frei_mb = z.min_frei_mb.min(*frei_mb);
                z.max_swap_mb = z.max_swap_mb.max(*swap_mb);
                z.ende_t_ms = *t_ms;
            }
            Eintrag::Stufe { t_ms, .. } | Eintrag::Kontext { t_ms, .. } => z.ende_t_ms = *t_ms,
        }
    }
    if z.min_frei_mb == u64::MAX {
        z.min_frei_mb = 0;
    }
    for p in &mut z.phasen {
        if p.min_frei_mb == u64::MAX {
            p.min_frei_mb = 0;
        }
    }
    z
}

fn zeit(ms: u64) -> String {
    format!(
        "{}:{:02}.{}",
        ms / 60_000,
        (ms / 1000) % 60,
        (ms % 1000) / 100
    )
}

/// Lesbarer Bericht für die Kommandozeile (`metrics-report`).
pub fn bericht(strom: &[Eintrag]) -> String {
    use std::fmt::Write as _;
    let z = zusammenfassen(strom);
    let mut text = String::new();
    for eintrag in strom {
        if let Eintrag::Kontext { kontext: k, .. } = eintrag {
            let refs: Vec<String> = k
                .referenzen
                .iter()
                .map(|(b, h)| format!("{b}x{h}"))
                .collect();
            let _ = writeln!(
                text,
                "Bild: {} {}x{}, {} Schritte, Gewichte {}, Threads {}",
                k.preset,
                k.breite,
                k.hoehe,
                k.steps,
                k.wtype.as_deref().unwrap_or("wie Datei"),
                k.threads
            );
            let _ = writeln!(
                text,
                "  Referenzen: {} (Obergrenze {})   mmap {}, Flash-Attention {}, VAE-Tiling {}",
                if refs.is_empty() {
                    "keine".into()
                } else {
                    refs.join(", ")
                },
                k.ref_max_px
                    .map_or("sd.cpp-Standard (1 MP)".to_string(), |px| format!(
                        "{px} px"
                    )),
                k.mmap,
                k.flash_attention,
                k.vae_tiling
            );
        }
    }
    let _ = writeln!(
        text,
        "\n{:<14} {:>8} {:>8} {:>10} {:>10} {:>9}",
        "Phase", "von", "bis", "RSS MiB", "frei MiB", "Swap MiB"
    );
    // Ohne Probe in einer Phase wäre "0" eine Behauptung; deshalb "–".
    let zahl = |proben: usize, wert: u64| {
        if proben == 0 {
            "–".to_string()
        } else {
            wert.to_string()
        }
    };
    for p in &z.phasen {
        let _ = writeln!(
            text,
            "{:<14} {:>8} {:>8} {:>10} {:>10} {:>9}",
            p.name,
            zeit(p.von_t_ms),
            zeit(p.bis_t_ms),
            zahl(p.proben, p.spitze_rss_mb),
            zahl(p.proben, p.min_frei_mb),
            zahl(p.proben, p.max_swap_mb)
        );
    }
    if !z.stufen.is_empty() {
        let _ = writeln!(
            text,
            "\nStufen (sd.cpp, innerhalb von 'erzeugen')\n{:<14} {:>8} {:>10} {:>10} {:>9} {:>13}",
            "Stufe", "Dauer", "RSS MiB", "frei MiB", "Swap MiB", "ausgelagert"
        );
        for st in &z.stufen {
            let _ = writeln!(
                text,
                "{:<14} {:>8} {:>10} {:>10} {:>9} {:>13}",
                st.name,
                format!("{:.1} s", st.dauer_s),
                zahl(st.proben, st.spitze_rss_mb),
                zahl(st.proben, st.min_frei_mb),
                zahl(st.proben, st.max_swap_mb),
                zahl(st.proben, st.swap_ausgelagert_mb)
            );
        }
    }
    let _ = writeln!(
        text,
        "\nSpitze RSS {} MiB bei {}, wenigster freier Speicher {} MiB, Swap bis {} MiB",
        z.spitze_rss_mb,
        zeit(z.spitze_rss_t_ms),
        z.min_frei_mb,
        z.max_swap_mb
    );
    let _ = writeln!(
        text,
        "Swap im Lauf: {} MiB ein, {} MiB aus (Spitze {:.0} MB/s ein, {:.0} MB/s aus)",
        z.swap_eingelagert_mb,
        z.swap_ausgelagert_mb,
        z.spitze_swap_ein_mb_s,
        z.spitze_swap_aus_mb_s
    );
    if let Some(phase) = &z.letzte_phase {
        let _ = writeln!(text, "Ende in Phase '{phase}' bei {}", zeit(z.ende_t_ms));
    }
    text
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
            gesamt_mb: 16384,
            swap_gesamt_mb: 4096,
            kerne: 8,
            swap_ein_mb: 0,
            swap_aus_mb: 0,
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
            ref_max_px: None,
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
                gesamt_mb: 16384,
                swap_gesamt_mb: 4096,
                kerne: 8,
                swap_ein_mb: 0,
                swap_aus_mb: 0,
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

    #[test]
    fn systemquelle_misst_den_eigenen_prozess() {
        let mut quelle = SystemQuelle::neu();
        // Die CPU-Last braucht zwei Messungen mit Abstand.
        quelle.messen();
        std::thread::sleep(std::time::Duration::from_millis(250));
        let m = quelle.messen();
        assert!(m.rss_mb > 0, "{m:?}");
        assert!(m.frei_mb > 0, "{m:?}");
        assert!(m.cpu_pct.is_finite() && m.cpu_pct >= 0.0, "{m:?}");
    }

    #[test]
    fn sd_log_geht_an_das_gesetzte_ziel_und_nach_dem_abraeumen_nirgends_hin() {
        let (_dir, pfad, m) = feste_uhr(7);
        sd_log_ziel_setzen(Some(m));
        sd_log_weiterleiten("x.cpp:1 - sampling completed, taking 1.00s");
        sd_log_ziel_setzen(None);
        sd_log_weiterleiten("x.cpp:1 - sampling completed, taking 2.00s");
        let namen: Vec<f32> = eintraege_lesen(&pfad)
            .unwrap()
            .into_iter()
            .filter_map(|e| match e {
                Eintrag::Stufe { dauer_s, .. } => Some(dauer_s),
                _ => None,
            })
            .collect();
        assert_eq!(namen, [1.0]);
    }

    fn probe(t_ms: u64, rss_mb: u64, frei_mb: u64, swap_mb: u64) -> Eintrag {
        Eintrag::Probe {
            t_ms,
            rss_mb,
            frei_mb,
            swap_mb,
            cpu_pct: 100.0,
            gesamt_mb: 16384,
            swap_gesamt_mb: 4096,
            kerne: 8,
            swap_ein_mb: 0,
            swap_aus_mb: 0,
        }
    }

    fn phase(t_ms: u64, name: &str) -> Eintrag {
        Eintrag::Phase {
            t_ms,
            name: name.into(),
        }
    }

    #[test]
    fn zusammenfassung_nennt_spitzen_letzte_phase_und_letztes_lebenszeichen() {
        let strom = [
            phase(0, "referenz"),
            probe(100, 5000, 900, 0),
            phase(200, "erzeugen"),
            probe(300, 9000, 150, 2000),
            probe(400, 8000, 300, 4000),
        ];
        let z = zusammenfassen(&strom);
        assert_eq!((z.spitze_rss_mb, z.spitze_rss_t_ms), (9000, 300));
        assert_eq!(z.min_frei_mb, 150);
        assert_eq!(z.max_swap_mb, 4000);
        assert_eq!(z.letzte_phase.as_deref(), Some("erzeugen"));
        assert_eq!(z.ende_t_ms, 400);
    }

    #[test]
    fn zusammenfassung_schluesselt_die_werte_nach_phasen_auf() {
        let strom = [
            phase(0, "referenz"),
            probe(100, 5000, 900, 0),
            phase(200, "erzeugen"),
            probe(300, 9000, 150, 2000),
            probe(400, 8000, 300, 4000),
        ];
        let phasen = zusammenfassen(&strom).phasen;
        assert_eq!(phasen.len(), 2);
        assert_eq!(phasen[0].name, "referenz");
        assert_eq!((phasen[0].von_t_ms, phasen[0].bis_t_ms), (0, 200));
        assert_eq!(
            (
                phasen[0].spitze_rss_mb,
                phasen[0].min_frei_mb,
                phasen[0].max_swap_mb
            ),
            (5000, 900, 0)
        );
        assert_eq!(phasen[1].name, "erzeugen");
        assert_eq!((phasen[1].von_t_ms, phasen[1].bis_t_ms), (200, 400));
        assert_eq!(
            (
                phasen[1].spitze_rss_mb,
                phasen[1].min_frei_mb,
                phasen[1].max_swap_mb
            ),
            (9000, 150, 4000)
        );
    }

    #[test]
    fn bericht_nennt_kontext_phasen_und_den_ort_des_endes() {
        let strom = [
            Eintrag::Kontext {
                t_ms: 0,
                kontext: Kontext {
                    preset: "klein-9b".into(),
                    wtype: None,
                    breite: 768,
                    hoehe: 1536,
                    steps: 4,
                    referenzen: vec![(1024, 1024)],
                    mmap: false,
                    flash_attention: false,
                    vae_tiling: false,
                    threads: 8,
                    ref_max_px: Some(512),
                },
            },
            phase(0, "referenz"),
            probe(100, 5000, 900, 0),
            phase(200, "erzeugen"),
            probe(300, 9000, 150, 2000),
        ];
        let text = bericht(&strom);
        assert!(
            text.contains("klein-9b") && text.contains("768x1536"),
            "{text}"
        );
        assert!(text.contains("Referenzen: 1024x1024"), "{text}");
        assert!(text.contains("erzeugen") && text.contains("9000"), "{text}");
        assert!(
            text.contains("Ende in Phase 'erzeugen' bei 0:00.3"),
            "{text}"
        );
    }

    #[test]
    fn systemquelle_nennt_auch_die_bezugsgroessen_der_skala() {
        let m = SystemQuelle::neu().messen();
        assert!(m.gesamt_mb >= m.frei_mb && m.gesamt_mb > 0, "{m:?}");
        assert!(m.kerne >= 1, "{m:?}");
        assert!(m.swap_gesamt_mb >= m.swap_mb, "{m:?}");
    }

    #[test]
    fn zusammenfassung_nennt_den_letzten_messpunkt_samt_skala() {
        let strom = [
            phase(0, "erzeugen"),
            probe(100, 5000, 900, 0),
            probe(400, 8000, 300, 4000),
        ];
        let z = zusammenfassen(&strom);
        let aktuell = z.aktuell.expect("es gab Proben");
        assert_eq!(
            (
                aktuell.t_ms,
                aktuell.rss_mb,
                aktuell.frei_mb,
                aktuell.swap_mb
            ),
            (400, 8000, 300, 4000)
        );
        assert_eq!(
            (aktuell.gesamt_mb, aktuell.swap_gesamt_mb, aktuell.kerne),
            (16384, 4096, 8)
        );
        assert_eq!(z.spitze_cpu_pct, 100.0);
        assert!(zusammenfassen(&[phase(0, "x")]).aktuell.is_none());
    }

    const VM_STAT: &str = "Mach Virtual Memory Statistics: (page size of 16384 bytes)
Pages free:                               12345.
Pages active:                            400000.
Pageins:                                 999999.
Pageouts:                                 55555.
Swapins:                                    128.
Swapouts:                                 64000.
";

    #[test]
    fn vm_stat_liefert_swap_ein_und_auslagerungen_in_mib() {
        // 128 Seiten à 16 KiB = 2 MiB; 64000 Seiten = 1000 MiB.
        assert_eq!(swap_zaehler_aus_vm_stat(VM_STAT), Some((2, 1000)));
    }

    #[test]
    fn proben_aelterer_laeufe_ohne_swap_zaehler_bleiben_lesbar() {
        let alt = r#"{"art":"probe","t_ms":5,"rss_mb":1,"frei_mb":2,"swap_mb":3,"cpu_pct":0.5,"gesamt_mb":10,"swap_gesamt_mb":4,"kerne":8}"#;
        let neu = r#"{"art":"probe","t_ms":5,"rss_mb":1,"frei_mb":2,"swap_mb":3,"cpu_pct":0.5,"gesamt_mb":10,"swap_gesamt_mb":4,"kerne":8,"swap_ein_mb":7,"swap_aus_mb":9}"#;
        let zaehler = |zeile: &str| match serde_json::from_str::<Eintrag>(zeile).unwrap() {
            Eintrag::Probe {
                swap_ein_mb,
                swap_aus_mb,
                ..
            } => (swap_ein_mb, swap_aus_mb),
            andere => panic!("{andere:?}"),
        };
        assert_eq!(zaehler(alt), (0, 0));
        assert_eq!(zaehler(neu), (7, 9));
    }

    fn probe_swap(t_ms: u64, ein: u64, aus: u64) -> Eintrag {
        match probe(t_ms, 1, 1, 1) {
            Eintrag::Probe {
                t_ms,
                rss_mb,
                frei_mb,
                swap_mb,
                cpu_pct,
                gesamt_mb,
                swap_gesamt_mb,
                kerne,
                ..
            } => Eintrag::Probe {
                t_ms,
                rss_mb,
                frei_mb,
                swap_mb,
                cpu_pct,
                gesamt_mb,
                swap_gesamt_mb,
                kerne,
                swap_ein_mb: ein,
                swap_aus_mb: aus,
            },
            _ => unreachable!(),
        }
    }

    #[test]
    fn zusammenfassung_berechnet_die_swap_raten_aus_den_zaehlern() {
        let strom = [
            probe_swap(1000, 0, 100),
            probe_swap(2000, 0, 300),  // 200 MB/s ausgelagert
            probe_swap(3000, 50, 300), // 50 MB/s eingelagert
        ];
        let z = zusammenfassen(&strom);
        let aktuell = z.aktuell.unwrap();
        assert_eq!((aktuell.swap_ein_mb_s, aktuell.swap_aus_mb_s), (50.0, 0.0));
        assert_eq!(
            (z.spitze_swap_ein_mb_s, z.spitze_swap_aus_mb_s),
            (50.0, 200.0)
        );
        assert_eq!((z.swap_eingelagert_mb, z.swap_ausgelagert_mb), (50, 200));
    }

    #[test]
    fn phasen_zaehlen_ihre_proben_damit_leere_nicht_als_null_erscheinen() {
        let strom = [
            phase(0, "referenz"),
            phase(5, "erzeugen"),
            probe(100, 5000, 900, 0),
            probe(200, 5000, 900, 0),
            phase(300, "bild_fertig"),
        ];
        let proben: Vec<usize> = zusammenfassen(&strom)
            .phasen
            .iter()
            .map(|p| p.proben)
            .collect();
        assert_eq!(proben, [0, 2, 0]);
    }

    fn mit_aus(mut e: Eintrag, aus: u64) -> Eintrag {
        if let Eintrag::Probe { swap_aus_mb, .. } = &mut e {
            *swap_aus_mb = aus;
        }
        e
    }

    fn stufe(t_ms: u64, name: &str, dauer_s: f32) -> Eintrag {
        Eintrag::Stufe {
            t_ms,
            name: name.into(),
            dauer_s,
        }
    }

    #[test]
    fn stufen_fassen_dauer_speicher_und_swap_im_zeitfenster_zusammen() {
        let strom = [
            mit_aus(probe(1000, 5000, 900, 0), 100),
            stufe(1200, "laden", 1.2), // Fenster 0–1200 ms
            mit_aus(probe(3000, 9000, 150, 0), 400),
            mit_aus(probe(5000, 8000, 300, 0), 450),
            stufe(5000, "sampling", 3.5), // Fenster 1500–5000 ms
        ];
        let stufen = zusammenfassen(&strom).stufen;
        let namen: Vec<&str> = stufen.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(namen, ["laden", "sampling"], "nach Beginn geordnet");
        let laden = &stufen[0];
        assert_eq!(
            (laden.proben, laden.spitze_rss_mb, laden.swap_ausgelagert_mb),
            (1, 5000, 0)
        );
        let sampling = &stufen[1];
        assert_eq!(sampling.dauer_s, 3.5);
        assert_eq!((sampling.von_t_ms, sampling.bis_t_ms), (1500, 5000));
        assert_eq!(
            (
                sampling.proben,
                sampling.spitze_rss_mb,
                sampling.min_frei_mb
            ),
            (2, 9000, 150)
        );
        assert_eq!(sampling.swap_ausgelagert_mb, 50);
    }

    #[test]
    fn bericht_zeigt_stufen_swap_und_markiert_phasen_ohne_proben() {
        let strom = [
            phase(0, "referenz"),
            phase(5, "erzeugen"),
            mit_aus(probe(1000, 5000, 900, 0), 100),
            stufe(1200, "laden", 1.2),
            mit_aus(probe(3000, 9000, 150, 2000), 400),
            mit_aus(probe(5000, 8000, 300, 4000), 450),
            stufe(5000, "sampling", 3.5),
        ];
        let text = bericht(&strom);
        let zeile = |anfang: &str| {
            text.lines()
                .find(|z| z.starts_with(anfang))
                .unwrap_or_else(|| panic!("{anfang}: {text}"))
        };
        assert!(zeile("referenz").contains('–'), "{text}");
        assert!(text.contains("Stufen (sd.cpp"), "{text}");
        assert!(
            zeile("sampling").contains("3.5 s") && zeile("sampling").contains("9000"),
            "{text}"
        );
        assert!(
            text.contains("350 MiB aus") && text.contains("150 MB/s"),
            "{text}"
        );
    }

    #[test]
    fn bericht_nennt_die_obergrenze_der_referenzen() {
        let mit = |px: Option<u32>| {
            bericht(&[Eintrag::Kontext {
                t_ms: 0,
                kontext: Kontext {
                    preset: "klein-4b".into(),
                    wtype: None,
                    breite: 768,
                    hoehe: 1536,
                    steps: 8,
                    referenzen: vec![(400, 512)],
                    mmap: false,
                    flash_attention: true,
                    vae_tiling: true,
                    threads: 6,
                    ref_max_px: px,
                },
            }])
        };
        assert!(
            mit(Some(512)).contains("Obergrenze 512 px"),
            "{}",
            mit(Some(512))
        );
        assert!(
            mit(None).contains("Obergrenze sd.cpp-Standard (1 MP)"),
            "{}",
            mit(None)
        );
    }
}
