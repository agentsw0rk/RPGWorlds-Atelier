//! Bildnachbearbeitung für generierte Bilder: Freistellen und exaktes Skalieren —
//! dazu die Aufträge der Skripte als Rust-Daten (Arten, Listen, Modellsätze).

pub mod api;
pub mod auftrag;
pub mod demo;
pub mod dienst;
#[cfg(feature = "generate")]
pub mod engine;
pub mod job;
pub mod keying;
pub mod kind;
pub mod konfig;
pub mod lauf;
pub mod listen;
pub mod matting;
pub mod metriken;
pub mod modelle;
pub mod nachbearbeitung;
pub mod params;
pub mod referenzen;
pub mod saliency;
pub mod server;
pub mod zufall;
