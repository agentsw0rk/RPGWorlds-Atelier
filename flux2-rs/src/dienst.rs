//! Der Dienst hinter der API: Aufträge annehmen, in eine Warteschlange stellen
//! und einen Job nach dem anderen abarbeiten.
//!
//! Es läuft immer genau ein Job zugleich. Ein Lauf braucht Minuten und viel
//! Arbeitsspeicher; zwei gleichzeitige Läufe würden sich nur gegenseitig
//! ausbremsen oder den Speicher sprengen.
//!
//! Jeder Job liegt als `jobs/<id>/job.json` auf der Platte. Nach einem Neustart
//! werden wartende Jobs wieder eingereiht; was gerade lief, gilt als abgebrochen.

use crate::auftrag::{self, Auftrag, Plan, Umgebung};
use crate::job::{BildStatus, Job, JobStatus};
use crate::kind::Kind;
use crate::lauf::{self, Ereignis, Engine, Host};
use crate::listen::{self, Eintrag};
use crate::metriken::{Metriken, Sampler, SystemQuelle};
use crate::referenzen::Uploads;
use anyhow::{anyhow, bail, Context, Result};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Wo der Dienst seine Dateien ablegt und woher er die Listen liest.
#[derive(Debug, Clone)]
pub struct Konfig {
    /// Datenverzeichnis: `jobs/`, `uploads/` und die Batch-Ausgabeordner.
    pub daten: PathBuf,
    /// Projektverzeichnis mit den Listen (`fullbody.txt`, …).
    pub projekt: PathBuf,
    pub host: Host,
}

#[derive(Default)]
struct Zustand {
    jobs: BTreeMap<String, Job>,
    warteschlange: VecDeque<String>,
    beenden: bool,
}

struct Innen {
    konfig: Konfig,
    uploads: Uploads,
    zustand: Mutex<Zustand>,
    /// Weckt den Worker bei neuen Jobs und Wartende bei jeder Änderung.
    signal: Condvar,
    /// Gesetzt, solange der laufende Job abgebrochen werden soll.
    abbruch: AtomicBool,
    /// Abstand des Samplers in ms; 0 = Beobachtung aus.
    metrik_abstand_ms: AtomicU64,
}

/// Der Dienst. Klonen ist billig und teilt den Zustand.
#[derive(Clone)]
pub struct Dienst {
    innen: Arc<Innen>,
}

fn jetzt() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Was `planen` über die Außenwelt wissen muss — gegen die echten Dateien.
struct DateiUmgebung<'a> {
    projekt: &'a Path,
    uploads: &'a Uploads,
    /// Dauerhafter Ausgabeordner der Art, für das Fortsetzen von Batches.
    ausgabe: PathBuf,
}

impl Umgebung for DateiUmgebung<'_> {
    fn liste(&self, kind: Kind) -> Result<Vec<Eintrag>> {
        let pfad = self.projekt.join(listen::standard_liste(kind));
        let text = std::fs::read_to_string(&pfad)
            .with_context(|| format!("Liste {} nicht lesbar", pfad.display()))?;
        listen::lesen(&text)
    }

    fn upload(&self, id: &str) -> Result<PathBuf> {
        self.uploads.pfad(id)
    }

    fn wuerfeln(&mut self) -> i64 {
        crate::zufall::seed()
    }

    fn vorhanden(&self, datei: &str) -> bool {
        self.ausgabe.join(datei).is_file()
    }
}

impl Dienst {
    /// Öffnet das Datenverzeichnis und lädt gespeicherte Jobs.
    pub fn oeffnen(konfig: Konfig) -> Result<Dienst> {
        std::fs::create_dir_all(konfig.daten.join("jobs"))?;
        let uploads = Uploads::neu(konfig.daten.join("uploads"))?;
        let mut zustand = Zustand::default();

        for eintrag in std::fs::read_dir(konfig.daten.join("jobs"))? {
            let pfad = eintrag?.path().join("job.json");
            let Ok(text) = std::fs::read_to_string(&pfad) else { continue };
            let mut job: Job = match serde_json::from_str(&text) {
                Ok(job) => job,
                Err(e) => {
                    eprintln!("Job {} übersprungen: {e}", pfad.display());
                    continue;
                }
            };
            if job.status == JobStatus::Laeuft {
                // Der Prozess ist mitten im Lauf gestorben; das Bild in Arbeit
                // ist verloren, die fertigen bleiben.
                for bild in &mut job.bilder {
                    if matches!(bild.status, BildStatus::Laeuft | BildStatus::Wartend) {
                        bild.status = BildStatus::Abgebrochen;
                    }
                }
                job.status = JobStatus::Abgebrochen;
                job.beendet = Some(jetzt());
                job.meldung = "Dienst wurde neu gestartet".into();
                speichern(&konfig.daten, &job)?;
            }
            zustand.jobs.insert(job.id.clone(), job);
        }
        let mut wartend: Vec<&Job> = zustand.jobs.values().filter(|j| j.status == JobStatus::Wartend).collect();
        wartend.sort_by_key(|j| (j.erstellt, j.id.clone()));
        zustand.warteschlange = wartend.iter().map(|j| j.id.clone()).collect();

        Ok(Dienst {
            innen: Arc::new(Innen {
                konfig,
                uploads,
                zustand: Mutex::new(zustand),
                signal: Condvar::new(),
                abbruch: AtomicBool::new(false),
                metrik_abstand_ms: AtomicU64::new(0),
            }),
        })
    }

