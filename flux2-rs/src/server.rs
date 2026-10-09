//! Verbindet den Router (`api`) mit einem echten Socket (`tiny_http`).
//!
//! Jede Anfrage bekommt einen eigenen Thread: `GET /jobs/{id}?wait=60` blockiert
//! bis zu einer Minute, und währenddessen müssen andere Anfragen (Status,
//! Abbruch) durchkommen. Die Rechenarbeit liegt ohnehin im Worker des Dienstes.

use crate::api::{self, Anfrage, Zugang};
use crate::dienst::Dienst;
use crate::referenzen::MAX_UPLOAD_BYTES;
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use tiny_http::{Header, Method, Response, Server};

/// Größe, die ein Body höchstens haben darf: ein Upload plus etwas Spielraum.
const MAX_BODY: usize = MAX_UPLOAD_BYTES + 1024;

fn query_lesen(roh: &str) -> HashMap<String, String> {
    roh.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None => (p.to_string(), String::new()),
        })
        .collect()
}

fn header(name: &str, wert: &str) -> Header {
    Header::from_bytes(name.as_bytes(), wert.as_bytes()).expect("gültiger Header")
}

/// Bedient Anfragen, bis der Server geschlossen wird. Blockiert.
pub fn bedienen(server: Arc<Server>, dienst: Dienst, zugang: Zugang, projekt: PathBuf) {
    for mut anfrage in server.incoming_requests() {
        let dienst = dienst.clone();
        let zugang = zugang.clone();
        let projekt = projekt.clone();
        std::thread::spawn(move || {
            let methode = match anfrage.method() {
                Method::Get => "GET",
                Method::Post => "POST",
                Method::Delete => "DELETE",
                Method::Put => "PUT",
                _ => "ANDERE",
            };
            let url = anfrage.url().to_string();
            let (pfad, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
            let autorisierung = anfrage
                .headers()
                .iter()
                .find(|h| h.field.equiv("Authorization"))
                .map(|h| h.value.to_string());

            // Zu große Bodies nicht erst vollständig einlesen.
            let mut body = Vec::new();
            let gelesen = anfrage.as_reader().take(MAX_BODY as u64 + 1).read_to_end(&mut body);
            let antwort = if gelesen.is_err() {
                api::Antwort::fehler_text(400, "Body nicht lesbar.")
            } else if body.len() > MAX_BODY {
                api::Antwort::fehler_text(413, "Body ist zu groß.")
            } else {
                let a = Anfrage {
                    methode: methode.into(),
                    pfad: pfad.into(),
                    query: query_lesen(query),
                    autorisierung,
                    body,
                };
                api::behandeln(&dienst, &zugang, &projekt, &a)
            };
            let mut antwort_http = Response::from_data(antwort.body).with_status_code(antwort.status);
            antwort_http.add_header(header("Content-Type", antwort.content_type));
            // Fertige Bilder ändern sich nie unter derselben URL; alles andere ist Zustand.
            let cache = if antwort.content_type == "image/png" { "private, max-age=3600" } else { "no-store" };
            antwort_http.add_header(header("Cache-Control", cache));
            antwort_http.add_header(header("X-Content-Type-Options", "nosniff"));
            if antwort.content_type.starts_with("text/html") {
                // Die Oberfläche lädt nur von sich selbst; Bilder auch als blob: (mit Token geholt).
                antwort_http.add_header(header(
                    "Content-Security-Policy",
                    "default-src 'self'; img-src 'self' blob: data:; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'",
                ));
            }
            let _ = anfrage.respond(antwort_http);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dienst::Konfig;
    use crate::lauf::{Engine, Erzeugung, Host};
    use crate::modelle::Modellsatz;
    use std::time::Duration;

    struct Attrappe;
    impl Engine for Attrappe {
        fn erzeuge(&mut self, _: &Modellsatz, _: &Host, e: &Erzeugung) -> anyhow::Result<()> {
            image::RgbImage::from_pixel(16, 16, image::Rgb([9, 9, 9])).save(e.ziel)?;
            Ok(())
        }
    }

    /// Startet einen echten Server auf einem freien Port.
    fn start(token: Option<&str>) -> (tempfile::TempDir, String, Dienst) {
        let dir = tempfile::tempdir().unwrap();
        let modelle = dir.path().join("models");
        for datei in ["diffusion/flux-2-klein-9b-Q5_K_M.gguf", "text_encoder/Qwen3-8B-Q5_K_M.gguf", "vae/flux2-vae.safetensors"] {
            let pfad = modelle.join(datei);
            std::fs::create_dir_all(pfad.parent().unwrap()).unwrap();
            std::fs::write(pfad, b"x").unwrap();
        }
        let dienst = Dienst::oeffnen(Konfig {
            daten: dir.path().join("daten"),
            projekt: dir.path().to_path_buf(),
            host: Host { modelle, threads: 1, mmap: false, flash_attention: false, vae_tiling: false, hf_token: None, ref_bg: [255; 3], ref_max_px: None },
        })
        .unwrap();
        dienst.worker_starten(|| Box::new(Attrappe));
        let server = Arc::new(Server::http("127.0.0.1:0").unwrap());
        let adresse = format!("http://{}", server.server_addr().to_ip().unwrap());
        let zugang = Zugang { token: token.map(String::from) };
        let (d, p) = (dienst.clone(), dir.path().to_path_buf());
        std::thread::spawn(move || bedienen(server, d, zugang, p));
        (dir, adresse, dienst)
    }

    #[test]
    fn query_wird_in_paare_zerlegt() {
        let q = query_lesen("wait=5&x&leer=");
        assert_eq!(q.get("wait").map(String::as_str), Some("5"));
        assert_eq!(q.get("x").map(String::as_str), Some(""));
        assert!(query_lesen("").is_empty());
    }

    #[test]
    fn ein_job_laeuft_ueber_echtes_http_von_der_anfrage_bis_zum_png() {
        let (_dir, adresse, dienst) = start(None);
        let antwort = ureq::post(&format!("{adresse}/jobs"))
            .send_string(r#"{"kind":"fullbody","prompt":"girl","seeds":2}"#)
            .unwrap();
        assert_eq!(antwort.status(), 202);
        let job: serde_json::Value = serde_json::from_reader(antwort.into_reader()).unwrap();
        let id = job["id"].as_str().unwrap();

        let antwort = ureq::get(&format!("{adresse}/jobs/{id}?wait=10"))
            .timeout(Duration::from_secs(20))
            .call()
            .unwrap();
        let fertig: serde_json::Value = serde_json::from_reader(antwort.into_reader()).unwrap();
        assert_eq!(fertig["status"], "fertig");

        let url = fertig["bilder"][1]["url"].as_str().unwrap();
        let bild = ureq::get(&format!("{adresse}{url}")).call().unwrap();
        assert_eq!(bild.content_type(), "image/png");
        let mut daten = Vec::new();
        bild.into_reader().read_to_end(&mut daten).unwrap();
        assert!(daten.starts_with(b"\x89PNG"));
        dienst.beenden();
    }

    #[test]
    fn fehlerantworten_tragen_status_und_json() {
        let (_dir, adresse, dienst) = start(None);
        match ureq::post(&format!("{adresse}/jobs")).send_string("{kaputt") {
            Err(ureq::Error::Status(400, antwort)) => {
                let wert: serde_json::Value = serde_json::from_reader(antwort.into_reader()).unwrap();
                assert!(wert["fehler"].as_str().unwrap().contains("nicht lesbar"));
            }
            andere => panic!("400 erwartet, bekam {andere:?}"),
        }
        assert!(matches!(ureq::get(&format!("{adresse}/nix")).call(), Err(ureq::Error::Status(404, _))));
        dienst.beenden();
    }

    #[test]
    fn token_wird_ueber_den_authorization_header_geprueft() {
        let (_dir, adresse, dienst) = start(Some("geheim"));
        assert!(matches!(ureq::get(&format!("{adresse}/jobs")).call(), Err(ureq::Error::Status(401, _))));
        let ok = ureq::get(&format!("{adresse}/jobs")).set("Authorization", "Bearer geheim").call().unwrap();
        assert_eq!(ok.status(), 200);
        assert_eq!(ureq::get(&format!("{adresse}/health")).call().unwrap().status(), 200);
        dienst.beenden();
    }

    #[test]
    fn zu_grosser_body_wird_mit_413_abgelehnt() {
        let (_dir, adresse, dienst) = start(None);
        let gross = vec![0u8; MAX_BODY + 10];
        match ureq::post(&format!("{adresse}/uploads")).send_bytes(&gross) {
            Err(ureq::Error::Status(413, _)) => {}
            andere => panic!("413 erwartet, bekam {andere:?}"),
        }
        dienst.beenden();
    }
}
