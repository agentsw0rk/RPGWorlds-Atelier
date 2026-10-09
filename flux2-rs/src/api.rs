//! Die HTTP-Schnittstelle als reine Funktion: `Anfrage` rein, `Antwort` raus.
//!
//! Der Router kennt keine Sockets — `server` verbindet ihn mit `tiny_http`.
//! So lässt sich jede Route ohne Netzwerk testen.
//!
//! ```text
//! GET    /                             die Weboberfläche (auch /app.js, /logik.js, /style.css)
//! GET    /health                       Lebenszeichen
//! GET    /arten                        die vier Arten mit ihren Vorgaben
//! GET    /listen/{art}                 Einträge der Standardliste einer Art, mit vorhandenen Bildern
//! GET    /bibliothek/{art}/{datei}     Bild aus dem dauerhaften Ordner der Art (Batch-Ergebnis)
//! POST   /uploads                      Rohbild als Body → {"id": …} für style_ref/refs
//! POST   /jobs                         Auftrag (JSON) → 202 mit dem Job
//! POST   /jobs?dry_run=1               nur planen: Prompts, Seeds, Dateinamen — nichts wird eingereiht
//! GET    /jobs                         alle Jobs, neueste zuerst
//! GET    /jobs/{id}[?wait=Sekunden]    ein Job; wait wartet bis zum Ende (max. 60 s)
//! DELETE /jobs/{id}                    laufenden Job abbrechen / beendeten entfernen
//! GET    /jobs/{id}/bilder/{datei}     fertiges Bild (PNG)
//! ```

use crate::auftrag::Auftrag;
use crate::dienst::Dienst;
use crate::job::Job;
use crate::kind::Kind;
use crate::listen;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

/// Längstes Warten, das `?wait=` erlaubt.
const MAX_WARTEN_S: u64 = 60;

/// Die Weboberfläche, ins Binary eingebettet: kein zweiter Prozess, kein
/// Build-Schritt, und sie läuft vom selben Ursprung wie die API (kein CORS).
fn statische_datei(pfad: &str) -> Option<Antwort> {
    let (inhalt, typ) = match pfad {
        "" | "index.html" => (include_str!("../web/index.html"), "text/html; charset=utf-8"),
        "app.js" => (include_str!("../web/app.js"), "text/javascript; charset=utf-8"),
        "logik.js" => (include_str!("../web/logik.js"), "text/javascript; charset=utf-8"),
        "style.css" => (include_str!("../web/style.css"), "text/css; charset=utf-8"),
        _ => return None,
    };
    Some(Antwort { status: 200, content_type: typ, body: inhalt.as_bytes().to_vec() })
}

