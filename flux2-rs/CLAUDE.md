# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Environment setup (required first)

The Rust toolchain and C/C++ toolchain are **not on the default PATH**. Every shell
that builds or tests must source the environment first:

```sh
source /workspace/tools/envrc
```

It sets `CARGO_HOME`/`RUSTUP_HOME` under `/workspace/tools`, points `CC`/`CXX`/`AR`/… at
the conda toolchain, and sets `LIBCLANG_PATH` (bindgen) plus `CMAKE_GENERATOR=Ninja`
(there is no `make` in this container).

## Never run image generation (hard rule)

**Do not execute anything that produces an image — not as a test, not as a "quick
check", not to verify a change.** This container is a development environment only:
the work here is writing and building code, never running the generator.

- ❌ `./target/release/flux2-rs "..."` and every variant of it
- ❌ `cargo run` / `cargo test` with the `generate` feature enabled
- ❌ any script under `scripts/` that ends in a rendered `*.png`
- ❌ "just one 256² image with 1 step to see if it works"

Why: CPU-only Docker with ~5 GB RAM — a single run takes many minutes per step, is the
main OOM source in this container, and proves nothing that a build or a unit test does
not prove faster.

Verify changes with these instead:

```sh
source /workspace/tools/envrc
cargo test --no-default-features     # matting unit tests
cargo check                          # type-check the generator without running it
cargo build --release                # compile only
```

The `matte` binary does not generate images — it only transforms an existing PNG — and
may be run on an existing sample file.

If a change really can only be validated by generating an image, say so and let the user
run it; never start such a run yourself.

## Commands

```sh
# Fast loop: matting code + its tests only, no C++ build (seconds)
cargo test --no-default-features
cargo test --no-default-features matting::tests::einpassen   # single test / filter

# Full build — pulls in diffusion-rs → stable-diffusion.cpp (long C++/CMake build)
cargo build --release

# Second model set (host only): non-distilled 9B, gated repo, needs HF_TOKEN
scripts/generate-macos.sh --preset klein-base-9b --wtype q8_0 "prompt"

# Generate an image — REFERENCE ONLY, do not run (see rule above); the user runs this
# (all parameters are env vars, there are no CLI flags)
SIZE=512 STEPS=4 SEED=42 OUT=/workspace/out.png ./target/release/flux2-rs "prompt"

# Build only (host); --tests adds the fast unit tests, --clean wipes target/
scripts/build.sh --tests

# Whole flow on a macOS host — fetch models, build, generate, cut out the subject.
# REFERENCE ONLY, never run here: the last step generates an image (and the first
# downloads several GB).
scripts/generate-macos.sh -r figur.png -o token.png --out-w 138 --out-h 244 "prompt"

# Cut out the subject and fit it to a fixed canvas
MODEL=/workspace/models/matting/u2netp.onnx OUT_W=138 OUT_H=244 \
  ./target/release/matte in.png out.png
```

`cargo build` / `cargo test` without `--no-default-features` enables the `generate`
feature and triggers the sd.cpp build — only do that when the generator itself changed.

## Architecture

Two independent binaries forming a two-stage pipeline; `src/lib.rs` exposes only the
pure, testable half.

- **`src/main.rs` (`flux2-rs`, requires feature `generate`)** — text→image via
  `diffusion-rs` (Rust bindings over `stable-diffusion.cpp`). FLUX.2 is loaded as three
  separate files: the 4B diffusion transformer (GGUF), **Qwen3-4B as text encoder**
  (GGUF — FLUX.2 uses an LLM, not CLIP/T5), and the VAE (safetensors). Model paths come
  from the `MODELS` const (`/workspace/models`, i.e. **outside this crate**) and can be
  overridden via `DIT`/`LLM`.
- **`src/bin/matte.rs` (`matte`)** — runs u2netp (salient object detection) through
  `tract-onnx`, pure Rust, no C++ runtime. Produces the mask, then calls into the library.
- **`src/matting.rs`** — the library half, deliberately free of I/O and of model code:
  `apply_mask_as_alpha` → `crop_to_content` → `fit_into`. All unit tests live here; this
  is why the matting path is testable without building sd.cpp.
- **`src/keying.rs`** — the colour-keying half of `matte`: border-median background colour
  plus a flood fill from the image border, with a soft tolerance band for the edges. Pure
  functions, unit-tested, no I/O.