    fn sperre(&self) -> MutexGuard<'_, Zustand> {
        self.innen.zustand.lock().expect("Zustand vergiftet")
    }

    pub fn uploads(&self) -> &Uploads {
        &self.innen.uploads
    }

    /// Plant einen Auftrag, ohne ihn einzureihen — das Gegenstück zu `-n` der
    /// Skripte: man sieht Prompts, Seeds und Dateinamen, bevor Rechenzeit fließt.
    /// Ein Seed `-1` wird dabei gewürfelt; der echte Job würfelt neu.
    pub fn vorschau(&self, auftrag: &Auftrag) -> Result<Plan> {
        let ausgabe_art = auftrag.kind.map(listen::standard_ausgabeordner).unwrap_or("");
        let mut umgebung = DateiUmgebung {
            projekt: &self.innen.konfig.projekt,
            uploads: &self.innen.uploads,
            ausgabe: self.innen.konfig.daten.join(ausgabe_art),
        };
        auftrag::planen(auftrag, &mut umgebung)
    }

    /// Prüft und plant den Auftrag und stellt ihn in die Warteschlange.
    /// Fehler im Auftrag kommen hier, nicht erst beim Lauf.
    pub fn einreichen(&self, auftrag: Auftrag) -> Result<Job> {
        let plan = self.vorschau(&auftrag)?;

        let id = crate::zufall::hex(6);
        let ausgabe = if plan.dauerhaft {
            listen::standard_ausgabeordner(plan.kind).to_string()
        } else {
            format!("jobs/{id}/bilder")
        };
        let job = Job::neu(id.clone(), auftrag, plan, ausgabe, jetzt());
        speichern(&self.innen.konfig.daten, &job)?;

        let mut zustand = self.sperre();
        zustand.jobs.insert(id.clone(), job.clone());
        zustand.warteschlange.push_back(id);
        self.innen.signal.notify_all();
        Ok(job)
    }

    pub fn job(&self, id: &str) -> Option<Job> {
        self.sperre().jobs.get(id).cloned()
    }

    /// Alle Jobs, neueste zuerst.
    pub fn jobs(&self) -> Vec<Job> {
        let mut jobs: Vec<Job> = self.sperre().jobs.values().cloned().collect();
        jobs.sort_by(|a, b| (b.erstellt, &b.id).cmp(&(a.erstellt, &a.id)));
        jobs
    }

    /// Wartet, bis der Job beendet ist, höchstens `frist`. Gibt den dann
    /// aktuellen Stand zurück (auch wenn er noch nicht beendet ist).
    pub fn abwarten(&self, id: &str, frist: Duration) -> Option<Job> {
        let ende = std::time::Instant::now() + frist;
        let mut zustand = self.sperre();
        loop {
            let job = zustand.jobs.get(id)?;
            if job.status.ist_beendet() {
                return Some(job.clone());
            }
            let rest = ende.saturating_duration_since(std::time::Instant::now());
            if rest.is_zero() {
                return Some(job.clone());
            }
            zustand = self.innen.signal.wait_timeout(zustand, rest).expect("Zustand vergiftet").0;
        }
    }

    /// Bricht einen Job ab. Ein wartender Job wird sofort beendet; bei einem
    /// laufenden greift der Abbruch nach dem aktuellen Bild.
    pub fn abbrechen(&self, id: &str) -> Result<Job> {
        let mut zustand = self.sperre();
        let job = zustand.jobs.get_mut(id).ok_or_else(|| anyhow!("Job '{id}' gibt es nicht."))?;
        match job.status {
            JobStatus::Wartend => {
                for bild in &mut job.bilder {
                    if bild.status == BildStatus::Wartend {
                        bild.status = BildStatus::Abgebrochen;
                    }
                }
                job.status = JobStatus::Abgebrochen;
                job.beendet = Some(jetzt());
                job.meldung = "abgebrochen".into();
                let job = job.clone();
                zustand.warteschlange.retain(|w| w != id);
                speichern(&self.innen.konfig.daten, &job)?;
                self.innen.signal.notify_all();
                Ok(job)
            }
            JobStatus::Laeuft => {
                job.abbruch_angefordert = true;
                job.meldung = "Abbruch angefordert — nach dem aktuellen Bild".into();
                self.innen.abbruch.store(true, Ordering::Relaxed);
                let job = job.clone();
                speichern(&self.innen.konfig.daten, &job)?;
                Ok(job)
            }
            _ => bail!("Job '{id}' ist schon beendet."),
        }
    }

    /// Entfernt einen beendeten Job samt seinen Bildern. Bilder aus Listen
    /// liegen im dauerhaften Ordner der Art und bleiben erhalten.
    pub fn entfernen(&self, id: &str) -> Result<()> {
        let mut zustand = self.sperre();
        let job = zustand.jobs.get(id).ok_or_else(|| anyhow!("Job '{id}' gibt es nicht."))?;
        if !job.status.ist_beendet() {
            bail!("Job '{id}' ist noch nicht beendet — erst abbrechen.");
        }
        zustand.jobs.remove(id);
        let ordner = self.innen.konfig.daten.join("jobs").join(id);
        let _ = std::fs::remove_dir_all(ordner);
        Ok(())
    }

    /// Pfad zu einer Bilddatei des Jobs. Nur Dateien, die der Job selbst
    /// gemeldet hat — kein freier Zugriff aufs Dateisystem über den Namen.
    pub fn bild_pfad(&self, id: &str, datei: &str) -> Result<PathBuf> {
        let zustand = self.sperre();
        let job = zustand.jobs.get(id).ok_or_else(|| anyhow!("Job '{id}' gibt es nicht."))?;
        let bekannt = job
            .bilder
            .iter()
            .any(|b| b.datei.as_deref() == Some(datei) || b.roh.as_deref() == Some(datei));
        if !bekannt {
            bail!("Bild '{datei}' gehört nicht zu Job '{id}'.");
        }
        let pfad = self.innen.konfig.daten.join(&job.ausgabe).join(datei);
        if !pfad.is_file() {
            bail!("Bild '{datei}' ist noch nicht fertig.");
        }
        Ok(pfad)
    }

    /// Fertige Bilder im dauerhaften Ordner einer Art (Batch-Ergebnisse), ohne
    /// Rohbilder, alphabetisch.
    pub fn bibliothek(&self, kind: Kind) -> Vec<String> {
        let ordner = self.innen.konfig.daten.join(listen::standard_ausgabeordner(kind));
        let mut dateien: Vec<String> = std::fs::read_dir(ordner)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.ends_with(".png") && !n.ends_with(".raw.png") && !n.starts_with('.'))
            .collect();
        dateien.sort();
        dateien
    }

    /// Pfad zu einem Bild der Bibliothek. Der Name darf nur aus harmlosen
    /// Zeichen bestehen: so führt er garantiert nicht aus dem Ordner heraus.
    pub fn bibliothek_pfad(&self, kind: Kind, datei: &str) -> Result<PathBuf> {
        let harmlos = datei.ends_with(".png")
            && !datei.starts_with('.')
            && datei.len() <= 140
            && datei.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if !harmlos {
            bail!("Bild '{datei}' gibt es nicht.");
        }
        let pfad = self.innen.konfig.daten.join(listen::standard_ausgabeordner(kind)).join(datei);
        if !pfad.is_file() {
            bail!("Bild '{datei}' gibt es nicht.");
        }
        Ok(pfad)
    }

    /// Hält den Worker an; er beendet noch das laufende Bild.
    pub fn beenden(&self) {
        self.sperre().beenden = true;
        self.innen.abbruch.store(true, Ordering::Relaxed);
        self.innen.signal.notify_all();
    }

    /// Startet den Worker in einem eigenen Thread. Die Engine wird erst dort
    /// gebaut: sie hält native Zeiger und muss nicht zwischen Threads wandern.
    pub fn worker_starten<F>(&self, engine_bauen: F) -> std::thread::JoinHandle<()>
    where
        F: FnOnce() -> Box<dyn Engine> + Send + 'static,
    {
        let dienst = self.clone();
        std::thread::spawn(move || {
            let mut engine = engine_bauen();
            dienst.worker(engine.as_mut());
        })
    }

    fn naechster(&self) -> Option<Job> {
        let mut zustand = self.sperre();
        loop {
            if zustand.beenden {
                return None;
            }
            if let Some(id) = zustand.warteschlange.pop_front() {
                // Zwischen Einreihen und Abholen kann der Job abgebrochen worden sein.
                let Some(job) = zustand.jobs.get_mut(&id).filter(|j| j.status == JobStatus::Wartend) else {
                    continue;
                };
                job.status = JobStatus::Laeuft;
                job.gestartet = Some(jetzt());
                job.meldung = "startet".into();
                self.innen.abbruch.store(false, Ordering::Relaxed);
                let job = job.clone();
                let _ = speichern(&self.innen.konfig.daten, &job);
                self.innen.signal.notify_all();
                return Some(job);
            }
            zustand = self.innen.signal.wait(zustand).expect("Zustand vergiftet");
        }
    }

    /// Schaltet die Beobachtung ein (`Some(Abstand)`) oder aus (`None`). Wirkt ab
    /// dem nächsten Job. Standard: aus.
    pub fn metriken_einschalten(&self, abstand: Option<Duration>) {
        let ms = abstand.map_or(0, |d| d.as_millis().max(1) as u64);
        self.innen.metrik_abstand_ms.store(ms, Ordering::Relaxed);
    }

    /// Pfad zum Metrikstrom eines Jobs, wenn es ihn gibt.
    pub fn metriken_pfad(&self, id: &str) -> Option<PathBuf> {
        self.sperre().jobs.get(id)?;
        let pfad = self.innen.konfig.daten.join("jobs").join(id).join("metrics.jsonl");
        pfad.is_file().then_some(pfad)
    }

    /// Öffnet den Metrikstrom des Jobs und startet den Sampler — wenn eingeschaltet.
    fn beobachtung_starten(&self, id: &str) -> Option<(Metriken, Sampler)> {
        let ms = self.innen.metrik_abstand_ms.load(Ordering::Relaxed);
        if ms == 0 {
            return None;
        }
        let pfad = self.innen.konfig.daten.join("jobs").join(id).join("metrics.jsonl");
        let metriken = match Metriken::oeffnen(&pfad) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Beobachtung aus: {e:#}");
                return None;
            }
        };
        let sampler = Sampler::starten(metriken.clone(), Box::new(SystemQuelle::neu()), Duration::from_millis(ms));
        Some((metriken, sampler))
    }

    fn worker(&self, engine: &mut dyn Engine) {
        while let Some(job) = self.naechster() {
            let ausgabe = self.innen.konfig.daten.join(&job.ausgabe);
            let beobachtung = self.beobachtung_starten(&job.id);
            let ergebnis = lauf::ausfuehren_mit_metriken(
                &job.plan,
                &ausgabe,
                &self.innen.konfig.host,
                engine,
                &self.innen.abbruch,
                &mut |ereignis| self.anwenden(&job.id, ereignis),
                beobachtung.as_ref().map(|(m, _)| m),
            );
            if let Some((_, sampler)) = beobachtung {
                sampler.stoppen();
            }
            self.abschliessen(&job.id, ergebnis.err().map(|e| format!("{e:#}")));
        }
    }

    /// Überträgt ein Ereignis des Laufs in den Jobzustand und speichert ihn.
    fn anwenden(&self, id: &str, ereignis: Ereignis) {
        let mut zustand = self.sperre();
        let Some(job) = zustand.jobs.get_mut(id) else { return };
        match ereignis {
            Ereignis::Meldung(text) => job.meldung = text,
            Ereignis::BildStart(i) => {
                if let Some(b) = job.bilder.get_mut(i) {
                    b.status = BildStatus::Laeuft;
                }
            }
            Ereignis::BildFertig { index, datei, roh, dauer_s } => {
                if let Some(b) = job.bilder.get_mut(index) {
                    b.status = BildStatus::Fertig;
                    b.datei = Some(datei);
                    b.roh = roh;
                    b.dauer_s = Some(dauer_s);
                }
            }
            Ereignis::BildFehler { index, fehler } => {
                if let Some(b) = job.bilder.get_mut(index) {
                    b.status = BildStatus::Fehlgeschlagen;
                    b.fehler = Some(fehler);
                }
            }
            Ereignis::BildAbgebrochen(index) => {
                if let Some(b) = job.bilder.get_mut(index) {
                    b.status = BildStatus::Abgebrochen;
                }
            }
        }
        // Unter der Sperre speichern: wer den neuen Stand im Speicher sieht,
        // findet ihn auch auf der Platte. Sonst liest ein Neustart zwischen
        // beidem einen veralteten Zustand ein.
        if let Err(e) = speichern(&self.innen.konfig.daten, job) {
            eprintln!("Job {id} nicht speicherbar: {e:#}");
        }
        self.innen.signal.notify_all();
    }

    fn abschliessen(&self, id: &str, fehler: Option<String>) {
        let mut zustand = self.sperre();
        let Some(job) = zustand.jobs.get_mut(id) else { return };
        job.fehler = fehler;
        // Bilder, die nie drankamen (Job scheiterte vor dem Start), nicht als
        // "wartend" stehen lassen.
        let gescheitert = job.fehler.is_some();
        for bild in &mut job.bilder {
            if matches!(bild.status, BildStatus::Wartend | BildStatus::Laeuft) {
                bild.status = if gescheitert { BildStatus::Fehlgeschlagen } else { BildStatus::Abgebrochen };
                if gescheitert {
                    bild.fehler = job.fehler.clone();
                }
            }
        }
        job.status = job.endstatus();
        job.beendet = Some(jetzt());
        job.meldung = match job.status {
            JobStatus::Fertig => "fertig".into(),
            JobStatus::Abgebrochen => "abgebrochen".into(),
            _ => "fehlgeschlagen".into(),
        };
        // Unter der Sperre speichern: wer den neuen Stand im Speicher sieht,
        // findet ihn auch auf der Platte. Sonst liest ein Neustart zwischen
        // beidem einen veralteten Zustand ein.
        if let Err(e) = speichern(&self.innen.konfig.daten, job) {
            eprintln!("Job {id} nicht speicherbar: {e:#}");
        }
        self.innen.signal.notify_all();
    }
}

