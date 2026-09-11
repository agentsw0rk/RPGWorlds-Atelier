#!/usr/bin/env bash
#
# Bildgenerierung auf einem macOS-Host (Apple Silicon).
#
# Drei Schritte in einem Lauf:
#   1. Modelle holen (nur was fehlt), Werkzeuge prüfen, Binaries bauen
#   2. Bild erzeugen — optional mit Referenzbild (FLUX.2-Edit-Modus)
#   3. Motiv freistellen: Hintergrund wird alpha = 0
#
# Metal aktiviert diffusion-rs-sys auf Apple-Targets von selbst
# (build.rs: `if target.contains("apple") ... use_metal = true`) — ein
# Feature-Flag ist dafür nicht nötig.
#
# Bewusst nur POSIX-nahe Bash-Konstrukte: macOS liefert bis heute Bash 3.2 aus,
# dort sind leere Arrays zusammen mit `set -u` ein Fehler.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$repo_root/scripts/umgebung.sh"

# --- Vorgaben, abgestimmt auf M1 mit 32 GB Unified Memory --------------------
# Bei so viel Speicher ist nicht die Quantisierung der Engpass, sondern die
# Wartezeit — Q5_K_M ist der Punkt, ab dem mehr Bits kaum noch sichtbar sind.
quant="${QUANT:-Q5_K_M}"
models_dir="${MODELS_DIR:-$repo_root/models}"
prompt=""
out="out.png"
size=1024
width=""
height=""
steps=4
seed=42
seeds=1
threads=""
cutoff=12
out_w=""
out_h=""
strength=0.75
init=""
ref_list=""
do_matte=1
force_build=0
download_only=0
vae_tiling=0          # 32 GB brauchen kein gekacheltes VAE-Decoding
# Flash-Attention ist in sd.cpp nur für manche Modell-Backend-Kombinationen
# implementiert und bremst laut Doku auf den meisten Backends. Es war hier nur
# an, um im Container Speicher zu sparen — auf einem 32-GB-Mac ist das unnötig
# und ein möglicher Grund für kaputte (graue) Bilder.
flash_attention=0
preset=klein-4b       # Modellsatz, siehe unten
wtype=""             # Gewichte beim Laden umwandeln, z. B. q8_0
cfg_scale=1.0        # wird vom Modellsatz überschrieben, wenn nicht gesetzt
cfg_gesetzt=""
steps_gesetzt=""
guidance=3.5          # destillierte Guidance, sd.cpps Default
log_level=1           # 0 = still, 1 = sd.cpp-Log, 2 = zusätzlich DEBUG
# mmap bindet die Gewichte an einen CPU-Buffer
# (model_loader.cpp: ggml_backend_cpu_buffer_from_ptr). Metal findet dann für
# jeden Tensor "buffer is nil" und rechnet mit Nullen — Ergebnis ist ein graues
# Bild. Auf GPU-Backends muss mmap deshalb aus sein; im CPU-Container ist es an.
mmap=0
ref_bg=ffffff        # Hintergrund für transparente Referenzbilder
# Farb-Keying statt u2netp: leer = Saliency, "auto" = Hintergrundfarbe vom
# Bildrand lesen, "rrggbb" = feste Farbe.
bg_key=""
key_innen=""
key_aussen=""
key_loch=""
key_loch_min=""
despill=1            # Farbsaum aus den Randpixeln herausrechnen

