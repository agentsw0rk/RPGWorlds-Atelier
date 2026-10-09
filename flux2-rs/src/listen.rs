//! Die Eintragslisten der Batch-Skripte (`charaktere.txt`, `orte.txt`,
//! `charaktere_portraits.txt`, `fullbody.txt`).
//!
//! Format je Zeile, mit `|` getrennt:
//!
//! ```text
//! slug | Name | Beschreibung (deutsch, für den Menschen) | Prompt (englisch, fürs Modell)
//! ```
//!
//! Zeilen mit `#` und leere Zeilen werden übersprungen. Das Modell bekommt nur
//! den Prompt; der `slug` wird zum Dateinamen.

use crate::kind::Kind;
use anyhow::{bail, Result};
use serde::Serialize;

/// Ein Eintrag aus einer Liste.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Eintrag {
    pub slug: String,
    pub name: String,
    /// Deutsche Beschreibung für den Menschen; das Modell bekommt sie nie.
    pub beschreibung: String,
    pub prompt: String,
}

/// Dateiname der Standardliste im Projektverzeichnis.
pub fn standard_liste(kind: Kind) -> &'static str {
    match kind {
        Kind::Token => "charaktere.txt",
        Kind::Location => "orte.txt",
        Kind::Portrait => "charaktere_portraits.txt",
        Kind::Fullbody => "fullbody.txt",
    }
}

/// Zielverzeichnis, das das jeweilige Batch-Skript benutzt.
pub fn standard_ausgabeordner(kind: Kind) -> &'static str {
    match kind {
        Kind::Token => "tokens",
        Kind::Location => "locations",
        Kind::Portrait => "portraits",
        Kind::Fullbody => "fullbody",
    }
}

/// Liest eine Liste. Fehlt bei einer Zeile der Prompt, bricht das Lesen ab —
/// wie die Batch-Skripte, die lieber vor dem ersten Bild scheitern als nach dem
/// zehnten.
pub fn lesen(text: &str) -> Result<Vec<Eintrag>> {
    let mut eintraege: Vec<Eintrag> = Vec::new();
    for (index, zeile) in text.lines().enumerate() {
        let zeile = zeile.trim();
        if zeile.is_empty() || zeile.starts_with('#') {
            continue;
        }
        // Der Prompt ist das letzte Feld und darf selbst ein `|` enthalten —
        // `read -r a b c d` in den Skripten steckt den Rest ins vierte Feld.
        let mut felder = zeile.splitn(4, '|').map(str::trim);
        let slug = felder.next().unwrap_or_default();
        let name = felder.next().unwrap_or_default();
        let beschreibung = felder.next().unwrap_or_default();
        let prompt = felder.next().unwrap_or_default();
        if slug.is_empty() || prompt.is_empty() {
            bail!(
                "Zeile {} ({}) hat keinen Prompt — vier Felder mit | erwartet.",
                index + 1,
                if slug.is_empty() { "ohne slug" } else { slug }
            );
        }
        if eintraege.iter().any(|e| e.slug == slug) {
            bail!("Zeile {}: slug '{slug}' kommt doppelt vor.", index + 1);
        }
        eintraege.push(Eintrag {
            slug: slug.into(),
            name: name.into(),
            beschreibung: beschreibung.into(),
            prompt: prompt.into(),
        });
    }
    Ok(eintraege)
}