- **`src/params.rs`** — env-var parsing shared by both binaries (`REF` list, the
  divisible-by-16 check, `OUT_W`/`OUT_H` → `Canvas`). Pure functions, unit-tested; a
  missing `REF` path is a hard error because `diffusion-rs` would otherwise skip it
  silently and run for minutes without the reference.
- **`scripts/build.sh`** — build only, nothing else: `--tests` runs the fast unit tests
  first, `--clean` wipes `target/`. `generate-macos.sh` now builds unconditionally before
  every run (cargo no-ops when nothing changed), so `--rebuild` is a no-op kept only for
  old invocations; use `build.sh` when you want to build without generating.
- **`scripts/umgebung.sh`** — sourced by both scripts (not executable on its own): tool
  check, `LIBCLANG_PATH`/`CMAKE_GENERATOR`, thread count, and `bauen()` with the bindgen
  recovery. Changes to the toolchain search belong here, not in a caller.
- **Model directory resolution must agree across entry points.** `standard_modelle()` in
  `umgebung.sh` picks `<project>/models` (sibling of this repo) if it exists, else
  `<repo>/models`; both `generate-macos.sh` and `token.sh` call it for their default. They
  used to disagree — `generate-macos.sh` always defaulted to `<repo>/models` while
  `token.sh` preferred the sibling — which silently downloaded the same multi-GB weights
  into two places depending on which script was invoked directly. Fixed by centralizing
  the lookup; don't reintroduce a second copy of this logic.
- **"Childish" is fixed by head-to-body ratio, not by negation.** `not chibi` encodes
  *chibi*. The token STYLE states the number instead — `about seven and a half heads tall,
  head small relative to the body, mature adult face with defined cheekbones and jaw`.
  Watch the small folk and the child entries in `charaktere.txt`: their character line
  leads the prompt and usually wins, but `--style` is the escape hatch for those few.
- **A turned pose costs anatomy.** Feet and hands break first in diffusion models, and a
  twisted torso with the head counter-rotated is the hardest case — reversed boots are the
  usual result. The token POSE turns the head *with* the body and names the feet
  explicitly; more steps (12-16) and a re-rolled seed are the other levers.
- **Camera angle needs plain words, not jargon.** `30-degree elevated three-quarter view`
  in the token STYLE produced eye-level front views every time. What works is saying the
  two things separately in common phrasing: `high angle view looking down on the figure`
  for the camera and `body turned three-quarters away from the viewer` for the pose. A
  reference image via `-r` transfers the angle far more reliably than any wording.
- **Three model sets, `--preset`.** A preset pins the diffusion model **and** its text
  encoder together: `klein-4b` (Qwen3-4B), `klein-9b` (Qwen3-8B, distilled GGUF) and
  `klein-base-9b` (Qwen3-8B, non-distilled fp8, gated). The 9B checkpoints ship a ~8B
  encoder (`text_encoder/` in the BFL repo is 16.4 GB of bf16); pairing a 9B model with the
  4B encoder is a silent mismatch. `--llm FILE` overrides it for the Mistral case below.
  Two traps live here: sd.cpp expands fp8 to f16 while loading
  (`model_loader.cpp: f8_e4m3_to_f16_vec`), so 9.5 GB on disk become ~19 GB in memory —
  the preset therefore defaults to `WTYPE=q8_0`, which quantizes at load time. And the
  text encoder is chosen by sd.cpp from the block count, not by us: it logs
  `Version: Flux.2 klein` (Qwen3) or `Version: Flux.2` (Mistral Small 3.2). Only the first
  run tells you which one a given checkpoint needs.
- **`HF_TOKEN`** is sent as a bearer token on every download; gated repos return an HTML
  error page instead of weights without it, and `fetch` says so explicitly on failure.
- **`charaktere.txt` + `scripts/batch.sh`** — the whole asset library in one run. The list is
  `slug | Name | German description | English prompt`; only the prompt reaches the model.
  Resume is file-based: an existing `<out-dir>/<slug>.png` counts as done, so re-running
  continues where it stopped. The INT/TERM trap deletes the in-flight file first — otherwise
  a truncated image would be treated as finished and the figure would silently be missing.