usage() {
    cat <<'USAGE'
Aufruf: scripts/generate-macos.sh [Optionen] "prompt"

Bild:
  -p, --prompt TEXT     Prompt (alternativ als letztes Argument)
  -o, --out DATEI       Zieldatei (Default: out.png)
  -s, --size N          Kantenlänge quadratisch (Default: 1024, durch 16 teilbar)
  -W, --width N         Breite einzeln
  -H, --height N        Höhe einzeln
      --steps N         Diffusion-Steps (Default: 4 — klein ist distilliert)
      --seed N          Seed (Default: 42; -1 würfelt pro Lauf neu)
      --seeds N         N Varianten mit fortlaufenden Seeds erzeugen (Default: 1).
                        Jede Datei bekommt ihren Seed in den Namen, ein
                        fehlgeschlagener Lauf bricht die Reihe nicht ab.

Referenzbild:
  -r, --ref DATEI.png   Referenzbild für In-Context-Konditionierung (FLUX.2-Edit).
                        Mehrfach angebbar. Die Vorlage wird nicht übermalt, sondern
                        als Referenz mitgegeben ("dieselbe Figur, neue Szene").
      --init DATEI.png  Klassisches img2img: Vorlage wird verrauscht und neu
                        gezeichnet.
      --ref-bg HEX      Transparente Referenzbilder auf diese Farbe legen
                        (Default: ffffff). diffusion-rs wirft Alpha sonst weg und
                        macht daraus Schwarz.
      --strength F      Nur für --init: 0.0 = Vorlage bleibt, 1.0 = alles neu
                        (Default: 0.75)
                        --ref und --init schließen sich gegenseitig aus.

Transparenz (Freistellen mit u2netp):
      --keep-bg         Nicht freistellen, Bild mit Hintergrund ausgeben
      --key             Freistellen über die Hintergrundfarbe statt über u2netp.
                        Sockel, Podeste und abgelegte Gegenstände bleiben erhalten —
                        u2netp erkennt nur *ein* hervorstechendes Objekt und lässt
                        sie weg. Setzt einen einfarbigen Hintergrund voraus.
      --key-color HEX   Hintergrundfarbe vorgeben statt vom Bildrand lesen
      --key-innen N     Farbabstand, bis zu dem ein Pixel reiner Hintergrund ist
                        (Default: 70). Höher = weniger Schlagschatten bleibt stehen.
      --key-aussen N    Farbabstand, ab dem ein Pixel Motiv ist (Default: 95).
                        Deutlich höher zu setzen ist riskant: dann läuft die
                        Flutfüllung durch helle Hauttöne und macht Löcher.
      --key-loch N      Eingeschlossene Flächen (Lücke zwischen den Beinen, Zwickel
                        zwischen Arm und Rumpf) bis zu diesem Farbabstand ebenfalls
                        entfernen (Default: 12). 0 schaltet es ab. Höher zu setzen ist
                        riskant: auf den Sockel gemalte Schatten liegen nah an der
                        Hintergrundfarbe und werden dann mit weggeschnitten.
      --key-loch-min N  Mindestfläche einer solchen Lücke in Pixeln (Default: 500).
                        Verhindert, dass Glanzlichter auf Stahl — die zufällig
                        Hintergrundfarbe haben — die Rüstung durchlöchern.
      --no-despill      Den Farbsaum in den Randpixeln stehen lassen. Weiche Kanten
                        sind Mischungen aus Motiv und Hintergrund; Despill rechnet
                        den Hintergrundanteil heraus. Bei grauem Grund kaum sichtbar,
                        bei gesättigtem Keying-Hintergrund entscheidend.
      --cutoff N        Alpha <= N wird hart auf 0 gezogen (Default: 12)
      --out-w N         Nur zusammen mit --out-h: auf das Motiv zuschneiden und
      --out-h N         in eine Fläche dieser Größe einpassen (z. B. VTT-Token).
                        Ohne beides behält das Bild seine Abmessungen, es wird
                        nur der Hintergrund transparent.

Qualität und Diagnose:
      --cfg F           cfg_scale (Default: 1.0 — klein ist distilliert)
      --guidance F      Destillierte Guidance (Default: 3.5)
      --flash           Flash-Attention an (spart Speicher, in sd.cpp nur für
                        manche Modell/Backend-Paare implementiert — kann graue
                        Bilder erzeugen; deshalb standardmäßig aus)
      --mmap            Gewichte per mmap laden. Auf Metal/CUDA führt das zu
                        "buffer is nil" und einem grauen Bild — nur für CPU-Läufe.
      --quiet           Log von stable-diffusion.cpp unterdrücken
      --debug           zusätzlich DEBUG-Zeilen zeigen

Modelle und Build:
      --preset NAME     Modellsatz (Default: klein-4b)
                        klein-4b      distilliert, GGUF, cfg 1.0 und 4 Steps
                        klein-base-9b nicht distilliert, fp8-Safetensors, 9,5 GB,
                                      cfg 4.0 und 20 Steps. Repo ist gated:
                                      Lizenz bestätigen und HF_TOKEN setzen.
      --wtype TYP       Gewichte beim Laden umwandeln: f32, f16, q8_0, q6_k,
                        q5_k, q4_k, q3_k. sd.cpp rechnet fp8 beim Einlesen auf
                        f16 hoch — aus 9,5 GB werden sonst rund 19 GB im
                        Speicher. Bei klein-base-9b ist q8_0 voreingestellt.
      --quant Q         GGUF-Quantisierung für klein-4b, z. B. Q4_K_M, Q8_0 (Default: Q5_K_M)
      --models DIR      Modellverzeichnis (Default: <repo>/models)
      --threads N       Threads (Default: Performance-Kerne des Rechners)
      --vae-tiling      VAE gekachelt dekodieren (spart Speicher, kostet Zeit)
      --no-flash        Flash-Attention aus (Default)
      --rebuild         Wird nicht mehr gebraucht: es wird ohnehin vor jedem Lauf
                        gebaut. Bleibt erhalten, damit alte Aufrufe funktionieren.
      --download-only   Nur Modelle laden und bauen, kein Bild erzeugen
  -h, --help            Diese Hilfe

Beispiele:
  scripts/generate-macos.sh "a red panda on a mossy rock, plain solid background"

  scripts/generate-macos.sh -r figur.png -o token.png --out-w 138 --out-h 244 \
      "full body RPG character token of this character, plain solid background"
USAGE
}

