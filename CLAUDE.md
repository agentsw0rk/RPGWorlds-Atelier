# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository layout

The only tracked content in this git repo is `flux2-rs/` — a Rust project combining FLUX.2
image generation (via `diffusion-rs`/stable-diffusion.cpp) with a pure-Rust background
removal/keying pipeline — plus this file and `.gitignore`. Everything else at this top
level is generated or local data, excluded via `.gitignore`:

- `models/` — multi-GB GGUF/safetensors model weights, kept outside the crate and shared
  across checkouts.
- `tokens/` — character-token PNGs produced by `flux2-rs/scripts/batch.sh` from
  `flux2-rs/charaktere.txt` (a library of several hundred D&D-style characters).
- `scratch/`, `.idea/`, stray `*.png`/`*.raw.png`/`*.log` files — ad-hoc run output and
  editor state.

## Where to look

All environment setup, build/test commands, and architecture notes live in `flux2-rs/`:

- **`flux2-rs/CLAUDE.md`** — read this before touching anything in `flux2-rs/`. Required
  environment setup, a hard rule against running image generation in this container, the
  full build/test/architecture reference, and a long list of hard-won gotchas (mmap+Metal,
  sd.cpp's silent logging, keying/despill, model presets).
- **`flux2-rs/README.md`** — user-facing docs: model downloads and the macOS host scripts
  (`generate-macos.sh`, `token.sh`, `batch.sh`, `build.sh`).

Keep guidance in those two files current — don't duplicate it here.
