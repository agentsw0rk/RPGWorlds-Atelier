//! Freistell-Maske über u2netp (Salient Object Detection) mit `tract-onnx` —
//! reines Rust, keine C++-Laufzeit.
//!
//! u2netp sucht *ein* hervorstechendes Objekt. Sockel, Podeste und abgesetzte
//! Beigaben kommen deshalb als Hintergrund heraus; für Bilder mit bewusst
//! einfarbigem Hintergrund ist Farb-Keying (`keying`) die bessere Wahl.

use anyhow::{Context, Result};
use image::{GrayImage, Luma};
use std::path::Path;
use tract_onnx::prelude::*;

/// Eingabekantenlänge, auf die u2netp trainiert ist.
const NET_SIZE: usize = 320;

/// Lässt u2netp laufen und gibt die Maske in Originalauflösung zurück.
/// Alles bis `cutoff` wird hart auf 0 gezogen, damit der Hintergrund wirklich
/// alpha = 0 hat und nicht 3 oder 7.
pub fn maske(modell: &Path, rgb: &image::RgbImage, cutoff: u8) -> Result<GrayImage> {
    let model = tract_onnx::onnx()
        .model_for_path(modell)
        .with_context(|| format!("Modell {} nicht ladbar", modell.display()))?
        .with_input_fact(0, f32::fact([1, 3, NET_SIZE, NET_SIZE]).into())?
        .into_optimized()?
        .into_runnable()?;

    let small = image::imageops::resize(
        rgb,
        NET_SIZE as u32,
        NET_SIZE as u32,
        image::imageops::FilterType::Triangle,
    );

    // u2net normalisiert mit ImageNet-Statistik, nachdem es auf [0,1] skaliert hat.
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.229, 0.224, 0.225];
    let input = tract_ndarray::Array4::from_shape_fn((1, 3, NET_SIZE, NET_SIZE), |(_, c, y, x)| {
        let v = small.get_pixel(x as u32, y as u32)[c] as f32 / 255.0;
        (v - MEAN[c]) / STD[c]
    });

    let result = model.run(tvec!(Tensor::from(input).into()))?;
    // u2net hat mehrere Ausgänge (d0..d6); der erste ist die feinste Vorhersage.
    let pred = result[0].to_plain_array_view::<f32>()?;

    // Auf [0,1] normieren — die Rohausgabe ist nicht garantiert begrenzt.
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for &v in pred.iter() {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    let span = if (hi - lo).abs() < 1e-6 { 1.0 } else { hi - lo };

    let mut mask_small = GrayImage::new(NET_SIZE as u32, NET_SIZE as u32);
    for y in 0..NET_SIZE {
        for x in 0..NET_SIZE {
            let v = (pred[[0, 0, y, x]] - lo) / span;
            mask_small.put_pixel(x as u32, y as u32, Luma([(v * 255.0).round() as u8]));
        }
    }

    let (w, h) = rgb.dimensions();
    let mut mask = image::imageops::resize(&mask_small, w, h, image::imageops::FilterType::Lanczos3);
    for px in mask.pixels_mut() {
        if px[0] <= cutoff {
            px[0] = 0;
        }
    }
    Ok(mask)
}
