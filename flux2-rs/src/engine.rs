//! Die echte Bilderzeugung über `diffusion-rs` (Rust-Bindings zu
//! stable-diffusion.cpp). Nur mit dem Feature `generate`.
//!
//! Das Modell besteht aus drei getrennt geladenen Teilen: Diffusion-Transformer
//! (GGUF), Text-Encoder (Qwen3, GGUF — FLUX.2 nutzt ein LLM statt CLIP/T5) und
//! VAE. `diffusion-rs` baut den Modellkontext bei **jedem** `gen_img`-Aufruf
//! neu auf, mehrere Seeds laden die Gewichte also jedes Mal neu — wie bei den
//! Skripten, die pro Seed einen eigenen Prozess starten.

use crate::lauf::{Engine, Erzeugung, Host};
use crate::modelle::Modellsatz;
use crate::params::gewichts_typ;
use anyhow::{Context, Result};
use diffusion_rs::api::{gen_img, ConfigBuilder, ModelConfigBuilder, SampleMethod, WeightType};
use diffusion_rs_sys::{sd_log_level_t, sd_set_log_callback};
use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Schwelle für die sd.cpp-Logausgabe: 0 = aus, 1 = ab INFO, 2 = mit DEBUG.
static LOG_LEVEL: AtomicUsize = AtomicUsize::new(1);

/// Logzeilen von stable-diffusion.cpp nach stderr durchreichen.
///
/// Ohne gesetzten Callback verwirft `log_printf` in sd.cpp jede Zeile — dann
/// sieht man weder das benutzte Backend noch Warnungen des VAE, und ein
/// halbstündiger Lauf endet kommentarlos in einem grauen Bild.
unsafe extern "C" fn sd_log(level: sd_log_level_t, text: *const c_char, _data: *mut c_void) {
    if text.is_null() {
        return;
    }
    let zeile = CStr::from_ptr(text).to_string_lossy();
    // Die Metriken lesen die fertigen Stufen mit, unabhängig von `LOG`.
    if level != sd_log_level_t::SD_LOG_DEBUG {
        crate::metriken::sd_log_weiterleiten(&zeile);
    }
    let log = LOG_LEVEL.load(Ordering::Relaxed);
    if log == 0 || (level == sd_log_level_t::SD_LOG_DEBUG && log < 2) {
        return;
    }
    // sd.cpp hängt selbst ein \n an, deshalb eprint! statt eprintln!.
    eprint!("[sd] {zeile}");
}

pub struct SdEngine;

impl SdEngine {
    /// `log`: 0 = still, 1 = sd.cpp-Log, 2 = zusätzlich DEBUG.
    pub fn neu(log: usize) -> Self {
        LOG_LEVEL.store(log, Ordering::Relaxed);
        // Immer setzen: ohne Callback verwirft sd.cpp jede Zeile, und die Metriken
        // bekämen keine Stufen. Gedruckt wird nur ab `log > 0`.
        // Sicher: der Callback hält keinen eigenen Zustand.
        unsafe { sd_set_log_callback(Some(sd_log), std::ptr::null_mut()) };
        SdEngine
    }
}

fn gewichtstyp(name: &str) -> Result<WeightType> {
    Ok(match gewichts_typ(name)?.as_str() {
        "f32" => WeightType::SD_TYPE_F32,
        "f16" => WeightType::SD_TYPE_F16,
        "q8_0" => WeightType::SD_TYPE_Q8_0,
        "q6_k" => WeightType::SD_TYPE_Q6_K,
        "q5_k" => WeightType::SD_TYPE_Q5_K,
        "q4_k" => WeightType::SD_TYPE_Q4_K,
        "q3_k" => WeightType::SD_TYPE_Q3_K,
        _ => unreachable!("gewichts_typ hat den Wert bereits geprüft"),
    })
}

impl Engine for SdEngine {
    fn erzeuge(&mut self, satz: &Modellsatz, host: &Host, e: &Erzeugung) -> Result<()> {
        let mut modell = ModelConfigBuilder::default();
        modell
            .diffusion_model(satz.dit.pfad.clone())
            .llm(satz.llm.pfad.clone())
            .vae(satz.vae.pfad.clone())
            .enable_mmap(host.mmap)
            .diffusion_flash_attention(host.flash_attention)
            .flash_attention(host.flash_attention)
            .vae_tiling(host.vae_tiling)
            .n_threads(host.threads);
        // Gewichte beim Laden umwandeln — nötig für fp8-Modelle: sd.cpp rechnet
        // fp8 beim Einlesen auf f16 hoch, aus 9,5 GB würden rund 19 GB.
        if let Some(name) = e.wtype {
            modell.weight_type(gewichtstyp(name)?);
        }
        let mut modell = modell.build().map_err(|f| anyhow::anyhow!("ModelConfig: {f}"))?;

        let mut config = ConfigBuilder::default();
        config
            .prompt(e.prompt.to_string())
            .cfg_scale(e.cfg)
            .guidance(e.guidance)
            .steps(e.steps as i32)
            .width(e.width as i32)
            .height(e.height as i32)
            .sampling_method(SampleMethod::EULER_SAMPLE_METHOD)
            .seed(e.seed)
            .output(e.ziel.to_path_buf());
        if !e.refs.is_empty() {
            config.ref_images(e.refs.to_vec());
            // Namensdreher in diffusion-rs 0.1.20: `disable_auto_resize_ref_image`
            // wird unverändert an sd.cpps `auto_resize_ref_image` durchgereicht.
            // `true` schaltet die Skalierung also *ein* — und die braucht man, wenn
            // nichts anderes die Größe begrenzt: ohne sie ginge ein 12-MP-Foto in
            // Originalauflösung durch den VAE. sd.cpp bringt jede Referenz dabei auf
            // genau 1 MP (auch aufwärts). Hat der Ablauf die Referenzen schon auf eine
            // Obergrenze gebracht (`ref_max_px`), muss sie *aus* bleiben, sonst bläst
            // sd.cpp sie wieder auf.
            config.disable_auto_resize_ref_image(!e.ref_selbst_skaliert);
        }
        let config = config.build().map_err(|f| anyhow::anyhow!("Config: {f}"))?;

        crate::metriken::sd_log_ziel_setzen(e.metriken.cloned());
        let ergebnis = gen_img(&config, &mut modell).context("gen_img");
        crate::metriken::sd_log_ziel_setzen(None);
        ergebnis?;
        Ok(())
    }
}