- **`orte.txt` + `scripts/location-batch.sh`** — same idea as `charaktere.txt`/`batch.sh`
  but for `location.sh`: 77 D&D locations (Forgotten Realms cities, Underdark, Ravenloft,
  Eberron, Sigil, generic terrain for visual variety), same `slug | Name | German
  description | English prompt` format, same resume-by-existing-file and INT/TERM cleanup
  logic. Currently a separate copy of the batch driver rather than a shared library —
  keep the two in sync by hand until they're unified.
- **`charaktere_portraits.txt` + `scripts/portrait.sh` + `scripts/portrait-batch.sh`** —
  third pair on the same pattern: head-and-shoulders portraits instead of full-body tokens.
  `portrait.sh` hardcodes `--keep-bg` like `location.sh` (the parchment background is part
  of the image, not a cutout backdrop) and defaults to a 1024x1024 canvas. `portrait-batch.sh`
  is a third copy of the batch driver, same caveat as above. Note: `charaktere_portraits.txt`'s
  rows already end their own prompt with negations (`no scenery, no text, no border`) baked
  in by whatever produced the list — `PORTRAIT_STYLE` itself avoids that mistake, but the
  per-row text is untouched; fixing it means editing the 450 rows, not the script.
  Verified on a real generated portrait: without an explicit frame-fill instruction the
  subject rendered small and centered with a wide margin of parchment on all sides —
  `head and shoulders` only says what's shown, not how tightly it's cropped, and the
  rows' own `centered composition`/`simple uncluttered composition` (token vocabulary,
  where clearance around the figure is wanted) reinforces the same spacious default.
  `PORTRAIT_STYLE` now says explicitly `tight close-up crop filling almost the entire
  frame, subject cropped at the shoulders and just above the top of the head`.
- **`scripts/token.sh`** — thin wrapper over the driver for character-token batches: STYLE/POSE
  live in the script as variables (identical prompt text across runs is the whole point),
  the character comes in as the argument, and every unknown flag is forwarded to
  `generate-macos.sh` **after** its own defaults, so the caller's value wins. `-n` prints
  the assembled call instead of running it.
- **`scripts/location.sh`** — same shape as `token.sh` but for full-scene location
  illustrations (villages, ruins, landscapes) instead of cutout character tokens: fixed
  STYLE, the place description as the argument, `--large` toggles 1024x512/1536x768.
  Always passes `--keep-bg` — a location is a complete scene, not a subject to matte out.
- **`scripts/generate-macos.sh`** — the host-side driver (download → build → generate →
  matte). All of its argument validation happens before the first model is touched, so
  `--help` and bad-argument paths are safe to exercise anywhere; everything after the
  "Umgebung" section belongs on the Mac.

## Conventions

- **Doc comments, code comments and test names are German.** Keep new code consistent
  with that; test names read as assertions (`einpassen_liefert_exakt_die_zielgroesse`).
- Configuration is environment variables only — no arg parser. New knobs follow the
  `std::env::var(...).parse().ok().unwrap_or(default)` pattern (see `env_num` in `matte.rs`).
- `WIDTH`/`HEIGHT` must be divisible by 16; `main.rs` bails out otherwise.
- `klein` is a distilled model: `cfg_scale = 1.0` and ~4 steps. The non-distilled
  `FLUX.2-klein-base-4B` would need `cfg_scale ≈ 4.0` and ~20 steps.

## Gotchas that will cost you hours

- **The bindgen `_IO_FILE` layout assert is patched in the build output, not in source.**
  With clang ≥ 23, `diffusion-rs-sys` 0.1.20 generates `size_of::<_IO_FILE>() - 216usize`
  against an opaque (size-1) type → `error[E0080]`. The current working build has those
  layout asserts stripped from
  `target/release/build/diffusion-rs-sys-*/out/bindings.rs`.
  A `cargo clean`, a dependency bump, or anything else that re-runs that build script
  **brings the error back**. The durable fix is to vendor `diffusion-rs-sys` and add
  `.layout_tests(false)` to its `bindgen::Builder` (`build.rs` ~line 37), or use an older
  libclang. See README.md "Bekannte Stolpersteine".
- **u2netp is a salient-object detector, not a background remover.** It segments *the one*
  prominent subject. Pedestals, plinths, cast shadows and props standing apart from the
  figure come out as background — verified on a 1024² token: the sandstone base is 0 in the
  mask, so `CUTOFF` is irrelevant there (cutoff 0/12/40 all keep it out). `MASK_OUT=<png>`
  in `matte` dumps the raw mask and settles such questions in one run. For images with a
  genuinely uniform background, colour keying would fit this pipeline better than saliency.