require_number() {
    case "$2" in
        ''|*[!0-9-]*) die "$1 braucht eine Zahl, bekam: '$2'" ;;
    esac
}

# Eigener Prüfer für --strength: 0.75 würde an require_number scheitern.
require_float() {
    case "$2" in
        ''|*[!0-9.]*|*.*.*) die "$1 braucht eine Kommazahl, bekam: '$2'" ;;
    esac
}

while [ $# -gt 0 ]; do
    case "$1" in
        -p|--prompt)     prompt="$2"; shift 2 ;;
        -o|--out)        out="$2"; shift 2 ;;
        -s|--size)       require_number "--size" "$2";   size="$2"; shift 2 ;;
        -W|--width)      require_number "--width" "$2";  width="$2"; shift 2 ;;
        -H|--height)     require_number "--height" "$2"; height="$2"; shift 2 ;;
        --steps)         require_number "--steps" "$2";  steps="$2"; steps_gesetzt=1; shift 2 ;;
        --seed)          require_number "--seed" "$2";   seed="$2"; shift 2 ;;
        --seeds)         require_number "--seeds" "$2";  seeds="$2"; shift 2 ;;
        --out-w)         require_number "--out-w" "$2";  out_w="$2"; shift 2 ;;
        --out-h)         require_number "--out-h" "$2";  out_h="$2"; shift 2 ;;
        --cutoff)        require_number "--cutoff" "$2"; cutoff="$2"; shift 2 ;;
        --threads)       require_number "--threads" "$2"; threads="$2"; shift 2 ;;
        -r|--ref)
            [ -f "$2" ] || die "Referenzbild nicht gefunden: $2"
            # REF wird kommasepariert übergeben — ein Komma im Pfad bräche das.
            case "$2" in *,*) die "Referenzpfad darf kein Komma enthalten: $2" ;; esac
            if [ -z "$ref_list" ]; then ref_list="$2"; else ref_list="$ref_list,$2"; fi
            shift 2 ;;
        --init)
            [ -f "$2" ] || die "img2img-Vorlage nicht gefunden: $2"
            init="$2"; shift 2 ;;
        --strength)      require_float "--strength" "$2"; strength="$2"; shift 2 ;;
        --keep-bg)       do_matte=0; shift ;;
        --quant)         quant="$2"; shift 2 ;;
        --models)        models_dir="$2"; shift 2 ;;
        --vae-tiling)    vae_tiling=1; shift ;;
        --flash)         flash_attention=1; shift ;;
        --no-flash)      flash_attention=0; shift ;;
        --mmap)          mmap=1; shift ;;
        --ref-bg)        ref_bg="$2"; shift 2 ;;
        --key)           bg_key="auto"; shift ;;
        --no-key)        bg_key=""; shift ;;
        --key-color)     bg_key="$2"; shift 2 ;;
        --key-innen)     require_float "--key-innen" "$2";  key_innen="$2"; shift 2 ;;
        --key-aussen)    require_float "--key-aussen" "$2"; key_aussen="$2"; shift 2 ;;
        --key-loch)      require_float "--key-loch" "$2";   key_loch="$2"; shift 2 ;;
        --key-loch-min)  require_number "--key-loch-min" "$2"; key_loch_min="$2"; shift 2 ;;
        --no-despill)    despill=0; shift ;;
        --cfg)           require_float "--cfg" "$2"; cfg_scale="$2"; cfg_gesetzt=1; shift 2 ;;
        --preset)        preset="$2"; shift 2 ;;
        --wtype)         wtype="$2"; shift 2 ;;
        --guidance)      require_float "--guidance" "$2"; guidance="$2"; shift 2 ;;
        --quiet)         log_level=0; shift ;;
        --debug)         log_level=2; shift ;;
        --rebuild)       force_build=1; shift ;;
        --download-only) download_only=1; shift ;;
        -h|--help)       usage; exit 0 ;;
        -*)              echo "Unbekannte Option: $1" >&2; usage >&2; exit 2 ;;
        *)               prompt="$1"; shift ;;
    esac
