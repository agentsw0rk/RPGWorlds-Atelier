# flux2-rs — FLUX.2 [klein] 4B in Rust, ohne Python

Text-zu-Bild mit [FLUX.2-klein-4B](https://huggingface.co/black-forest-labs/FLUX.2-klein-4B)
als reines Rust-Programm.

## Warum nicht candle?

[candle](https://github.com/huggingface/candle), das Rust-ML-Framework von Hugging Face,
unterstützt **nur FLUX.1** (`schnell`/`dev`) — siehe
`candle-transformers/src/models/flux/`, dessen Beispiel nur `Model::Schnell | Model::Dev`
mit T5-/CLIP-Encodern kennt. FLUX.2 hat eine andere Architektur (LLM statt T5/CLIP als
Text-Encoder) und ist dort nicht implementiert.

Dieses Projekt nutzt daher [`diffusion-rs`](https://github.com/newfla/diffusion-rs) —
eine Rust-API über [`stable-diffusion.cpp`](https://github.com/leejet/stable-diffusion.cpp),
das FLUX.2 klein offiziell unterstützt. Der Anwendungscode ist Rust, die Rechenkernel sind
C++ (ggml). Kein Python im Spiel.

## Modell-Aufbau

FLUX.2 besteht aus drei separat geladenen Teilen:

| Teil | Datei | Größe |
|---|---|---|
| Diffusion-Transformer (4B) | `unsloth/FLUX.2-klein-4B-GGUF` → `flux-2-klein-4b-Q3_K_M.gguf` | 2,1 GB |
| Text-Encoder: **Qwen3-4B** | `unsloth/Qwen3-4B-GGUF` → `Qwen3-4B-Q4_K_M.gguf` | 2,4 GB |
| VAE | `unsloth/FLUX.2-VAE` → `split_files/vae/flux2-vae.safetensors` | 336 MB |

Der Text-Encoder ist ein **unverändertes Qwen3-4B** — FLUX.2 nutzt ein LLM statt CLIP/T5.
Das ist der Grund, warum das Modell trotz "4B" im Namen ~8B Parameter lädt.

## Download

```sh
mkdir -p models/{diffusion,text_encoder,vae}
B=https://huggingface.co
curl -L -o models/diffusion/flux-2-klein-4b-Q3_K_M.gguf \
  "$B/unsloth/FLUX.2-klein-4B-GGUF/resolve/main/flux-2-klein-4b-Q3_K_M.gguf"
curl -L -o models/text_encoder/Qwen3-4B-Q4_K_M.gguf \
  "$B/unsloth/Qwen3-4B-GGUF/resolve/main/Qwen3-4B-Q4_K_M.gguf"
curl -L -o models/vae/flux2-vae.safetensors \
  "$B/unsloth/FLUX.2-VAE/resolve/main/split_files/vae/flux2-vae.safetensors"
```

Auf einem Mac übernimmt `scripts/generate-macos.sh` diese Downloads selbst (siehe unten);
manuell nötig ist das nur auf anderen Systemen.

Bei mehr RAM/VRAM lohnt eine höhere Quantisierung (`Q4_K_M`, `Q5_K_M`, `Q8_0`).

## Bauen

Voraussetzungen: Rust, ein C/C++-Compiler, CMake, **Ninja oder Make**, **libclang** (für bindgen).

Auf einem Mac erledigt das ein Script — es sucht die Werkzeuge, setzt `LIBCLANG_PATH`
und `CMAKE_GENERATOR` und fängt die bindgen-Falle (siehe Stolpersteine) selbst ab:

```sh
scripts/build.sh              # Release bauen, inkrementell
scripts/build.sh --tests      # vorher die Unit-Tests (Sekunden, ohne C++-Teil)
scripts/build.sh --clean      # target/ wegwerfen und alles neu bauen
```

Nach einer Änderung an `src/*.rs` dauert das Sekunden; der lange C++-Build läuft nur
beim ersten Mal, nach `--clean` oder wenn sich Abhängigkeiten ändern. `--clean` brauchst
du praktisch nur, wenn `target/` Artefakte einer anderen Plattform enthält.

Von Hand geht es genauso:

```sh
export LIBCLANG_PATH=/pfad/zu/lib   # nur nötig, wenn libclang nicht im Standardpfad liegt
export CMAKE_GENERATOR=Ninja        # nur nötig, wenn kein `make` vorhanden
cargo build --release
```

`generate-macos.sh` baut bei Bedarf selbst (`--rebuild` erzwingt es) — `build.sh` ist
für den Fall, dass du nur bauen und nichts erzeugen willst.

## Ein Script für macOS (Apple Silicon)

`scripts/generate-macos.sh` erledigt auf einem Mac alles in einem Lauf: fehlende Modelle
laden, Binaries bauen, Bild erzeugen, Motiv freistellen.

```sh
# Einfachster Fall — Ergebnis hat einen transparenten Hintergrund
scripts/generate-macos.sh "a red panda on a mossy rock, plain solid background"

# Mit Referenzbild und fester Zielgröße
scripts/generate-macos.sh -r figur.png -o token.png --out-w 138 --out-h 244 \
    "full body RPG character token of this character, plain solid background"

# Fünf Varianten in einem Rutsch — Hände und Waffen sind Würfelglück
scripts/generate-macos.sh --seeds 5 --seed -1 -o held.png "a dwarf cleric, plain solid background"

# Nur vorbereiten (Download + Build), ohne zu rechnen
scripts/generate-macos.sh --download-only
```

Mehrere Seeds (`--seeds N`) erzeugen `held-s1234.png`, `held-s1235.png`, … — der Seed
steht im Dateinamen, damit ein Treffer reproduzierbar bleibt. Mit `--seed -1` würfelt das
Script einmal einen Startwert und zählt von dort hoch. Ein fehlgeschlagener Lauf bricht
die Reihe nicht ab; am Ende stehen gelungene und gescheiterte Dateien getrennt, der
Exit-Code ist dann ungleich 0.

Der erste Lauf lädt bei `Q5_K_M` rund 5,9 GB (DiT 2,9 GB + Qwen3 2,7 GB + VAE 336 MB)
und baut stable-diffusion.cpp — beides passiert nur einmal, abgebrochene Downloads
setzt `curl -C -` fort.

Geschrieben werden zwei Dateien: `out.raw.png` (wie das Modell es gemalt hat) und
`out.png` (freigestellt, Hintergrund `alpha = 0`). `--keep-bg` lässt den zweiten Schritt
weg. Wichtigste Optionen, `--help` zeigt alle:

| Option | Default | Bedeutung |
|---|---|---|
| `-o, --out` | `out.png` | Zieldatei |
| `-s, --size` | `1024` | Kantenlänge; `-W`/`-H` einzeln, immer durch 16 teilbar |
| `--steps` / `--seed` | `4` / `42` | `--seed -1` würfelt pro Lauf neu |
| `--seeds N` | `1` | N Varianten mit fortlaufenden Seeds in einem Lauf |
| `-r, --ref DATEI` | – | Referenzbild, mehrfach angebbar |
| `--init DATEI` | – | img2img-Vorlage, Stärke über `--strength` (`0.75`) |
| `--out-w` / `--out-h` | – | freigestelltes Motiv in diese Fläche einpassen |
| `--keep-bg` | aus | nicht freistellen, Bild mit Hintergrund ausgeben |
| `--cfg` / `--guidance` | `1.0` / `3.5` | Qualitäts-Stellschrauben |
| `--flash` | aus | Flash-Attention an (auf den meisten Backends langsamer) |
| `--mmap` | aus | Gewichte per mmap — auf Metal führt das zu einem grauen Bild |
| `--ref-bg HEX` | `ffffff` | Hintergrund für transparente Referenzbilder |
| `--key` | aus | Freistellen über die Hintergrundfarbe statt über u2netp — behält den Sockel |
| `--key-color HEX` | – | Hintergrundfarbe vorgeben statt vom Rand lesen |
| `--key-innen` / `--key-aussen` | `70` / `95` | Toleranzband des Keyings |
| `--key-loch` / `--key-loch-min` | `30` / `500` | eingeschlossene Lücken (zwischen den Beinen, Arm/Rumpf) |
| `--quiet` / `--debug` | – | sd.cpp-Log aus bzw. mit DEBUG-Zeilen |
| `--cutoff N` | `12` | Alpha ≤ N gilt als Hintergrund |
| `--quant Q` | `Q5_K_M` | Quantisierung der beiden großen Modelle |
| `--threads N` | P-Kerne | Default: `hw.perflevel0.physicalcpu` |
| `--rebuild` / `--download-only` | – | neu bauen bzw. nur vorbereiten |

**Referenz oder img2img?** `-r/--ref` gibt das Bild als *Referenz* mit — "dieselbe Figur,
neue Szene", die Vorlage selbst wird nicht übermalt (FLUX.2-Edit-Modus). `--init`
verrauscht die Vorlage und malt sie neu; `--strength 0.3` bleibt nah am Original, `0.9`
kaum noch. Beides zusammen lehnt das Script ab, weil das Ergebnis nicht vorhersagbar wäre.

Metal aktiviert `diffusion-rs-sys` auf Apple-Targets von selbst — ein Feature-Flag
braucht es dafür nicht. Mit 32 GB Unified Memory ist `Q5_K_M` bei 1024² der vernünftige
Startpunkt; nicht der Speicher ist dort der Engpass, sondern die Wartezeit.

## Asset-Bibliothek: `scripts/token.sh`

Ein Aufsatz auf `generate-macos.sh` für den Fall, dass viele Figuren **derselben**
Kunstrichtung folgen sollen. Stil und Pose stehen als Variablen im Script, von außen
kommt nur der Charakter:

```sh
scripts/token.sh "female human warrior, weathered steel plate armor, longsword"
scripts/token.sh --seeds 5 --seed -1 "dwarf cleric, dark iron mail, warhammer"
scripts/token.sh -n "halfling rogue, leather armor, daggers"   # nur zeigen, was liefe
```

Der Dateiname entsteht aus dem Teil vor dem ersten Komma
(`female-human-warrior.png`), `-o` überschreibt ihn. Vorgaben sind 1024², 8 Steps und
`--out-w 138 --out-h 244`.

Eigene Optionen: `-o/--out`, `--pose`, `--style`, `--models`, `-n/--dry-run`, `-h`.
Freigestellt wird per Farb-Keying (`--key`), weil der Hintergrund im `STYLE` festgelegt
ist — damit bleibt der Sockel erhalten. `--no-key` schaltet auf u2netp zurück.

**Zum Licht:** `STYLE` verlangt bewusst `flat even ambient lighting` statt einer
gerichteten Lichtquelle. Gerichtetes Licht erzeugt zuverlässig auch einen Schlagschatten
auf dem Boden, und dagegen hilft keine Verneinung: bei `cfg_scale 1.0` wird der
Negativ-Prompt nie ausgewertet, und `no cast shadow` im Positiv-Prompt kodiert das Wort
*shadow*. Die Plastik der Figur kommt stattdessen aus dem Cel-Shading. Was trotzdem an
Schatten entsteht, schluckt das Farb-Keying beim Freistellen — `--key-innen 80` nimmt
mehr davon, falls ein Rest bleibt.

**Alles andere geht unverändert an `generate-macos.sh`** — und zwar hinter den eigenen
Vorgaben, sodass deine Angabe gewinnt:

```sh
scripts/token.sh -s 512 --steps 4 "gnome bard, lute"      # 512² statt 1024²
scripts/token.sh -r kriegerin.png "the same warrior, kneeling, shield raised"
```

Mehrere Charaktere sind eine Zeile Shell:

```sh
while IFS= read -r c; do scripts/token.sh "$c"; done < charaktere.txt
```

Dass `STYLE` eine Variable ist und kein Text zum Kopieren, ist der eigentliche Punkt:
das Modell hat kein Gedächtnis zwischen Läufen, ein einziges geändertes Wort verschiebt
den Stil der ganzen Reihe. Zusammen mit festem Seed und `-r` auf eine bereits
akzeptierte Figur ist das der einzige Weg zu einer einheitlichen Bibliothek.

## Ganze Reihe: `charaktere.txt` und `scripts/batch.sh`

`charaktere.txt` enthält 43 fertige Figuren — Völker von Zwerg bis Thri-Kreen, Klassen von
Barbar bis Artificer, jeweils mit erfundenem Namen, deutscher Beschreibung und englischem
Prompt:

```
slug | Name | Beschreibung (deutsch, für dich) | Prompt (englisch, fürs Modell)
```

Der slug wird zum Dateinamen, die Beschreibung liest nur der Mensch. `scripts/batch.sh`
arbeitet die Liste ab:

```sh
scripts/batch.sh                       # alles, was noch fehlt
scripts/batch.sh --dry-run             # nur zeigen, was zu tun wäre
scripts/batch.sh --from korth-froststurm
scripts/batch.sh --only nyx-aschenkind --seeds 3 --seed -1
```

| Option | Bedeutung |
|---|---|
| `--liste DATEI` | andere Charakterliste (Default: `<repo>/charaktere.txt`) |
| `--out-dir DIR` | Zielverzeichnis (Default: `tokens`) |
| `--from SLUG` | erst ab diesem Eintrag beginnen |
| `--only SLUG` | nur diesen einen Eintrag |
| `--force` | auch vorhandene Tokens neu erzeugen |
| `-n, --dry-run` | nur zeigen, was zu tun wäre |

Alles Weitere geht an `token.sh` und damit an `generate-macos.sh` durch.

**Fortsetzen ist der Normalfall.** Vorhandene Dateien im Zielverzeichnis gelten als
erledigt und werden übersprungen — ein erneuter Aufruf macht dort weiter, wo der letzte
aufgehört hat. Ein Abbruch mit Ctrl-C räumt die gerade entstehende Datei weg, damit sie
beim nächsten Mal nicht fälschlich als fertig gilt. Eine einzelne fehlgeschlagene Figur
bricht die Reihe nicht ab; sie steht am Ende in der Zusammenfassung, und der Exit-Code
ist dann ungleich 0.

Nach jeder Figur schätzt das Script aus dem bisherigen Mittel die Restzeit — bei über
40 Figuren ist der Unterschied zwischen "gleich fertig" und "über Nacht" die
Planungsgrundlage.

## Ausführen

```sh
SIZE=512 STEPS=4 SEED=42 OUT=out.png ./target/release/flux2-rs "dein prompt hier"
```

Alle Parameter sind Umgebungsvariablen — es gibt keinen Argument-Parser:

| Variable | Default | Bedeutung |
|---|---|---|
| `SIZE` | `512` | Kantenlänge in Pixeln (quadratisch) |
| `WIDTH` / `HEIGHT` | `SIZE` | Kanten einzeln; beide müssen durch 16 teilbar sein |
| `STEPS` | `4` | Diffusion-Steps |
| `SEED` | `42` | Fester Wert = reproduzierbar; **`-1` würfelt pro Lauf neu** |
| `THREADS` | `8` | Rechenthreads für ggml |
| `OUT` | `/workspace/out.png` | Zieldatei |
| `REF` | – | Referenzbilder für den Edit-Modus, kommasepariert |
| `INIT` | – | img2img-Vorlage |
| `STRENGTH` | `0.75` | nur mit `INIT`: 0.0 = Vorlage bleibt, 1.0 = alles neu |
| `REF_BG` | `ffffff` | Fläche, auf die transparente Referenzbilder gelegt werden |
| `MODELS_DIR` | `/workspace/models` | Wurzel für die Default-Modellpfade |
| `DIT` | Q3_K_M-Pfad | Diffusion-Transformer (GGUF) |
| `LLM` | Q4_K_M-Pfad | Text-Encoder (GGUF) |
| `VAE` | `$MODELS_DIR/vae/flux2-vae.safetensors` | VAE |
| `MMAP` | `1` | GGUF von der Platte mappen statt in den Heap kopieren. **Auf Metal/CUDA `0` setzen** — sonst graues Bild, siehe Stolpersteine |
| `FLASH_ATTENTION` | `1` | Flash-Attention in DiT und Text-Encoder — in sd.cpp nur für manche Modell/Backend-Paare implementiert |
| `VAE_TILING` | `1` | VAE gekachelt dekodieren — spart Speicher, kostet Zeit |
| `CFG` | `1.0` | `cfg_scale`; bei distillierten Modellen 1.0 |
| `GUIDANCE` | `3.5` | destillierte Guidance (eigener Eingang, nicht `cfg_scale`) |
| `LOG` | `1` | sd.cpp-Log: `0` still, `1` ab INFO, `2` mit DEBUG |

Referenzbilder **mit Alphakanal** werden vorher auf `REF_BG` gelegt: `diffusion-rs`
reicht sie mit `to_rgb8()` weiter und wirft Alpha dabei weg, transparente Flächen kämen
sonst als Schwarz im Modell an — ein bereits freigestelltes Token als Vorlage würde dem
Modell also "schwarzer Hintergrund" sagen.

Ein nicht existierendes `REF`-Bild ist ein **Fehler**: `diffusion-rs` überspringt fehlende
Referenzbilder stillschweigend, der Lauf liefe sonst minutenlang ohne Referenz weiter.

`klein` ist ein distilliertes Modell: `cfg_scale = 1.0` (kein Classifier-Free Guidance)
und wenige Steps (4). Die Nicht-distillierte Variante `FLUX.2-klein-base-4B` braucht
stattdessen `cfg_scale ≈ 4.0` und ~20 Steps.

## Transparenter Hintergrund: `matte`

Ein zweites Binary, unabhängig vom Generator und ohne C++-Abhängigkeit: es schickt das
Bild durch **u2netp** (Salient Object Detection, via `tract-onnx`) und zieht den
Hintergrund auf `alpha = 0`.

```sh
mkdir -p models/matting
curl -L -o models/matting/u2netp.onnx \
  https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2netp.onnx

# Nur den Hintergrund entfernen, Abmessungen bleiben erhalten
MODEL=models/matting/u2netp.onnx ./target/release/matte bild.png transparent.png

# Zusätzlich auf das Motiv zuschneiden und in 138x244 einpassen (z. B. VTT-Token)
MODEL=models/matting/u2netp.onnx OUT_W=138 OUT_H=244 \
  ./target/release/matte bild.png token.png
```

| Variable | Default | Bedeutung |
|---|---|---|
| `MODEL` | `$MODELS_DIR/matting/u2netp.onnx` | ONNX-Modell |
| `CUTOFF` | `12` | Alpha ≤ diesem Wert wird hart auf 0 gezogen |
| `OUT_W`/`OUT_H` | – | nur gemeinsam: zuschneiden und einpassen |
| `MASK_OUT` | – | Maske als Graustufenbild mitschreiben (Fehlersuche) |

### Zwei Wege zur Maske

`matte` kann die Maske auf zwei Arten bestimmen:

| | Saliency (Default) | Farb-Keying (`BG_KEY`) |
|---|---|---|
| Verfahren | u2netp (ONNX) sucht *ein* hervorstechendes Objekt | alles, was der Hintergrundfarbe ähnelt **und vom Bildrand aus zusammenhängt** |
| Sockel, Podeste, abgelegte Gegenstände | werden **weggeschnitten** | bleiben erhalten |
| Voraussetzung | beliebiger Hintergrund | einfarbiger Hintergrund |
| Modell nötig | ja (4,5 MB) | nein |
| Laufzeit 1024² | ~20 s | ~1 s |

```sh
# Hintergrundfarbe vom Bildrand lesen
BG_KEY=auto ./target/release/matte bild.png transparent.png

# Farbe vorgeben
BG_KEY=808080 ./target/release/matte bild.png transparent.png
```

| Variable | Default | Bedeutung |
|---|---|---|
| `BG_KEY` | – | `auto` oder `rrggbb`; ungesetzt = u2netp |
| `KEY_INNEN` | `70` | Farbabstand, bis zu dem ein Pixel reiner Hintergrund ist |
| `KEY_AUSSEN` | `95` | Farbabstand, ab dem ein Pixel Motiv ist |
| `KEY_LOCH` | `30` | eingeschlossene Flächen bis zu diesem Abstand ebenfalls entfernen; `0` = aus |
| `KEY_LOCH_MIN` | `500` | Mindestfläche einer solchen Lücke in Pixeln |

Zwischen beiden Werten wird weich übergeblendet — ein harter Schwellwert gäbe
Treppenkanten an jeder weich gemalten Silhouette.

Die Flutfüllung vom Bildrand ist der Kern: eine graue Rüstung mitten in der Figur hat
denselben Farbabstand wie der Hintergrund, ist aber vom Rand aus nicht erreichbar und
bleibt deshalb deckend.

**Eingeschlossene Lücken.** Die Flutfüllung startet am Bildrand — eine Lücke zwischen den
Beinen ist von dort aber nicht erreichbar, wenn der Sockel sie unten schließt. Ein zweiter
Durchgang sucht deshalb eingeschlossene Flächen. Er braucht **zwei** Bedingungen, weil
eine nicht reicht:

* nahezu exakte Hintergrundfarbe (`KEY_LOCH`, Default 30) — sonst bekämen dunkle Flächen Löcher, und
* eine Mindestgröße (`KEY_LOCH_MIN`, Default 500 px) — sonst trifft es die Glanzlichter auf
  Stahl, die im mittelgrauen Stil zufällig genau die Hintergrundfarbe haben.

Ohne die Größenschwelle löchert der Durchgang Bart, Rüstung und Sockel; ohne die
Farbschwelle frisst er ganze Bauteile. `KEY_LOCH=0` schaltet ihn ab.
Die Defaults sind an generierten Tokens auf mittelgrauem Grund gemessen. `KEY_INNEN`
höher zu setzen schluckt mehr vom weichen Schlagschatten; `KEY_AUSSEN` deutlich über 95
ist riskant — ab etwa 98 wird helle Haut durchlässig, und die Füllung schlägt Löcher ins
Gesicht.

Fehlt im Ergebnis etwas, das dabei sein sollte, zeigt `MASK_OUT` warum:

```sh
MODEL=models/matting/u2netp.onnx MASK_OUT=maske.png \
  ./target/release/matte bild.png transparent.png
```

Weiß heißt „gehört zum Motiv", Schwarz „Hintergrund". u2netp ist ein **Salient Object
Detector**: es sucht *ein* hervorstechendes Objekt. Sockel, Podeste und Schatten zählt
es zum Hintergrund, auch wenn sie im Prompt stehen — an `CUTOFF` liegt es dann nicht,
die Maske ist dort schon 0.

Die Ausgabe nennt den Anteil voll transparenter Pixel. Liegt er unter 1 %, warnt das
Programm — dann hat u2netp kein klares Motiv gefunden, und ein einfarbiger Hintergrund
im Ausgangsbild (`plain solid background` im Prompt) hilft mehr als jede Nachbearbeitung.

## Verifizierter Lauf (CPU-only, aarch64)

Getestet auf 8 Kernen ARM64 (Docker/Apple Silicon), **ohne GPU**, ~5 GB freiem RAM:

```sh
DIT=models/diffusion/flux-2-klein-4b-Q2_K.gguf \
LLM=models/text_encoder/Qwen3-4B-Q2_K.gguf \
SIZE=256 STEPS=4 OUT=out.png ./target/release/flux2-rs "a red panda on a mossy rock..."
```

Laufzeit: 4 Steps à ~86 s + 20 s VAE-Decode = **7:21 min**.

### RAM ist der Engpass, nicht die Auflösung

| DiT | Encoder | Gewichte | Auflösung | Ergebnis |
|---|---|---|---|---|
| Q3_K_M (2,1 GB) | Q4_K_M (2,4 GB) | 4,5 GB | 512² | OOM |
| Q3_K_M (2,1 GB) | Q4_K_M (2,4 GB) | 4,5 GB | 256² | OOM |
| Q2_K (1,8 GB) | Q2_K (1,7 GB) | 3,5 GB | 256² | ✅ |

Die Auflösung zu halbieren hat *nicht* gereicht — beide Modelle liegen gleichzeitig im
Speicher, und ihre Summe dominiert. Der Hebel ist die Quantisierung. Q2_K kostet sichtbar
Qualität; mit mehr RAM oder einer GPU sind Q5_K_M/Q8_0 und 1024² die richtige Wahl,
der Code bleibt identisch.

## Bekannte Stolpersteine

* **`enable_mmap` und GPU-Backends vertragen sich nicht.** Mit mmap legt sd.cpp die
  Gewichte in einen **CPU**-Buffer (`model_loader.cpp`:
  `ggml_backend_cpu_buffer_from_ptr`). Metal (und ebenso CUDA) findet für jeden Tensor
  dann `ggml_metal_buffer_get_id: error: tensor '…' buffer is nil`, rechnet mit Nullen
  und liefert ein **gleichmäßig graues Bild** — ohne Abbruch, ohne Fehlercode. Die
  Log-Zeilen stehen mitten im Ladevorgang und sind leicht zu übersehen.
  `MMAP=0` (Default im macOS-Script) ist die Lösung; im CPU-Container bleibt mmap an,
  dort ist es der Unterschied zwischen Lauf und OOM.
* **stable-diffusion.cpp schweigt ohne Log-Callback.** `log_printf` in
  `src/core/util.cpp` verwirft jede Zeile, solange kein `sd_log_callback` gesetzt ist,
  und `diffusion-rs` setzt keinen. Ohne diesen Callback sieht man weder das benutzte
  Backend noch Warnungen des VAE — ein halbstündiger Lauf endet kommentarlos.
  `src/main.rs` setzt ihn deshalb selbst; `LOG=0` schaltet ihn ab, `LOG=2` zeigt DEBUG.
* **Flash-Attention ist kein sicherer Schalter.** `diffusion-rs` hat ihn aus gutem Grund
  per Default aus: laut eigener Doku ist er "only supported for some models and some
  backends" und bremst auf den meisten Backends. Wird er für eine Kombination gesetzt,
  die ihn nicht unterstützt, kommt ein graues oder schwarzes Bild heraus statt einer
  Fehlermeldung. Im Container ist er trotzdem an (Speicher), das macOS-Script hat ihn aus.
* **bindgen + Clang ≥ 23**: bindgen 0.71 erzeugt für `_IO_FILE` einen Layout-Assert
  (`size_of == 216`), generiert den Typ aber opak (Größe 1) → `error[E0080]: attempt to
  compute 1_usize - 216_usize`. Abhilfe: `diffusion-rs-sys` vendoren und in dessen
  `build.rs` `.layout_tests(false)` an den `bindgen::Builder` hängen. Alternativ ein
  älteres libclang verwenden.
* **CMake findet kein Build-Tool**: die `cmake`-Crate wählt standardmäßig
  "Unix Makefiles". Ohne `make` im PATH `CMAKE_GENERATOR=Ninja` setzen.
* **Wenig RAM**: `enable_mmap(true)` (in `src/main.rs` gesetzt) mappt die GGUF-Gewichte
  von der Platte, statt sie in den Heap zu kopieren. Auf CPU-only-Maschinen mit ~5 GB
  freiem RAM ist das der Unterschied zwischen Lauf und OOM.
