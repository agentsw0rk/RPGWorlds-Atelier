//! HTTP-API für FLUX.2: nimmt Aufträge als JSON an, stellt sie in eine
//! Warteschlange und erzeugt die Bilder einen Job nach dem anderen.
//!
//! Alles läuft in diesem einen Prozess — kein Shell-Skript, kein Subprozess.
//! Konfiguration über Umgebungsvariablen, siehe `flux2_rs::konfig`.
//!
//! ```sh
//! FLUX2_PROJECT=. FLUX2_DATA=api-daten flux2-api
//! curl -X POST localhost:8080/jobs -d '{"kind":"fullbody","prompt":"…","seeds":4}'
//! ```

use anyhow::{Context, Result};
use flux2_rs::api::Zugang;
use flux2_rs::dienst::Dienst;
use flux2_rs::demo::DemoEngine;
use flux2_rs::engine::SdEngine;
use flux2_rs::{konfig, server};
use std::sync::Arc;

fn main() -> Result<()> {
    let mut konfig = konfig::aus_umgebung(&|name| std::env::var(name).ok())?;
    // FLUX2_DEMO=1: Platzhalterbilder statt Modell — zum Ausprobieren der Oberfläche.
    let demo = std::env::var("FLUX2_DEMO").is_ok_and(|v| flux2_rs::params::flag_wert(Some(&v), false));
    if demo && std::env::var("FLUX2_DATA").is_err() {
        // Eigenes Verzeichnis: Demo-Bilder in fullbody/ & Co. würden später als
        // "schon vorhanden" gelten und echte Bilder beim Batch überspringen lassen.
        konfig.dienst.daten = "api-daten-demo".into();
    }
    let dienst = Dienst::oeffnen(konfig.dienst.clone())?;
    dienst.metriken_einschalten(konfig.metriken_abstand);

    let log = konfig.log;
    dienst.worker_starten(move || -> Box<dyn flux2_rs::lauf::Engine> {
        if demo {
            Box::new(DemoEngine::neu())
        } else {
            Box::new(SdEngine::neu(log))
        }
    });
    if demo {
        println!("DEMO-MODUS: Platzhalterbilder, keine Modelle, kein Download.");
    }

    let http = tiny_http::Server::http(konfig.bind)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .with_context(|| format!("Port {} nicht bindbar", konfig.bind))?;
    println!("flux2-api lauscht auf http://{}", konfig.bind);
    println!("Modelle: {}", konfig.dienst.host.modelle.display());
    println!("Daten:   {}", konfig.dienst.daten.display());
    match konfig.metriken_abstand {
        Some(d) => println!("Metriken: alle {} ms nach jobs/<id>/metrics.jsonl", d.as_millis()),
        None => println!("Metriken: aus (METRICS_MS=0)"),
    }
    if konfig.token.is_none() {
        println!("Kein FLUX2_API_TOKEN gesetzt — die API ist offen (nur lokal erreichbar).");
    }
    server::bedienen(
        Arc::new(http),
        dienst,
        Zugang { token: konfig.token },
        konfig.dienst.projekt.clone(),
    );
    Ok(())
}