done

width="${width:-$size}"
height="${height:-$size}"

[ "$download_only" -eq 1 ] || [ -n "$prompt" ] || { usage >&2; die "Kein Prompt angegeben."; }
[ $(( width % 16 ))  -eq 0 ] || die "WIDTH=$width ist nicht durch 16 teilbar — das Modell verlangt das."
[ $(( height % 16 )) -eq 0 ] || die "HEIGHT=$height ist nicht durch 16 teilbar — das Modell verlangt das."
if [ -n "$out_w$out_h" ] && { [ -z "$out_w" ] || [ -z "$out_h" ]; }; then
    die "--out-w und --out-h nur gemeinsam angeben."
fi
if [ -n "$ref_list" ] && [ -n "$init" ]; then
    # Beides zusammen wäre ein Lauf, dessen Ergebnis niemand vorhersagt:
    # --init legt die Leinwand fest, --ref beschreibt den Inhalt.
    die "--ref und --init schließen sich aus — entweder Referenzbild oder img2img-Vorlage."
fi
if [ "$seeds" -lt 1 ]; then
    die "--seeds braucht mindestens 1, bekam: '$seeds'"
fi
if [ "$do_matte" -eq 0 ] && [ -n "$out_w$out_h" ]; then
    die "--out-w/--out-h wirken nur beim Freistellen, nicht zusammen mit --keep-bg."
fi

# Lieber jetzt anlegen als nach einer halben Stunde Rechnen am Schreiben scheitern.
out_dir="$(dirname "$out")"
[ -d "$out_dir" ] || mkdir -p "$out_dir"
[ -w "$out_dir" ] || die "Zielverzeichnis $out_dir ist nicht beschreibbar."

# --- Werkzeuge ---------------------------------------------------------------
umgebung_pruefen
threads="${threads:-$THREADS}"

# --- Modelle -----------------------------------------------------------------
say "Modelle in $models_dir"

fetch() {
    url="$1"; dest="$2"; label="$3"
    if [ -s "$dest" ]; then
        echo "vorhanden: $label"
        return 0
    fi
    echo "lade $label ..."
    mkdir -p "$(dirname "$dest")"
    # Manche Repos sind gated (FLUX.2-klein-base etwa): dort braucht es einen
    # Zugriffstoken, sonst kommt eine HTML-Fehlerseite statt der Gewichte.
    if [ -n "${HF_TOKEN:-}" ]; then
        set -- -H "Authorization: Bearer $HF_TOKEN"
    else
        set --
    fi
    # -C - setzt abgebrochene Downloads fort, .part verhindert halbe Dateien.
    if ! curl -fL --retry 3 --retry-delay 2 -C - ${1+"$@"} -o "$dest.part" "$url"; then
        rm -f "$dest.part"
        echo >&2
        echo "Download fehlgeschlagen: $label" >&2
        case "$url" in *black-forest-labs*)
            echo "Dieses Repo ist gated. Lizenz auf huggingface.co bestätigen," >&2
            echo "dann ein Token anlegen und HF_TOKEN=hf_... setzen." >&2 ;;
        esac
        return 1
    fi
    mv "$dest.part" "$dest"
}