/// Schränkt eine Liste wie `--from` und `--only` der Batch-Skripte ein.
///
/// `ab` überspringt alles vor diesem slug; `nur` behält genau einen Eintrag.
/// Ein unbekannter slug ist ein Fehler — sonst liefe ein Tippfehler als
/// "nichts zu tun" durch.
pub fn eingrenzen(liste: Vec<Eintrag>, ab: Option<&str>, nur: Option<&str>) -> Result<Vec<Eintrag>> {
    for (name, slug) in [("from", ab), ("only", nur)] {
        if let Some(slug) = slug {
            if !liste.iter().any(|e| e.slug == slug) {
                bail!("{name}: slug '{slug}' steht nicht in der Liste.");
            }
        }
    }
    let mut angekommen = ab.is_none();
    let mut ergebnis = Vec::new();
    for eintrag in liste {
        if Some(eintrag.slug.as_str()) == ab {
            angekommen = true;
        }
        if !angekommen {
            continue;
        }
        if nur.is_some_and(|n| n != eintrag.slug) {
            continue;
        }
        ergebnis.push(eintrag);
    }
    Ok(ergebnis)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTE: &str = "\
# Kommentar
#   slug | Name | Beschreibung | Prompt

mira | Mira Aschengrau | Mädchen | young orphan girl, tangled hair
jonas | Jonas Steinweg | Junge | young orphan boy | mit Strich im Prompt
tobi | Tobi | Junge | little boy
";

    #[test]
    fn kommentare_und_leerzeilen_werden_uebersprungen() {
        let liste = lesen(LISTE).unwrap();
        let slugs: Vec<_> = liste.iter().map(|e| e.slug.as_str()).collect();
        assert_eq!(slugs, ["mira", "jonas", "tobi"]);
    }

    #[test]
    fn die_deutsche_beschreibung_wird_fuer_die_anzeige_mitgelesen() {
        let liste = lesen(LISTE).unwrap();
        assert_eq!(liste[0].beschreibung, "Mädchen");
    }

    #[test]
    fn nur_der_prompt_geht_ans_modell() {
        let liste = lesen(LISTE).unwrap();
        assert_eq!(liste[0].prompt, "young orphan girl, tangled hair");
        assert_eq!(liste[0].name, "Mira Aschengrau");
    }

    #[test]
    fn der_prompt_darf_einen_senkrechten_strich_enthalten() {
        let liste = lesen(LISTE).unwrap();
        assert_eq!(liste[1].prompt, "young orphan boy | mit Strich im Prompt");
    }

    #[test]
    fn zeile_ohne_prompt_ist_ein_fehler_mit_zeilennummer() {
        let fehler = lesen("# x\nmira | Mira | nur drei\n").unwrap_err().to_string();
        assert!(fehler.contains("Zeile 2") && fehler.contains("mira"), "{fehler}");
    }

    #[test]
    fn doppelter_slug_ist_ein_fehler() {
        let fehler = lesen("a | A | x | p1\na | B | y | p2\n").unwrap_err().to_string();
        assert!(fehler.contains("doppelt"), "{fehler}");
    }

    #[test]
    fn eingrenzen_ab_ueberspringt_alles_davor() {
        let liste = lesen(LISTE).unwrap();
        let rest = eingrenzen(liste, Some("jonas"), None).unwrap();
        let slugs: Vec<_> = rest.iter().map(|e| e.slug.as_str()).collect();
        assert_eq!(slugs, ["jonas", "tobi"]);
    }

    #[test]
    fn eingrenzen_nur_behaelt_genau_einen_eintrag() {
        let liste = lesen(LISTE).unwrap();
        let rest = eingrenzen(liste, None, Some("tobi")).unwrap();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].slug, "tobi");
    }

    #[test]
    fn eingrenzen_mit_unbekanntem_slug_ist_ein_fehler() {
        let liste = lesen(LISTE).unwrap();
        let fehler = eingrenzen(liste, None, Some("gibtsnicht")).unwrap_err().to_string();
        assert!(fehler.contains("gibtsnicht"), "{fehler}");
    }

    #[test]
    fn die_echten_listen_des_projekts_sind_lesbar() {
        // Die Listen liegen im Repo — ein kaputter Eintrag soll hier auffallen,
        // nicht erst nach Stunden mitten im Batch.
        for kind in Kind::ALLE {
            let pfad = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(standard_liste(kind));
            let text = std::fs::read_to_string(&pfad)
                .unwrap_or_else(|e| panic!("{} nicht lesbar: {e}", pfad.display()));
            let liste = lesen(&text).unwrap_or_else(|e| panic!("{}: {e}", pfad.display()));
            assert!(!liste.is_empty(), "{} ist leer", pfad.display());
        }
    }
}
