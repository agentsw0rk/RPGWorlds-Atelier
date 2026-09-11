//! FLUX.2 [klein] 4B Text-zu-Bild in Rust — ohne Python.
//!
//! Nutzt `diffusion-rs` (Rust-Bindings zu stable-diffusion.cpp).
//! Das Modell besteht aus drei Teilen, die getrennt geladen werden:
//!   * Diffusion-Transformer (4B, GGUF-quantisiert)
//!   * Text-Encoder: Qwen3-4B (GGUF) — FLUX.2 nutzt ein LLM statt CLIP/T5
//!   * VAE zum Dekodieren der Latents
use anyhow::{Context, Result};
use diffusion_rs::api::{gen_img, ConfigBuilder, ModelConfigBuilder, SampleMethod};
use diffusion_rs_sys::{sd_log_level_t, sd_set_log_callback};
use flux2_rs::matting::auf_hintergrund;
use flux2_rs::params::{check_multiple_of_16, hex_farbe, ref_image_paths};
use std::ffi::{c_char, c_void, CStr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

fn main() -> Result<()> {
    // Logausgabe von sd.cpp anschalten, bevor irgendetwas geladen wird —
    // sonst läuft die Generierung minutenlang ohne jede Rückmeldung.
    let log_level: usize = env_num("LOG").unwrap_or(1);
    LOG_LEVEL.store(log_level, Ordering::Relaxed);
    if log_level > 0 {
        // Sicher: der Callback schreibt nur nach stderr und hält keinen Zustand.
        unsafe { sd_set_log_callback(Some(sd_log), std::ptr::null_mut()) };
    }

    let prompt = std::env::args().nth(1).unwrap_or_else(|| {
        "a red panda sitting on a moss-covered rock in a misty forest, \
         soft morning light, highly detailed photograph"
            .to_string()
    });
    // Modellverzeichnis ist konfigurierbar, damit derselbe Build im Container
    // (/workspace/models) und auf einem Host-Rechner läuft.
    let models = std::env::var("MODELS_DIR").unwrap_or_else(|_| "/workspace/models".into());

    // Auflösung klein halten, wenn RAM der Engpass ist.
    // SIZE setzt beide Kanten (quadratisch), WIDTH/HEIGHT überschreiben einzeln.
    let size: i32 = env_num("SIZE").unwrap_or(512);
    let width: i32 = env_num("WIDTH").unwrap_or(size);
    let height: i32 = env_num("HEIGHT").unwrap_or(size);
    check_multiple_of_16("WIDTH", width)?;
    check_multiple_of_16("HEIGHT", height)?;

    let steps: i32 = env_num("STEPS").unwrap_or(4);
    // Fester Default, damit Läufe reproduzierbar sind; SEED=-1 würfelt pro Lauf neu.
    let seed: i64 = env_num("SEED").unwrap_or(42);
    let threads: i32 = env_num("THREADS").unwrap_or(8);
    let out = PathBuf::from(std::env::var("OUT").unwrap_or_else(|_| "/workspace/out.png".into()));

    // REF: In-Context-Referenz (FLUX.2-Edit-Modus) — mehrere Bilder kommagetrennt.
    // INIT: klassisches img2img, STRENGTH steuert, wie stark übermalt wird.
    // klein ist distilliert: cfg_scale 1.0 (kein Classifier-Free Guidance).
    // GUIDANCE ist die *destillierte* Guidance, die FLUX als Eingang bekommt —
    // ein anderer Wert als cfg_scale, sd.cpps Default ist 3.5.
    let cfg_scale: f32 = env_num("CFG").unwrap_or(1.0);
    let guidance: f32 = env_num("GUIDANCE").unwrap_or(3.5);

    let refs = ref_image_paths(&std::env::var("REF").unwrap_or_default())?;
    // REF_BG: Hintergrund, auf den transparente Referenzbilder gelegt werden.
    let ref_bg = hex_farbe(&std::env::var("REF_BG").unwrap_or_else(|_| "ffffff".into()))?;
    let refs = refs_deckend_machen(refs, ref_bg)?;
    let init = std::env::var("INIT").unwrap_or_default();
    let strength: f32 = env_num("STRENGTH").unwrap_or(0.75);

    println!(
        "Prompt: {prompt}\nGröße: {width}x{height}, Steps: {steps}, Seed: {seed}, Threads: {threads}\ncfg_scale: {cfg_scale}, guidance: {guidance}\nZiel: {}",
        out.display()
    );
    if !refs.is_empty() {
        println!("Referenzbilder: {}", refs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "));
    }
    if !init.is_empty() {
        println!("img2img-Vorlage: {init} (strength {strength})");
    }

    let mut model_config = ModelConfigBuilder::default()
        .diffusion_model(PathBuf::from(std::env::var("DIT").unwrap_or_else(|_| {
            format!("{models}/diffusion/flux-2-klein-4b-Q3_K_M.gguf")
        })))
        .llm(PathBuf::from(std::env::var("LLM").unwrap_or_else(|_| {
            format!("{models}/text_encoder/Qwen3-4B-Q4_K_M.gguf")
        })))
        .vae(PathBuf::from(std::env::var("VAE").unwrap_or_else(|_| {
            format!("{models}/vae/flux2-vae.safetensors")
        })))
        // mmap statt Kopie in den Heap: entscheidend bei ~5 GB freiem RAM.
        .enable_mmap(env_flag("MMAP", true))
        // Flash-Attention auf beiden Pfaden und gekacheltes VAE-Decoding:
        // auf CPU langsamer, aber deutlich sparsamer im Peak-Speicher.
        // Mit viel (V)RAM lohnt VAE_TILING=0 — das spart den Kachel-Overhead.
        .diffusion_flash_attention(env_flag("FLASH_ATTENTION", true))
        .flash_attention(env_flag("FLASH_ATTENTION", true))
        .vae_tiling(env_flag("VAE_TILING", true))
        .n_threads(threads)
        .build()
        .map_err(|e| anyhow::anyhow!("ModelConfig: {e}"))?;

    let mut config = ConfigBuilder::default();
    config
        .prompt(prompt)
        .cfg_scale(cfg_scale)
        .guidance(guidance)
        .steps(steps)
        .width(width)
        .height(height)
        .sampling_method(SampleMethod::EULER_SAMPLE_METHOD)
        .seed(seed)
        .output(out.clone());

    if !refs.is_empty() {
        config.ref_images(refs);
        // Achtung, Namensdreher in diffusion-rs 0.1.20: `disable_auto_resize_ref_image`
        // wird unverändert an sd.cpps `auto_resize_ref_image` durchgereicht
        // (api.rs:1567). `true` schaltet die Skalierung also *ein* — und die braucht
        // man: ohne sie wird das Referenzbild in Originalauflösung durch den VAE
        // geschickt, ein 12-MP-Foto sprengt damit den Speicher.
        config.disable_auto_resize_ref_image(env_flag("REF_RESIZE", true));
    }
    if !init.is_empty() {
        config.init_img(PathBuf::from(&init)).strength(strength);
    }

    let config = config.build().map_err(|e| anyhow::anyhow!("Config: {e}"))?;

    gen_img(&config, &mut model_config).context("Bildgenerierung fehlgeschlagen")?;
    println!("Fertig: {}", out.display());
    Ok(())
}

fn env_num<T: std::str::FromStr>(key: &str) -> Option<T> {
    std::env::var(key).ok().and_then(|v| v.parse().ok())
}

/// Schaltervariable: `0`, `false` und `off` schalten ab, alles andere ein.
fn env_flag(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off" | "no"),
        Err(_) => default,
    }
}

/// Schwelle für die sd.cpp-Logausgabe: 0 = aus, 1 = ab INFO, 2 = mit DEBUG.
static LOG_LEVEL: AtomicUsize = AtomicUsize::new(1);

/// Logzeilen von stable-diffusion.cpp nach stderr durchreichen.
///
/// Ohne gesetzten Callback verwirft `log_printf` in sd.cpp jede Zeile
/// (`core/util.cpp`: `if (sd_log_cb)`) — dann sieht man weder das benutzte
/// Backend noch Warnungen des VAE, und ein halbstündiger Lauf endet
/// kommentarlos in einem grauen Bild.
unsafe extern "C" fn sd_log(level: sd_log_level_t, text: *const c_char, _data: *mut c_void) {
    if text.is_null() {
        return;
    }
    if level == sd_log_level_t::SD_LOG_DEBUG && LOG_LEVEL.load(Ordering::Relaxed) < 2 {
        return;
    }
    // sd.cpp hängt selbst ein \n an, deshalb eprint! statt eprintln!.
    eprint!("[sd] {}", CStr::from_ptr(text).to_string_lossy());
}

/// Ersetzt Referenzbilder mit Alphakanal durch eine deckende Fassung.
///
/// `diffusion-rs` reicht Referenzbilder mit `to_rgb8()` weiter und wirft Alpha
/// dabei weg — transparente Flächen kommen im Modell als **Schwarz** an. Ein
/// bereits freigestelltes Token als Vorlage sagt dem Modell also "schwarzer
/// Hintergrund". Deshalb legen wir es vorher selbst auf eine Fläche.
fn refs_deckend_machen(refs: Vec<PathBuf>, hintergrund: [u8; 3]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::with_capacity(refs.len());
    for (i, pfad) in refs.into_iter().enumerate() {
        let bild = image::open(&pfad)
            .with_context(|| format!("Referenzbild {} nicht lesbar", pfad.display()))?;
        let rgba = bild.to_rgba8();
        if rgba.pixels().all(|px| px[3] == 255) {
            out.push(pfad);
            continue;
        }
        let deckend = auf_hintergrund(&rgba, image::Rgb(hintergrund));
        let ziel = std::env::temp_dir().join(format!("flux2-ref-{i}.png"));
        deckend
            .save(&ziel)
            .with_context(|| format!("{} nicht schreibbar", ziel.display()))?;
        println!(
            "Referenzbild {} hat Alpha — auf #{:02x}{:02x}{:02x} gelegt: {}",
            pfad.display(),
            hintergrund[0],
            hintergrund[1],
            hintergrund[2],
            ziel.display()
        );
        out.push(ziel);
    }
    Ok(out)
}
