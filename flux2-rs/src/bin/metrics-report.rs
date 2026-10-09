//! Liest eine `metrics.jsonl` und druckt einen Bericht: Einstellungen, Speicher
//! je Phase und die Phase, in der der Lauf endet.
//!
//! ```sh
//! metrics-report api-daten/jobs/<id>/metrics.jsonl
//! ```

use anyhow::{bail, Result};
use flux2_rs::metriken::{bericht, eintraege_lesen};
use std::path::PathBuf;

fn main() -> Result<()> {
    let Some(pfad) = std::env::args().nth(1).map(PathBuf::from) else {
        bail!("Aufruf: metrics-report <metrics.jsonl>");
    };
    let strom = eintraege_lesen(&pfad)?;
    if strom.is_empty() {
        bail!("{} enthält keine Einträge.", pfad.display());
    }
    print!("{}", bericht(&strom));
    Ok(())
}