hf="https://huggingface.co"
# --- Modellsatz --------------------------------------------------------------
# klein-4b:      distilliert, GGUF, cfg 1.0 und wenige Steps. Der Normalfall.
# klein-base-9b: nicht distilliert, eine fp8-Safetensors-Datei von 9,5 GB.
#                Braucht cfg ~4.0 und ~20 Steps, sonst kommt Matsch heraus.
#                sd.cpp rechnet fp8 beim Laden auf f16 hoch, deshalb ist hier
#                WTYPE=q8_0 voreingestellt — ohne das sind es rund 19 GB.
case "$preset" in
    klein-4b)
        dit="$models_dir/diffusion/flux-2-klein-4b-$quant.gguf"
        dit_url="$hf/unsloth/FLUX.2-klein-4B-GGUF/resolve/main/flux-2-klein-4b-$quant.gguf"
        dit_label="Diffusion-Transformer klein 4B ($quant)"
        [ -n "$steps_gesetzt" ] || steps=4
        [ -n "$cfg_gesetzt" ]   || cfg_scale=1.0
        ;;
    klein-base-9b)
        dit="$models_dir/diffusion/flux-2-klein-base-9b-fp8.safetensors"
        dit_url="$hf/black-forest-labs/FLUX.2-klein-base-9b-fp8/resolve/main/flux-2-klein-base-9b-fp8.safetensors"
        dit_label="Diffusion-Transformer klein-base 9B (fp8, 9,5 GB)"
        [ -n "$steps_gesetzt" ] || steps=20
        [ -n "$cfg_gesetzt" ]   || cfg_scale=4.0
        [ -n "$wtype" ]         || wtype=q8_0
        ;;
    *)
        die "Unbekannter Modellsatz: '$preset'. Gültig: klein-4b, klein-base-9b"
        ;;
esac

llm="$models_dir/text_encoder/Qwen3-4B-$quant.gguf"
vae="$models_dir/vae/flux2-vae.safetensors"
u2netp="$models_dir/matting/u2netp.onnx"

fetch "$dit_url" "$dit" "$dit_label"
fetch "$hf/unsloth/Qwen3-4B-GGUF/resolve/main/Qwen3-4B-$quant.gguf" \
      "$llm" "Text-Encoder Qwen3-4B ($quant)"
fetch "$hf/unsloth/FLUX.2-VAE/resolve/main/split_files/vae/flux2-vae.safetensors" \
      "$vae" "VAE"
# Beim Farb-Keying wird u2netp gar nicht geladen — dann ist der Download unnötig.
if [ "$do_matte" -eq 1 ] && [ -z "$bg_key" ]; then
    fetch "https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2netp.onnx" \
          "$u2netp" "u2netp (Freistellen)"
fi

# --- Build -------------------------------------------------------------------
bin_gen="$repo_root/target/release/flux2-rs"
bin_matte="$repo_root/target/release/matte"

# Immer bauen lassen — nicht nur, wenn ein Binary fehlt.
#
# cargo entscheidet selbst, ob etwas zu tun ist: bei unverändertem Stand kostet
# das Sekundenbruchteile. Die frühere Prüfung "Datei existiert" hat dagegen ein
# veraltetes Binary stillschweigend weiterbenutzt — nach einer Änderung an
# src/*.rs lief die alte Fassung weiter, und das fällt erst am Ergebnis auf.
bauen "$repo_root"

if [ "$download_only" -eq 1 ]; then
    say "Fertig (--download-only): Modelle liegen bereit, Binaries sind gebaut."
    exit 0
fi

# --- Bild erzeugen -----------------------------------------------------------