- **`matte` has two mask paths.** Default is u2netp saliency; `BG_KEY=auto|rrggbb` switches
  to colour keying — a border-seeded flood fill over pixels close to the background colour
  (`src/keying.rs`). Keying keeps bases, plinths and props that saliency drops, needs no
  ONNX model and is ~20x faster, but assumes a uniform background. `KEY_AUSSEN` above ~98
  lets the fill pass through light skin tones and punches holes in faces — measured, not
  guessed; `MASK_OUT` shows it immediately.
- **Reference images lose their alpha to black.** `diffusion-rs` passes ref images through
  `img.to_rgb8()` (`api.rs`), which drops the alpha channel — a transparent PNG (e.g. a
  token this pipeline produced earlier) reaches the model as a subject on **black**.
  `src/main.rs` therefore composites any ref with alpha onto `REF_BG` (default `ffffff`)
  into a temp file first; `matting::auf_hintergrund` does the blend and is unit-tested.
- **Border-seeded keying alone leaves gaps opaque.** The flood fill starts at the image
  border, so a gap between the legs that the base closes off below is never reached.
  `src/keying.rs` therefore runs a second pass over enclosed areas, gated by **two**
  conditions: near-exact background colour (`KEY_LOCH`, 12) *and* a minimum area
  (`KEY_LOCH_MIN`, 500 px). Both are needed — measured on 1024² tokens: with the colour
  test alone, steel highlights (which hit the grey background colour exactly) punch holes
  through beard, armour and base. The colour threshold has to stay tight: at 30, grey
  shading painted onto a pale stone base counted as background and was cut out. A real gap
  shows the background itself and matches its colour almost exactly.
- **Despill runs by default when keying.** Soft edges are blends of subject and background;
  `keying::despill` inverts `c = a·fg + (1-a)·key` per semi-transparent pixel, leaving
  anything below 3 % alpha alone. Only possible with `BG_KEY` — u2netp never learns a
  background colour. `DESPILL=0` turns it off. This is what makes a saturated key colour
  (magenta, chroma green) viable at all; without it a coloured fringe survives in the alpha
  edge of every token.
- **`enable_mmap` breaks every GPU backend.** With mmap, sd.cpp binds the weights to a
  **CPU** buffer (`model_loader.cpp`: `ggml_backend_cpu_buffer_from_ptr`). Metal/CUDA then
  log `ggml_metal_buffer_get_id: error: tensor '…' buffer is nil` for every tensor, compute
  with zeros, and write a uniform grey image — no error, exit code 0. Verified on an M1 Pro
  with FLUX.2 klein Q5_K_M. `MMAP=0` fixes it; keep it on here (CPU-only, RAM-bound).
- **sd.cpp writes no log unless you install a callback.** `log_printf`
  (`stable-diffusion.cpp/src/core/util.cpp`) drops every line while `sd_log_cb` is null,
  and `diffusion-rs` never sets one. `src/main.rs` installs it via `diffusion-rs-sys`
  (direct dependency for exactly this); `LOG=0` silences it, `LOG=2` adds DEBUG. Without
  it a half-hour run gives no backend, no VAE warning, no model-type detection.
- **Flash attention silently corrupts output on unsupported combinations.** Both
  `flash_attention` flags default to `false` upstream — the crate docs say "only supported
  for some models and some backends" and that it slows most backends down. Enabled where
  it is not supported, the run produces a uniform grey/black image instead of an error.
  Here it stays on (RAM), `scripts/generate-macos.sh` defaults it off.
- **RAM, not resolution, is the limit on this CPU-only box (~5 GB free).** DiT and text
  encoder are resident simultaneously; halving the resolution does not help, lowering
  quantization does. Q3_K_M + Q4_K_M OOMs even at 256²; Q2_K + Q2_K works. `enable_mmap`,
  `vae_tiling` and flash attention are on in `main.rs` for exactly this reason — slower on
  CPU, much lower peak memory.
- Generation is slow here: ~2 min per step at 288×512, plus ~50 s VAE decode. Do not
  assume a run is hung.
- Model weights are large binaries in `/workspace/models` and are not part of this crate;
  see README.md for the download commands.