/// Schreibt `jobs/<id>/job.json` atomar: erst eine temporäre Datei, dann umbenennen.
fn speichern(daten: &Path, job: &Job) -> Result<()> {
    let ordner = daten.join("jobs").join(&job.id);
    std::fs::create_dir_all(&ordner)?;
    let tmp = ordner.join("job.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(job)?)?;
    std::fs::rename(&tmp, ordner.join("job.json"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lauf::Erzeugung;
    use crate::modelle::Modellsatz;
    use std::sync::atomic::AtomicUsize;

    /// Engine, die ein kleines Bild schreibt und mitzählt — oder auf Wunsch wartet.
    struct Attrappe {
        aufrufe: Arc<AtomicUsize>,
        sperre: Option<Arc<(Mutex<bool>, Condvar)>>,
        scheitern: bool,
    }

    impl Engine for Attrappe {
        fn erzeuge(&mut self, _: &Modellsatz, _: &Host, e: &Erzeugung) -> Result<()> {
            self.aufrufe.fetch_add(1, Ordering::SeqCst);
            if let Some(sperre) = &self.sperre {
                let (frei, signal) = &**sperre;
                let mut frei = frei.lock().unwrap();
                while !*frei {
                    frei = signal.wait(frei).unwrap();
                }
            }
            if self.scheitern {
                anyhow::bail!("kaputt");
            }
            image::RgbImage::from_pixel(16, 16, image::Rgb([9, 9, 9])).save(e.ziel)?;
            Ok(())
        }
    }

    struct Aufbau {
        _dir: tempfile::TempDir,
        konfig: Konfig,
        aufrufe: Arc<AtomicUsize>,
    }

    fn aufbau() -> Aufbau {
        let dir = tempfile::tempdir().unwrap();
        let projekt = dir.path().join("projekt");
        std::fs::create_dir_all(&projekt).unwrap();
        std::fs::write(
            projekt.join("fullbody.txt"),
            "mira | Mira | x | orphan girl\njonas | Jonas | y | orphan boy\n",
        )
        .unwrap();
        let modelle = dir.path().join("models");
        for datei in [
            "diffusion/flux-2-klein-9b-Q5_K_M.gguf",
            "text_encoder/Qwen3-8B-Q5_K_M.gguf",
            "vae/flux2-vae.safetensors",
        ] {
            let pfad = modelle.join(datei);
            std::fs::create_dir_all(pfad.parent().unwrap()).unwrap();
            std::fs::write(pfad, b"x").unwrap();
        }
        let konfig = Konfig {
            daten: dir.path().join("daten"),
            projekt,
            host: Host {
                modelle,
                threads: 1,
                mmap: false,
                flash_attention: false,
                vae_tiling: false,
                hf_token: None,
                ref_bg: [255; 3],
                ref_max_px: None,
            },
        };
        Aufbau { _dir: dir, konfig, aufrufe: Arc::new(AtomicUsize::new(0)) }
    }

    fn start(a: &Aufbau, sperre: Option<Arc<(Mutex<bool>, Condvar)>>, scheitern: bool) -> Dienst {
        let dienst = Dienst::oeffnen(a.konfig.clone()).unwrap();
        let aufrufe = a.aufrufe.clone();
        dienst.worker_starten(move || Box::new(Attrappe { aufrufe, sperre, scheitern }));
        dienst
    }

    fn auftrag(json: &str) -> Auftrag {
        serde_json::from_str(json).unwrap()
    }

    const FRIST: Duration = Duration::from_secs(10);

    #[test]
    fn ein_eingereichter_job_laeuft_durch_und_liefert_die_bilder() {
        let a = aufbau();
        let dienst = start(&a, None, false);
        let job = dienst.einreichen(auftrag(r#"{"kind":"fullbody","prompt":"girl","seeds":2}"#)).unwrap();
        assert_eq!(job.status, JobStatus::Wartend);

        let fertig = dienst.abwarten(&job.id, FRIST).unwrap();
        assert_eq!(fertig.status, JobStatus::Fertig, "{fertig:?}");
        assert_eq!(fertig.bilder.len(), 2);
        assert!(fertig.bilder.iter().all(|b| b.status == BildStatus::Fertig));
        let pfad = dienst.bild_pfad(&job.id, "girl-s42.png").unwrap();
        assert!(pfad.is_file());
        assert!(fertig.beendet.is_some());
        dienst.beenden();
    }

    #[test]
    fn eingeschaltete_beobachtung_legt_pro_job_einen_metrikstrom_an() {
        use crate::metriken::{self, Eintrag as M};
        let a = aufbau();
        let dienst = start(&a, None, false);
        dienst.metriken_einschalten(Some(Duration::from_millis(5)));
        let job = dienst.einreichen(auftrag(r#"{"kind":"fullbody","prompt":"girl"}"#)).unwrap();
        dienst.abwarten(&job.id, FRIST).unwrap();

        let pfad = dienst.metriken_pfad(&job.id).expect("Metrikstrom vorhanden");
        let eintraege = metriken::eintraege_lesen(&pfad).unwrap();
        assert!(eintraege.iter().any(|e| matches!(e, M::Kontext { .. })), "{eintraege:?}");
        assert!(eintraege.iter().any(|e| matches!(e, M::Phase { name, .. } if name == "erzeugen")));
        assert!(eintraege.iter().any(|e| matches!(e, M::Probe { rss_mb, .. } if *rss_mb > 0)));
        dienst.beenden();
    }

    #[test]
    fn bibliothek_listet_listenbilder_ohne_rohbilder_und_liefert_nur_harmlose_pfade() {
        let a = aufbau();
        let dienst = Dienst::oeffnen(a.konfig.clone()).unwrap();
        assert!(dienst.bibliothek(Kind::Fullbody).is_empty());
        let ordner = a.konfig.daten.join("fullbody");
        std::fs::create_dir_all(&ordner).unwrap();
        for n in ["mira.png", "mira.raw.png", "jonas-s7.png", "notiz.txt", ".versteckt.png"] {
            std::fs::write(ordner.join(n), b"x").unwrap();
        }
        assert_eq!(dienst.bibliothek(Kind::Fullbody), ["jonas-s7.png", "mira.png"]);
        assert!(dienst.bibliothek_pfad(Kind::Fullbody, "mira.png").is_ok());
        for boese in ["../fullbody.txt", "..%2Fx.png", "a/b.png", ".versteckt.png", "notiz.txt", "nix.png", ""] {
            assert!(dienst.bibliothek_pfad(Kind::Fullbody, boese).is_err(), "{boese:?}");
        }
    }

    #[test]
    fn vorschau_plant_ohne_einzureihen() {
        let a = aufbau();
        let dienst = Dienst::oeffnen(a.konfig.clone()).unwrap();
        let plan = dienst.vorschau(&auftrag(r#"{"kind":"fullbody","prompt":"girl","seeds":2}"#)).unwrap();
        assert_eq!(plan.aufgaben.len(), 2);
        assert!(dienst.jobs().is_empty(), "eine Vorschau hinterlässt keinen Job");
        assert!(!a.konfig.daten.join("jobs").read_dir().unwrap().any(|_| true), "und nichts auf der Platte");
        assert!(dienst.vorschau(&auftrag(r#"{"kind":"fullbody","prompt":"x","width":300}"#)).is_err());
    }

    #[test]
    fn fehler_im_auftrag_kommen_sofort_beim_einreichen() {
        let a = aufbau();
        let dienst = Dienst::oeffnen(a.konfig.clone()).unwrap();
        let fehler = dienst.einreichen(auftrag(r#"{"kind":"fullbody","prompt":"x","width":300}"#)).unwrap_err();
        assert!(fehler.to_string().contains("durch 16"), "{fehler}");
        assert!(dienst.jobs().is_empty(), "ein abgelehnter Auftrag hinterlässt keinen Job");
    }

    #[test]
    fn jobs_laufen_nacheinander_in_der_reihenfolge_des_einreichens() {
        let a = aufbau();
        let dienst = start(&a, None, false);
        let ids: Vec<_> = ["one", "two", "three"]
            .iter()
            .map(|p| dienst.einreichen(auftrag(&format!(r#"{{"kind":"fullbody","prompt":"{p}"}}"#))).unwrap().id)
            .collect();
        let mut gestartet = Vec::new();
        for id in &ids {
            let j = dienst.abwarten(id, FRIST).unwrap();
            assert_eq!(j.status, JobStatus::Fertig);
            gestartet.push(j.gestartet.unwrap());
        }
        assert!(gestartet.windows(2).all(|w| w[0] <= w[1]));
        dienst.beenden();
    }

    #[test]
    fn scheiternde_engine_macht_den_job_fehlgeschlagen_mit_fehlertext_am_bild() {
        let a = aufbau();
        let dienst = start(&a, None, true);
        let job = dienst.einreichen(auftrag(r#"{"kind":"fullbody","prompt":"girl"}"#)).unwrap();
        let fertig = dienst.abwarten(&job.id, FRIST).unwrap();
        assert_eq!(fertig.status, JobStatus::Fehlgeschlagen);
        assert!(fertig.bilder[0].fehler.as_deref().unwrap().contains("kaputt"));
        dienst.beenden();
    }

    #[test]
    fn wartender_job_laesst_sich_abbrechen_und_laeuft_nie() {
        let a = aufbau();
        // Ohne Worker bleibt der Job sicher wartend.
        let dienst = Dienst::oeffnen(a.konfig.clone()).unwrap();
        let job = dienst.einreichen(auftrag(r#"{"kind":"fullbody","prompt":"girl"}"#)).unwrap();
        let abgebrochen = dienst.abbrechen(&job.id).unwrap();
        assert_eq!(abgebrochen.status, JobStatus::Abgebrochen);
        assert_eq!(abgebrochen.bilder[0].status, BildStatus::Abgebrochen);

        let aufrufe = a.aufrufe.clone();
        dienst.worker_starten(move || Box::new(Attrappe { aufrufe, sperre: None, scheitern: false }));
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(a.aufrufe.load(Ordering::SeqCst), 0);
        dienst.beenden();
    }

    #[test]
    fn laufender_job_wird_nach_dem_aktuellen_bild_abgebrochen() {
        let a = aufbau();
        let sperre = Arc::new((Mutex::new(false), Condvar::new()));
        let dienst = start(&a, Some(sperre.clone()), false);
        let job = dienst.einreichen(auftrag(r#"{"kind":"fullbody","prompt":"girl","seeds":3}"#)).unwrap();
        // Warten, bis das erste Bild in der Engine hängt.
        while a.aufrufe.load(Ordering::SeqCst) == 0 {
            std::thread::sleep(Duration::from_millis(10));
        }
        let angefordert = dienst.abbrechen(&job.id).unwrap();
        assert!(angefordert.abbruch_angefordert);
        {
            *sperre.0.lock().unwrap() = true;
            sperre.1.notify_all();
        }
        let ende = dienst.abwarten(&job.id, FRIST).unwrap();
        assert_eq!(ende.status, JobStatus::Abgebrochen, "{ende:?}");
        assert_eq!(a.aufrufe.load(Ordering::SeqCst), 1, "nur das laufende Bild wird fertig");
        assert_eq!(ende.bilder[0].status, BildStatus::Fertig);
        assert_eq!(ende.bilder[1].status, BildStatus::Abgebrochen);
        dienst.beenden();
    }

    #[test]
    fn beendeter_job_laesst_sich_nicht_abbrechen_aber_entfernen() {
        let a = aufbau();
        let dienst = start(&a, None, false);
        let job = dienst.einreichen(auftrag(r#"{"kind":"fullbody","prompt":"girl"}"#)).unwrap();
        dienst.abwarten(&job.id, FRIST).unwrap();
        assert!(dienst.abbrechen(&job.id).is_err());
        dienst.entfernen(&job.id).unwrap();
        assert!(dienst.job(&job.id).is_none());
        assert!(!a.konfig.daten.join("jobs").join(&job.id).exists(), "Ordner samt Bildern ist weg");
        dienst.beenden();
    }

    #[test]
    fn unbeendeter_job_laesst_sich_nicht_entfernen() {
        let a = aufbau();
        let dienst = Dienst::oeffnen(a.konfig.clone()).unwrap();
        let job = dienst.einreichen(auftrag(r#"{"kind":"fullbody","prompt":"girl"}"#)).unwrap();
        assert!(dienst.entfernen(&job.id).is_err());
    }

    #[test]
    fn bildzugriff_nur_auf_dateien_des_jobs() {
        let a = aufbau();
        let dienst = start(&a, None, false);
        let job = dienst.einreichen(auftrag(r#"{"kind":"fullbody","prompt":"girl"}"#)).unwrap();
        dienst.abwarten(&job.id, FRIST).unwrap();
        assert!(dienst.bild_pfad(&job.id, "girl.png").is_ok());
        for boese in ["../job.json", "job.json", "../../etc/passwd", "girl.png/../../job.json"] {
            assert!(dienst.bild_pfad(&job.id, boese).is_err(), "{boese}");
        }
        assert!(dienst.bild_pfad("gibtsnicht", "girl.png").is_err());
        dienst.beenden();
    }

    #[test]
    fn listenjob_schreibt_in_den_dauerhaften_ordner_und_setzt_beim_zweiten_lauf_fort() {
        let a = aufbau();
        let dienst = start(&a, None, false);
        let erster = dienst.einreichen(auftrag(r#"{"kind":"fullbody","liste":{}}"#)).unwrap();
        assert_eq!(erster.ausgabe, "fullbody");
        let erster = dienst.abwarten(&erster.id, FRIST).unwrap();
        assert_eq!(erster.status, JobStatus::Fertig);
        assert!(a.konfig.daten.join("fullbody/mira.png").is_file());
        assert_eq!(a.aufrufe.load(Ordering::SeqCst), 2);

        // Zweiter Lauf: alles liegt schon da, nichts wird neu erzeugt.
        let zweiter = dienst.einreichen(auftrag(r#"{"kind":"fullbody","liste":{}}"#)).unwrap();
        assert!(zweiter.bilder.iter().all(|b| b.status == BildStatus::Vorhanden));
        let zweiter = dienst.abwarten(&zweiter.id, FRIST).unwrap();
        assert_eq!(zweiter.status, JobStatus::Fertig);
        assert_eq!(a.aufrufe.load(Ordering::SeqCst), 2, "keine neuen Bilder");

        // Mit force wird alles neu erzeugt.
        let dritter = dienst.einreichen(auftrag(r#"{"kind":"fullbody","liste":{"force":true}}"#)).unwrap();
        dienst.abwarten(&dritter.id, FRIST).unwrap();
        assert_eq!(a.aufrufe.load(Ordering::SeqCst), 4);
        dienst.beenden();
    }

    #[test]
    fn jobs_ueberleben_einen_neustart_und_wartende_laufen_weiter() {
        let a = aufbau();
        let (fertig_id, wartend_id) = {
            let dienst = start(&a, None, false);
            let fertig = dienst.einreichen(auftrag(r#"{"kind":"fullbody","prompt":"done"}"#)).unwrap();
            dienst.abwarten(&fertig.id, FRIST).unwrap();
            dienst.beenden();
            // Ein Dienst ohne Worker: der Job bleibt wartend auf der Platte.
            let ohne_worker = Dienst::oeffnen(a.konfig.clone()).unwrap();
            let wartend = ohne_worker.einreichen(auftrag(r#"{"kind":"fullbody","prompt":"later"}"#)).unwrap();
            (fertig.id, wartend.id)
        };
        let dienst = start(&a, None, false);
        assert_eq!(dienst.job(&fertig_id).unwrap().status, JobStatus::Fertig);
        let weiter = dienst.abwarten(&wartend_id, FRIST).unwrap();
        assert_eq!(weiter.status, JobStatus::Fertig, "wartender Job wurde nach dem Neustart abgearbeitet");
        dienst.beenden();
    }

    #[test]
    fn ein_beim_absturz_laufender_job_gilt_nach_dem_neustart_als_abgebrochen() {
        let a = aufbau();
        let id = {
            let dienst = Dienst::oeffnen(a.konfig.clone()).unwrap();
            let job = dienst.einreichen(auftrag(r#"{"kind":"fullbody","prompt":"girl"}"#)).unwrap();
            let mut laufend = job.clone();
            laufend.status = JobStatus::Laeuft;
            laufend.bilder[0].status = BildStatus::Laeuft;
            speichern(&a.konfig.daten, &laufend).unwrap();
            job.id
        };
        let dienst = Dienst::oeffnen(a.konfig.clone()).unwrap();
        let job = dienst.job(&id).unwrap();
        assert_eq!(job.status, JobStatus::Abgebrochen);
        assert_eq!(job.bilder[0].status, BildStatus::Abgebrochen);
        assert!(job.meldung.contains("neu gestartet"));
    }

    #[test]
    fn jobliste_zeigt_die_neuesten_zuerst() {
        let a = aufbau();
        let dienst = Dienst::oeffnen(a.konfig.clone()).unwrap();
        let erster = dienst.einreichen(auftrag(r#"{"kind":"fullbody","prompt":"a"}"#)).unwrap();
        std::thread::sleep(Duration::from_millis(1100));
        let zweiter = dienst.einreichen(auftrag(r#"{"kind":"fullbody","prompt":"b"}"#)).unwrap();
        let ids: Vec<_> = dienst.jobs().into_iter().map(|j| j.id).collect();
        assert_eq!(ids, [zweiter.id, erster.id]);
    }

    #[test]
    fn upload_wird_als_stilreferenz_akzeptiert() {
        let a = aufbau();
        let dienst = Dienst::oeffnen(a.konfig.clone()).unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::new(8, 8))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let id = dienst.uploads().speichern(&png.into_inner()).unwrap();
        let job = dienst
            .einreichen(auftrag(&format!(r#"{{"kind":"fullbody","prompt":"girl","style_ref":"{id}"}}"#)))
            .unwrap();
        assert!(job.plan.aufgaben[0].prompt.starts_with("Use the reference image only"));
        assert_eq!(job.plan.aufgaben[0].refs.len(), 1);
    }
}