# Erzeugt genau ein Bild und stellt es frei. $1 = Seed, $2 = Zieldatei.
# Liefert ungleich 0 zurück, wenn ein Schritt scheitert — die Schleife über
# mehrere Seeds läuft dann weiter, statt einen ganzen Abend wegzuwerfen.
erzeuge() {
    lauf_seed="$1"
    ziel="$2"
    # Das Rohbild bleibt neben dem freigestellten liegen — praktisch zum Vergleichen.
    if [ "$do_matte" -eq 1 ]; then
        roh="${ziel%.*}.raw.png"
    else
        roh="$ziel"
    fi

    started=$SECONDS
    MODELS_DIR="$models_dir" \
    DIT="$dit" LLM="$llm" VAE="$vae" \
    WIDTH="$width" HEIGHT="$height" STEPS="$steps" SEED="$lauf_seed" THREADS="$threads" \
    REF="$ref_list" INIT="$init" STRENGTH="$strength" \
    VAE_TILING="$vae_tiling" FLASH_ATTENTION="$flash_attention" \
    CFG="$cfg_scale" GUIDANCE="$guidance" LOG="$log_level" MMAP="$mmap" WTYPE="$wtype" \
    REF_BG="$ref_bg" \
    OUT="$roh" \
        "$bin_gen" "$prompt" || return 1
    elapsed=$(( SECONDS - started ))
    printf 'Dauer der Generierung: %d:%02d min\n' $(( elapsed / 60 )) $(( elapsed % 60 ))

    if [ "$do_matte" -eq 1 ]; then
        say "Freistellen (Hintergrund wird alpha = 0)"
        MODEL="$u2netp" CUTOFF="$cutoff" \
        BG_KEY="$bg_key" KEY_INNEN="$key_innen" KEY_AUSSEN="$key_aussen" \
        KEY_LOCH="$key_loch" KEY_LOCH_MIN="$key_loch_min" DESPILL="$despill" \
        OUT_W="${out_w:-}" OUT_H="${out_h:-}" \
            "$bin_matte" "$roh" "$ziel" || return 1
        echo
        echo "Rohbild:     $roh"
        echo "Transparent: $ziel"
    else
        echo
        echo "Bild: $ziel"
    fi
    return 0
}

# -1 heißt "würfle den Seed". Bei mehreren Läufen würfeln wir einmal selbst und
# zählen von dort hoch: sonst stünde in keinem Dateinamen ein Seed, und ein
# Treffer wäre nicht reproduzierbar.
if [ "$seeds" -gt 1 ] && [ "$seed" -lt 0 ]; then
    seed=$(( $(od -An -N4 -tu4 < /dev/urandom | tr -d ' ') % 1000000000 ))
    echo "Startseed gewürfelt: $seed"
fi

gelungen=""
fehlgeschlagen=""
i=0
while [ "$i" -lt "$seeds" ]; do
    lauf_seed=$(( seed < 0 ? -1 : seed + i ))

    # Bei mehreren Seeds trägt jede Datei ihren Seed im Namen — sonst
    # überschreiben sich die Läufe gegenseitig.
    if [ "$seeds" -gt 1 ]; then
        case "$out" in
            *.*) ziel="${out%.*}-s${lauf_seed}.${out##*.}" ;;
            *)   ziel="${out}-s${lauf_seed}" ;;
        esac
        say "Generierung $(( i + 1 ))/$seeds — Seed $lauf_seed"
    else
        ziel="$out"
        say "Generierung"
    fi

    if erzeuge "$lauf_seed" "$ziel"; then
        gelungen="$gelungen$ziel
"
    else
        fehlgeschlagen="$fehlgeschlagen$ziel (Seed $lauf_seed)
"
    fi
    i=$(( i + 1 ))
done

if [ "$seeds" -gt 1 ]; then
    say "Fertig — $seeds Varianten"
    printf '%s' "$gelungen"
fi
if [ "$do_matte" -eq 1 ]; then
    echo "Tipp: 'plain solid background' im Prompt erleichtert dem Matting die Arbeit."
fi
if [ -n "$fehlgeschlagen" ]; then
    echo >&2
    echo "Fehlgeschlagen:" >&2
    printf '%s' "$fehlgeschlagen" >&2
    exit 1
fi