#[derive(Debug, Clone, Default)]
pub struct Anfrage {
    pub methode: String,
    pub pfad: String,
    pub query: HashMap<String, String>,
    /// Inhalt des `Authorization`-Headers, falls vorhanden.
    pub autorisierung: Option<String>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Antwort {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl Antwort {
    fn json(status: u16, wert: Value) -> Antwort {
        Antwort { status, content_type: "application/json", body: wert.to_string().into_bytes() }
    }

    /// Fehlerantwort für den Netzwerkanschluss, der keine Route aufruft.
    pub fn fehler_text(status: u16, text: &str) -> Antwort {
        Antwort::fehler(status, text)
    }

    fn fehler(status: u16, text: impl std::fmt::Display) -> Antwort {
        Antwort::json(status, json!({ "fehler": text.to_string() }))
    }

    fn leer(status: u16) -> Antwort {
        Antwort { status, content_type: "application/json", body: Vec::new() }
    }
}

/// Zugriffsschutz: ohne `token` ist die API offen (gedacht für `127.0.0.1`).
#[derive(Debug, Clone, Default)]
pub struct Zugang {
    pub token: Option<String>,
}

impl Zugang {
    fn erlaubt(&self, anfrage: &Anfrage) -> bool {
        let Some(erwartet) = &self.token else { return true };
        let Some(header) = &anfrage.autorisierung else { return false };
        let Some(gegeben) = header.strip_prefix("Bearer ") else { return false };
        // Vergleich ohne frühen Abbruch: die Laufzeit verrät sonst, wie viele
        // Zeichen eines geratenen Tokens stimmen.
        gegeben.len() == erwartet.len()
            && gegeben.bytes().zip(erwartet.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
    }
}

/// Ein Bild mit seiner URL und dem verwendeten Prompt.
fn bild_json(job: &Job, index: usize) -> Value {
    let bild = &job.bilder[index];
    let url = |datei: &Option<String>| datei.as_ref().map(|d| format!("/jobs/{}/bilder/{d}", job.id));
    json!({
        "stamm": bild.stamm,
        "eintrag": job.plan.aufgaben.get(index).and_then(|a| a.eintrag.as_deref()),
        "seed": bild.seed,
        "status": bild.status,
        "datei": bild.datei,
        "url": url(&bild.datei),
        "roh": bild.roh,
        "roh_url": url(&bild.roh),
        "fehler": bild.fehler,
        "dauer_s": bild.dauer_s,
        "prompt": job.plan.aufgaben.get(index).map(|a| a.prompt.as_str()),
    })
}

fn job_json(job: &Job) -> Value {
    json!({
        "id": job.id,
        "status": job.status,
        "kind": job.plan.kind,
        "preset": job.plan.preset,
        "erstellt": job.erstellt,
        "gestartet": job.gestartet,
        "beendet": job.beendet,
        "meldung": job.meldung,
        "fehler": job.fehler,
        "abbruch_angefordert": job.abbruch_angefordert,
        "groesse": [job.plan.width, job.plan.height],
        "steps": job.plan.steps,
        // Der ursprüngliche Auftrag, damit eine Oberfläche ihn übernehmen oder wiederholen kann.
        "auftrag": job.auftrag,
        "bilder": (0..job.bilder.len()).map(|i| bild_json(job, i)).collect::<Vec<_>>(),
    })
}

/// Was ein Auftrag erzeugen würde — die Antwort auf `?dry_run=1`.
fn plan_json(plan: &crate::auftrag::Plan) -> Value {
    json!({
        "kind": plan.kind,
        "preset": plan.preset,
        "groesse": [plan.width, plan.height],
        "steps": plan.steps,
        "cfg": plan.cfg,
        "guidance": plan.guidance,
        "wtype": plan.wtype,
        "ausgabe": if plan.dauerhaft { "dauerhafter Ordner der Art" } else { "Ordner des Jobs" },
        "bilder": plan.aufgaben.iter().map(|a| json!({
            "stamm": a.stamm,
            "eintrag": a.eintrag,
            "seed": a.seed,
            "prompt": a.prompt,
            "referenzen": a.refs.len(),
            "freigestellt": a.freistellen.is_some(),
            "uebersprungen": a.uebersprungen,
        })).collect::<Vec<_>>(),
    })
}

/// Ein kurzer Name für die Jobliste: der Prompt, der Listeneintrag oder "Liste".
fn job_titel(job: &Job) -> String {
    if let Some(prompt) = &job.auftrag.prompt {
        let kurz: String = prompt.chars().take(70).collect();
        return if prompt.chars().count() > 70 { format!("{kurz}…") } else { kurz };
    }
    let mut slugs: Vec<&str> = job.plan.aufgaben.iter().filter_map(|a| a.eintrag.as_deref()).collect();
    slugs.dedup(); // Varianten desselben Eintrags stehen hintereinander
    match slugs.as_slice() {
        [einer] => einer.to_string(),
        viele => format!("Liste ({} Einträge)", viele.len()),
    }
}

/// Eine Liste samt den Bildern, die es zu jedem Eintrag schon gibt.
fn liste_json(kind: Kind, eintraege: &[listen::Eintrag], vorhanden: &[String]) -> Value {
    let eintraege: Vec<Value> = eintraege
        .iter()
        .map(|e| {
            let bilder: Vec<&String> = vorhanden
                .iter()
                .filter(|n| {
                    // "<slug>.png" oder "<slug>-s<seed>.png" — aber nicht "<slug>-anderer.png".
                    let Some(rest) = n.strip_prefix(e.slug.as_str()) else { return false };
                    rest == ".png"
                        || rest.strip_prefix("-s").and_then(|r| r.strip_suffix(".png")).is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                })
                .collect();
            json!({
                "slug": e.slug,
                "name": e.name,
                "beschreibung": e.beschreibung,
                "prompt": e.prompt,
                "bilder": bilder,
            })
        })
        .collect();
    json!({ "kind": kind, "eintraege": eintraege })
}

/// Kurzfassung für die Jobliste: ohne die einzelnen Bilder.
fn job_kurz(job: &Job) -> Value {
    let fertig = job
        .bilder
        .iter()
        .filter(|b| matches!(b.status, crate::job::BildStatus::Fertig | crate::job::BildStatus::Vorhanden))
        .count();
    json!({
        "id": job.id,
        "status": job.status,
        "kind": job.plan.kind,
        "titel": job_titel(job),
        "erstellt": job.erstellt,
        "meldung": job.meldung,
        "bilder_gesamt": job.bilder.len(),
        "bilder_fertig": fertig,
    })
}

fn arten_json() -> Value {
    Value::Array(
        Kind::ALLE
            .iter()
            .map(|&k| {
                let v = k.vorgaben(false);
                json!({
                    "kind": k,
                    "width": v.width,
                    "height": v.height,
                    "steps": v.steps,
                    "freistellen": !v.keep_bg,
                    "liste": listen::standard_liste(k),
                })
            })
            .collect(),
    )
}

fn pfadteile(pfad: &str) -> Vec<&str> {
    pfad.split('/').filter(|t| !t.is_empty()).collect()
}

fn warte_sekunden(anfrage: &Anfrage) -> Result<u64, Antwort> {
    match anfrage.query.get("wait") {
        None => Ok(0),
        Some(wert) => wert
            .parse::<u64>()
            .map(|s| s.min(MAX_WARTEN_S))
            .map_err(|_| Antwort::fehler(400, "wait erwartet eine Zahl in Sekunden.")),
    }
}

/// Beantwortet eine Anfrage.
pub fn behandeln(dienst: &Dienst, zugang: &Zugang, projekt: &std::path::Path, anfrage: &Anfrage) -> Antwort {
    let teile = pfadteile(&anfrage.pfad);
    // Die Oberfläche selbst enthält nichts Geheimes und muss ohne Schlüssel
    // laden können, damit sie danach nach dem Token fragen kann. Ebenso /health.
    let offen = teile.as_slice() == ["health"]
        || (anfrage.methode.eq_ignore_ascii_case("GET")
            && teile.len() <= 1
            && statische_datei(teile.first().copied().unwrap_or("")).is_some());
    if !offen && !zugang.erlaubt(anfrage) {
        return Antwort::fehler(401, "Authorization: Bearer <token> fehlt oder stimmt nicht.");
    }
    let methode = anfrage.methode.to_ascii_uppercase();

    match (methode.as_str(), teile.as_slice()) {
        ("GET", []) | ("GET", ["index.html" | "app.js" | "logik.js" | "style.css"]) => {
            statische_datei(teile.first().copied().unwrap_or("")).expect("oben geprüft")
        }
        ("GET", ["health"]) => Antwort::json(200, json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") })),
        ("GET", ["arten"]) => Antwort::json(200, arten_json()),
        ("GET", ["listen", art]) => match art.parse::<Kind>() {
            Err(fehler) => Antwort::fehler(404, fehler),
            Ok(kind) => {
                let pfad = projekt.join(listen::standard_liste(kind));
                let eintraege = std::fs::read_to_string(&pfad)
                    .map_err(|e| e.to_string())
                    .and_then(|t| listen::lesen(&t).map_err(|e| e.to_string()));
                match eintraege {
                    Ok(eintraege) => {
                        let vorhanden = dienst.bibliothek(kind);
                        Antwort::json(200, liste_json(kind, &eintraege, &vorhanden))
                    }
                    Err(fehler) => Antwort::fehler(500, format!("Liste nicht lesbar: {fehler}")),
                }
            }
        },
        ("GET", ["bibliothek", art, datei]) => match art.parse::<Kind>() {
            Err(fehler) => Antwort::fehler(404, fehler),
            Ok(kind) => match dienst.bibliothek_pfad(kind, datei).and_then(|p| Ok(std::fs::read(p)?)) {
                Ok(daten) => Antwort { status: 200, content_type: "image/png", body: daten },
                Err(fehler) => Antwort::fehler(404, format!("{fehler:#}")),
            },
        },

        ("POST", ["uploads"]) => match dienst.uploads().speichern(&anfrage.body) {
            Ok(id) => Antwort::json(201, json!({ "id": id })),
            Err(fehler) => Antwort::fehler(400, format!("{fehler:#}")),
        },

        ("POST", ["jobs"]) => {
            let auftrag: Auftrag = match serde_json::from_slice(&anfrage.body) {
                Ok(a) => a,
                Err(fehler) => return Antwort::fehler(400, format!("Auftrag nicht lesbar: {fehler}")),
            };
            if matches!(anfrage.query.get("dry_run").map(String::as_str), Some("1" | "true")) {
                return match dienst.vorschau(&auftrag) {
                    Ok(plan) => Antwort::json(200, plan_json(&plan)),
                    Err(fehler) => Antwort::fehler(400, format!("{fehler:#}")),
                };
            }
            match dienst.einreichen(auftrag) {
                Ok(job) => Antwort::json(202, job_json(&job)),
                Err(fehler) => Antwort::fehler(400, format!("{fehler:#}")),
            }
        }
        ("GET", ["jobs"]) => {
            let jobs = dienst.jobs();
            Antwort::json(200, json!({ "jobs": jobs.iter().map(job_kurz).collect::<Vec<_>>() }))
        }
        ("GET", ["jobs", id]) => {
            let warten = match warte_sekunden(anfrage) {
                Ok(s) => s,
                Err(antwort) => return antwort,
            };
            let job = if warten > 0 { dienst.abwarten(id, Duration::from_secs(warten)) } else { dienst.job(id) };
            match job {
                Some(job) => Antwort::json(200, job_json(&job)),
                None => Antwort::fehler(404, format!("Job '{id}' gibt es nicht.")),
            }
        }
        ("DELETE", ["jobs", id]) => match dienst.job(id) {
            None => Antwort::fehler(404, format!("Job '{id}' gibt es nicht.")),
            Some(job) if job.status.ist_beendet() => match dienst.entfernen(id) {
                Ok(()) => Antwort::leer(204),
                Err(fehler) => Antwort::fehler(409, format!("{fehler:#}")),
            },
            Some(_) => match dienst.abbrechen(id) {
                Ok(job) => Antwort::json(202, job_json(&job)),
                Err(fehler) => Antwort::fehler(409, format!("{fehler:#}")),
            },
        },
        ("GET", ["jobs", id, "bilder", datei]) => match dienst.bild_pfad(id, datei) {
            Ok(pfad) => match std::fs::read(&pfad) {
                Ok(daten) => Antwort { status: 200, content_type: "image/png", body: daten },
                Err(fehler) => Antwort::fehler(500, format!("Bild nicht lesbar: {fehler}")),
            },
            Err(fehler) => Antwort::fehler(404, format!("{fehler:#}")),
        },

        ("GET", ["jobs", id, "metrics"]) => match dienst.metriken_pfad(id) {
            None => Antwort::fehler(404, format!("Für Job '{id}' gibt es keine Metriken (Job unbekannt oder Beobachtung aus).")),
            Some(pfad) => match crate::metriken::eintraege_lesen(&pfad) {
                Ok(eintraege) => {
                    let zusammenfassung = crate::metriken::zusammenfassen(&eintraege);
                    // `?kurz=1`: nur die Zusammenfassung — für die Anzeige, die jede Sekunde fragt.
                    if matches!(anfrage.query.get("kurz").map(String::as_str), Some("1" | "true")) {
                        Antwort::json(200, json!({ "id": id, "zusammenfassung": zusammenfassung }))
                    } else {
                        Antwort::json(200, json!({ "id": id, "zusammenfassung": zusammenfassung, "eintraege": eintraege }))
                    }
                }
                Err(fehler) => Antwort::fehler(500, format!("{fehler:#}")),
            },
        },

        // Pfad bekannt, Methode nicht: 405 statt 404, damit ein falscher Verb-Aufruf auffällt.
        (_, [] | ["health"] | ["arten"] | ["listen", _] | ["bibliothek", _, _] | ["uploads"] | ["jobs"] | ["jobs", _] | ["jobs", _, "bilder", _] | ["jobs", _, "metrics"]) => {
            Antwort::fehler(405, format!("{methode} ist hier nicht erlaubt."))
        }
        _ => Antwort::fehler(404, format!("{} gibt es nicht.", anfrage.pfad)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dienst::Konfig;
    use crate::lauf::{Engine, Erzeugung, Host};
    use crate::modelle::Modellsatz;

    struct Attrappe;
    impl Engine for Attrappe {
        fn erzeuge(&mut self, _: &Modellsatz, _: &Host, e: &Erzeugung) -> anyhow::Result<()> {
            image::RgbImage::from_pixel(16, 16, image::Rgb([9, 9, 9])).save(e.ziel)?;
            Ok(())
        }
    }

    struct Aufbau {
        _dir: tempfile::TempDir,
        projekt: std::path::PathBuf,
        dienst: Dienst,
    }

    fn aufbau(mit_worker: bool) -> Aufbau {
        let dir = tempfile::tempdir().unwrap();
        let projekt = dir.path().join("projekt");
        std::fs::create_dir_all(&projekt).unwrap();
        std::fs::write(projekt.join("fullbody.txt"), "mira | Mira | x | orphan girl\n").unwrap();
        let modelle = dir.path().join("models");
        for datei in ["diffusion/flux-2-klein-9b-Q5_K_M.gguf", "text_encoder/Qwen3-8B-Q5_K_M.gguf", "vae/flux2-vae.safetensors"] {
            let pfad = modelle.join(datei);
            std::fs::create_dir_all(pfad.parent().unwrap()).unwrap();
            std::fs::write(pfad, b"x").unwrap();
        }
        let dienst = Dienst::oeffnen(Konfig {
            daten: dir.path().join("daten"),
            projekt: projekt.clone(),
            host: Host { modelle, threads: 1, mmap: false, flash_attention: false, vae_tiling: false, hf_token: None, ref_bg: [255; 3], ref_max_px: None },
        })
        .unwrap();
        if mit_worker {
            dienst.worker_starten(|| Box::new(Attrappe));
        }
        Aufbau { _dir: dir, projekt, dienst }
    }

    fn anfrage(methode: &str, pfad: &str, body: &[u8]) -> Anfrage {
        let (pfad, query) = pfad.split_once('?').unwrap_or((pfad, ""));
        Anfrage {
            methode: methode.into(),
            pfad: pfad.into(),
            query: query.split('&').filter_map(|p| p.split_once('=')).map(|(k, v)| (k.into(), v.into())).collect(),
            autorisierung: None,
            body: body.to_vec(),
        }
    }

    fn rufen(a: &Aufbau, methode: &str, pfad: &str, body: &str) -> (u16, Value) {
        let antwort = behandeln(&a.dienst, &Zugang::default(), &a.projekt, &anfrage(methode, pfad, body.as_bytes()));
        let wert = if antwort.body.is_empty() || antwort.content_type != "application/json" {
            Value::Null
        } else {
            serde_json::from_slice(&antwort.body).unwrap()
        };
        (antwort.status, wert)
    }

    #[test]
    fn health_antwortet_ok() {
        let a = aufbau(false);
        let (status, wert) = rufen(&a, "GET", "/health", "");
        assert_eq!(status, 200);
        assert_eq!(wert["status"], "ok");
    }

    #[test]
    fn arten_nennt_die_vier_skripte_mit_ihren_vorgaben() {
        let a = aufbau(false);
        let (_, wert) = rufen(&a, "GET", "/arten", "");
        let arten = wert.as_array().unwrap();
        assert_eq!(arten.len(), 4);
        let fullbody = arten.iter().find(|k| k["kind"] == "fullbody").unwrap();
        assert_eq!((fullbody["width"].as_u64(), fullbody["height"].as_u64()), (Some(768), Some(1536)));
        let token = arten.iter().find(|k| k["kind"] == "token").unwrap();
        assert_eq!(token["freistellen"], true);
    }

    #[test]
    fn oberflaeche_wird_ohne_token_ausgeliefert_die_api_dahinter_nicht() {
        let a = aufbau(false);
        let zugang = Zugang { token: Some("geheim".into()) };
        for (pfad, typ) in [("/", "text/html"), ("/index.html", "text/html"), ("/app.js", "text/javascript"), ("/logik.js", "text/javascript"), ("/style.css", "text/css")] {
            let antwort = behandeln(&a.dienst, &zugang, &a.projekt, &anfrage("GET", pfad, b""));
            assert_eq!(antwort.status, 200, "{pfad}");
            assert!(antwort.content_type.starts_with(typ), "{pfad}: {}", antwort.content_type);
        }
        assert_eq!(behandeln(&a.dienst, &zugang, &a.projekt, &anfrage("GET", "/jobs", b"")).status, 401);
        assert_eq!(behandeln(&a.dienst, &zugang, &a.projekt, &anfrage("POST", "/", b"")).status, 401, "nur GET ist offen");
        assert_eq!(behandeln(&a.dienst, &zugang, &a.projekt, &anfrage("GET", "/secret.js", b"")).status, 401);
    }

    #[test]
    fn liste_nennt_beschreibung_und_vorhandene_bilder() {
        let a = aufbau(false);
        std::fs::write(a.projekt.join("fullbody.txt"), "mira | Mira | Mädchen am Brunnen | orphan girl\njonas | Jonas | Junge | boy\n").unwrap();
        let ordner = a._dir.path().join("daten/fullbody");
        std::fs::create_dir_all(&ordner).unwrap();
        for n in ["mira.png", "mira-s7.png", "mira-anders.png", "mira.raw.png", "jonas-s.png"] {
            std::fs::write(ordner.join(n), b"x").unwrap();
        }
        let (_, wert) = rufen(&a, "GET", "/listen/fullbody", "");
        assert_eq!(wert["eintraege"][0]["beschreibung"], "Mädchen am Brunnen");
        assert_eq!(wert["eintraege"][0]["bilder"], json!(["mira-s7.png", "mira.png"]));
        assert_eq!(wert["eintraege"][1]["bilder"], json!([]), "jonas-s.png trägt keinen Seed");
    }

    #[test]
    fn bibliothek_liefert_nur_bilder_der_art() {
        let a = aufbau(false);
        let ordner = a._dir.path().join("daten/fullbody");
        std::fs::create_dir_all(&ordner).unwrap();
        image::RgbImage::new(4, 4).save(ordner.join("mira.png")).unwrap();
        let antwort = behandeln(&a.dienst, &Zugang::default(), &a.projekt, &anfrage("GET", "/bibliothek/fullbody/mira.png", b""));
        assert_eq!((antwort.status, antwort.content_type), (200, "image/png"));
        for pfad in ["/bibliothek/fullbody/nix.png", "/bibliothek/comic/mira.png", "/bibliothek/fullbody/..%2Fx.png", "/bibliothek/token/mira.png"] {
            assert_eq!(rufen(&a, "GET", pfad, "").0, 404, "{pfad}");
        }
    }

    #[test]
    fn job_enthaelt_den_auftrag_und_die_liste_einen_titel() {
        let a = aufbau(false);
        let (_, job) = rufen(&a, "POST", "/jobs", r#"{"kind":"fullbody","prompt":"girl","seeds":2}"#);
        assert_eq!(job["auftrag"]["prompt"], "girl");
        assert_eq!(job["auftrag"]["seeds"], 2);
        let lang = "x".repeat(100);
        rufen(&a, "POST", "/jobs", &format!(r#"{{"kind":"fullbody","prompt":"{lang}"}}"#));
        rufen(&a, "POST", "/jobs", r#"{"kind":"fullbody","liste":{}}"#);
        let (_, liste) = rufen(&a, "GET", "/jobs", "");
        let titel: Vec<_> = liste["jobs"].as_array().unwrap().iter().map(|j| j["titel"].as_str().unwrap().to_string()).collect();
        assert!(titel.contains(&"girl".to_string()));
        assert!(titel.iter().any(|t| t.ends_with('…') && t.chars().count() == 71), "{titel:?}");
        assert!(titel.contains(&"mira".to_string()), "Liste mit einem Eintrag heißt wie der Eintrag: {titel:?}");
    }

    #[test]
    fn liste_zeigt_die_eintraege_einer_art() {
        let a = aufbau(false);
        let (status, wert) = rufen(&a, "GET", "/listen/fullbody", "");
        assert_eq!(status, 200);
        assert_eq!(wert["eintraege"][0]["slug"], "mira");
        assert_eq!(rufen(&a, "GET", "/listen/comic", "").0, 404);
    }

    #[test]
    fn job_einreichen_liefert_202_und_ein_wartendes_bild() {
        let a = aufbau(false);
        let (status, wert) = rufen(&a, "POST", "/jobs", r#"{"kind":"fullbody","prompt":"girl","seeds":2}"#);
        assert_eq!(status, 202, "{wert}");
        assert_eq!(wert["status"], "wartend");
        assert_eq!(wert["bilder"].as_array().unwrap().len(), 2);
        assert_eq!(wert["bilder"][1]["seed"], 43);
        assert!(wert["bilder"][0]["eintrag"].is_null(), "Einzelprompt hat keinen Listeneintrag");
        assert!(wert["bilder"][0]["prompt"].as_str().unwrap().starts_with("girl, "));
    }

    #[test]
    fn dry_run_zeigt_den_plan_ohne_einen_job_anzulegen() {
        let a = aufbau(false);
        let (status, plan) = rufen(&a, "POST", "/jobs?dry_run=1", r#"{"kind":"token","prompt":"dwarf","seeds":[5,6]}"#);
        assert_eq!(status, 200, "{plan}");
        assert_eq!(plan["bilder"].as_array().unwrap().len(), 2);
        assert_eq!(plan["bilder"][1]["stamm"], "dwarf-s6");
        assert_eq!(plan["bilder"][0]["freigestellt"], true);
        assert!(plan["bilder"][0]["prompt"].as_str().unwrap().contains("calm upright pose"));
        assert_eq!(rufen(&a, "GET", "/jobs", "").1["jobs"].as_array().unwrap().len(), 0, "nichts eingereiht");
        let (status, fehler) = rufen(&a, "POST", "/jobs?dry_run=1", r#"{"kind":"fullbody","prompt":"x","width":300}"#);
        assert_eq!(status, 400, "auch die Vorschau prüft den Auftrag: {fehler}");
    }

    #[test]
    fn kaputtes_json_und_unbekannte_felder_geben_400_mit_text() {
        let a = aufbau(false);
        let (status, wert) = rufen(&a, "POST", "/jobs", "{kaputt");
        assert_eq!(status, 400);
        assert!(wert["fehler"].as_str().unwrap().contains("nicht lesbar"));
        let (status, wert) = rufen(&a, "POST", "/jobs", r#"{"kind":"fullbody","prompt":"x","seed_count":3}"#);
        assert_eq!(status, 400);
        assert!(wert["fehler"].as_str().unwrap().contains("seed_count"));
    }

    #[test]
    fn ungueltiger_auftrag_gibt_400_und_hinterlaesst_keinen_job() {
        let a = aufbau(false);
        let (status, wert) = rufen(&a, "POST", "/jobs", r#"{"kind":"fullbody","prompt":"x","width":300}"#);
        assert_eq!(status, 400);
        assert!(wert["fehler"].as_str().unwrap().contains("durch 16"));
        assert_eq!(rufen(&a, "GET", "/jobs", "").1["jobs"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn job_mit_wait_wartet_bis_zum_ende_und_liefert_bild_urls() {
        let a = aufbau(true);
        let (_, job) = rufen(&a, "POST", "/jobs", r#"{"kind":"fullbody","prompt":"girl"}"#);
        let id = job["id"].as_str().unwrap();
        let (status, fertig) = rufen(&a, "GET", &format!("/jobs/{id}?wait=10"), "");
        assert_eq!(status, 200);
        assert_eq!(fertig["status"], "fertig", "{fertig}");
        let url = fertig["bilder"][0]["url"].as_str().unwrap();
        assert_eq!(url, format!("/jobs/{id}/bilder/girl.png"));

        let antwort = behandeln(&a.dienst, &Zugang::default(), &a.projekt, &anfrage("GET", url, b""));
        assert_eq!(antwort.status, 200);
        assert_eq!(antwort.content_type, "image/png");
        assert!(antwort.body.starts_with(b"\x89PNG"));
        a.dienst.beenden();
    }

    #[test]
    fn wait_muss_eine_zahl_sein() {
        let a = aufbau(false);
        let (_, job) = rufen(&a, "POST", "/jobs", r#"{"kind":"fullbody","prompt":"girl"}"#);
        let id = job["id"].as_str().unwrap();
        assert_eq!(rufen(&a, "GET", &format!("/jobs/{id}?wait=lange"), "").0, 400);
    }

    #[test]
    fn jobliste_ist_eine_kurzfassung() {
        let a = aufbau(false);
        rufen(&a, "POST", "/jobs", r#"{"kind":"fullbody","prompt":"girl","seeds":3}"#);
        let (_, liste) = rufen(&a, "GET", "/jobs", "");
        let eintrag = &liste["jobs"][0];
        assert_eq!(eintrag["bilder_gesamt"], 3);
        assert_eq!(eintrag["bilder_fertig"], 0);
        assert!(eintrag.get("bilder").is_none(), "die Liste enthält nicht jedes Bild");
    }

    #[test]
    fn delete_bricht_wartenden_ab_und_entfernt_beendeten() {
        let a = aufbau(false);
        let (_, job) = rufen(&a, "POST", "/jobs", r#"{"kind":"fullbody","prompt":"girl"}"#);
        let id = job["id"].as_str().unwrap().to_string();
        let (status, wert) = rufen(&a, "DELETE", &format!("/jobs/{id}"), "");
        assert_eq!((status, wert["status"].as_str()), (202, Some("abgebrochen")));
        assert_eq!(rufen(&a, "DELETE", &format!("/jobs/{id}"), "").0, 204, "beendet → entfernt");
        assert_eq!(rufen(&a, "GET", &format!("/jobs/{id}"), "").0, 404);
        assert_eq!(rufen(&a, "DELETE", &format!("/jobs/{id}"), "").0, 404);
    }

    #[test]
    fn bilder_gibt_es_nur_fuer_dateien_des_jobs() {
        let a = aufbau(true);
        let (_, job) = rufen(&a, "POST", "/jobs", r#"{"kind":"fullbody","prompt":"girl"}"#);
        let id = job["id"].as_str().unwrap();
        a.dienst.abwarten(id, Duration::from_secs(10));
        for boese in ["job.json", "..%2Fjob.json", "x.png"] {
            assert_eq!(rufen(&a, "GET", &format!("/jobs/{id}/bilder/{boese}"), "").0, 404, "{boese}");
        }
        a.dienst.beenden();
    }

    #[test]
    fn metrics_liefert_den_metrikstrom_des_jobs() {
        let a = aufbau(true);
        a.dienst.metriken_einschalten(Some(Duration::from_millis(5)));
        let (_, job) = rufen(&a, "POST", "/jobs", r#"{"kind":"fullbody","prompt":"girl"}"#);
        let id = job["id"].as_str().unwrap();
        a.dienst.abwarten(id, Duration::from_secs(10));
        let (status, wert) = rufen(&a, "GET", &format!("/jobs/{id}/metrics"), "");
        assert_eq!(status, 200, "{wert}");
        let arten: Vec<&str> = wert["eintraege"].as_array().unwrap().iter().filter_map(|e| e["art"].as_str()).collect();
        assert!(arten.contains(&"kontext") && arten.contains(&"phase") && arten.contains(&"probe"), "{arten:?}");
        a.dienst.beenden();
    }

    #[test]
    fn metrics_enthaelt_die_zusammenfassung_mit_phasen() {
        let a = aufbau(true);
        a.dienst.metriken_einschalten(Some(Duration::from_millis(5)));
        let (_, job) = rufen(&a, "POST", "/jobs", r#"{"kind":"fullbody","prompt":"girl"}"#);
        let id = job["id"].as_str().unwrap();
        a.dienst.abwarten(id, Duration::from_secs(10));
        let (_, wert) = rufen(&a, "GET", &format!("/jobs/{id}/metrics"), "");
        assert_eq!(wert["zusammenfassung"]["letzte_phase"], "bild_fertig");
        assert!(wert["zusammenfassung"]["spitze_rss_mb"].as_u64().unwrap() > 0);
        let phasen: Vec<&str> = wert["zusammenfassung"]["phasen"].as_array().unwrap().iter().filter_map(|p| p["name"].as_str()).collect();
        assert_eq!(phasen, ["referenz", "erzeugen", "bild_fertig"]);
        a.dienst.beenden();
    }

    #[test]
    fn metrics_kurz_laesst_die_einzelnen_eintraege_weg() {
        let a = aufbau(true);
        a.dienst.metriken_einschalten(Some(Duration::from_millis(5)));
        let (_, job) = rufen(&a, "POST", "/jobs", r#"{"kind":"fullbody","prompt":"girl"}"#);
        let id = job["id"].as_str().unwrap();
        a.dienst.abwarten(id, Duration::from_secs(10));
        let (status, wert) = rufen(&a, "GET", &format!("/jobs/{id}/metrics?kurz=1"), "");
        assert_eq!(status, 200);
        assert!(wert.get("eintraege").is_none(), "{wert}");
        assert!(wert["zusammenfassung"]["aktuell"]["gesamt_mb"].as_u64().unwrap() > 0);
        a.dienst.beenden();
    }

    #[test]
    fn upload_nimmt_bilder_an_und_lehnt_anderes_ab() {
        let a = aufbau(false);
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::new(4, 4)).write_to(&mut png, image::ImageFormat::Png).unwrap();
        let antwort = behandeln(&a.dienst, &Zugang::default(), &a.projekt, &anfrage("POST", "/uploads", &png.into_inner()));
        assert_eq!(antwort.status, 201);
        let id = serde_json::from_slice::<Value>(&antwort.body).unwrap()["id"].as_str().unwrap().to_string();

        let (status, job) = rufen(&a, "POST", "/jobs", &format!(r#"{{"kind":"fullbody","prompt":"girl","style_ref":"{id}"}}"#));
        assert_eq!(status, 202, "{job}");
        assert!(job["bilder"][0]["prompt"].as_str().unwrap().starts_with("Use the reference image only"));

        assert_eq!(rufen(&a, "POST", "/uploads", "kein bild").0, 400);
    }

    #[test]
    fn unbekannter_upload_in_einem_auftrag_gibt_400() {
        let a = aufbau(false);
        let (status, wert) = rufen(&a, "POST", "/jobs", r#"{"kind":"fullbody","prompt":"x","style_ref":"0123456789abcdef"}"#);
        assert_eq!(status, 400);
        assert!(wert["fehler"].as_str().unwrap().contains("0123456789abcdef"));
    }

    #[test]
    fn falsche_methode_gibt_405_unbekannter_pfad_404() {
        let a = aufbau(false);
        assert_eq!(rufen(&a, "PUT", "/jobs", "").0, 405);
        assert_eq!(rufen(&a, "POST", "/health", "").0, 405);
        assert_eq!(rufen(&a, "GET", "/gibtsnicht", "").0, 404);
    }

    #[test]
    fn mit_token_brauchen_alle_routen_ausser_health_den_bearer_header() {
        let a = aufbau(false);
        let zugang = Zugang { token: Some("geheim".into()) };
        let ohne = behandeln(&a.dienst, &zugang, &a.projekt, &anfrage("GET", "/jobs", b""));
        assert_eq!(ohne.status, 401);
        let mut falsch = anfrage("GET", "/jobs", b"");
        falsch.autorisierung = Some("Bearer falsch".into());
        assert_eq!(behandeln(&a.dienst, &zugang, &a.projekt, &falsch).status, 401);
        let mut richtig = anfrage("GET", "/jobs", b"");
        richtig.autorisierung = Some("Bearer geheim".into());
        assert_eq!(behandeln(&a.dienst, &zugang, &a.projekt, &richtig).status, 200);
        assert_eq!(behandeln(&a.dienst, &zugang, &a.projekt, &anfrage("GET", "/health", b"")).status, 200);
    }

    #[test]
    fn ohne_token_ist_die_api_offen() {
        let a = aufbau(false);
        assert_eq!(rufen(&a, "GET", "/jobs", "").0, 200);
    }
}
