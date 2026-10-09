#!/usr/bin/env bash
#
# Startet die HTTP-API (flux2-api) auf einem macOS-Host.
#
#   ./start.sh                       # bauen (inkrementell), dann starten auf 127.0.0.1:8080
#   ./start.sh --port 9000
#   ./start.sh --no-build            # nie bauen, vorhandenes Binary starten
#   ./start.sh --build               # immer bauen, auch wenn nichts geändert scheint
#   ./start.sh --bind 0.0.0.0:8080 --token geheim    # im LAN erreichbar
#   ./start.sh --demo                # Weboberfläche ausprobieren: Platzhalterbilder, keine Modelle
#   ./start.sh -n                    # nur zeigen, mit welchen Einstellungen gestartet würde
#
# HF_TOKEN (für gated Repos wie klein-base-9b): aus der Umgebung, sonst aus einer Datei.
# Gesucht wird hg-token.env oder hf-token.env im Projektstamm; --hf-token-file nennt eine
# andere. Die Datei darf nur das Token enthalten oder `HF_TOKEN=…` (auch mit `export`).
# Sie wird gelesen, nicht ausgeführt, und das Token nie ausgegeben. `*token*.env` steht
# in .gitignore.
#
# Gebaut wird nur, wenn nötig: wenn das Binary fehlt oder src/, web/, Cargo.toml oder
# Cargo.lock neuer sind als es. Ansonsten startet die API sofort.
#
# Danach ist die Weboberfläche unter http://127.0.0.1:8080/ erreichbar (dieselbe Adresse
# wie die API). Das Script setzt nur Umgebungsvariablen und startet das Binary; die
# Logik steckt vollständig in Rust. Eine bereits gesetzte Variable gewinnt immer
# gegen den Default hier — also geht auch `MODELS_DIR=/ssd/modelle ./start.sh`.
# Alle Variablen und Routen: flux2-rs/README.md, Abschnitt "HTTP-API: flux2-api".
#
# Bewusst nur POSIX-nahe Bash-Konstrukte: macOS liefert bis heute Bash 3.2 aus.
set -euo pipefail

# Dieses Script liegt im Projektstamm; Crate, Listen und Skripte liegen in flux2-rs/.
projekt_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$projekt_root/flux2-rs"
[ -d "$repo_root" ] || { echo "flux2-rs/ nicht gefunden neben $0" >&2; exit 2; }
. "$repo_root/scripts/umgebung.sh"

build=auto
dry_run=0
demo=0
port=""
bind_adresse=""
token=""
daten=""
modelle=""
hf_datei=""

usage() {
    cat <<'USAGE'
Aufruf: ./start.sh [Optionen]

      --port N         Port auf 127.0.0.1 (Default: 8080)
      --bind ADDR      Vollständige Adresse, z. B. 0.0.0.0:8080. Ist sie nicht lokal,
                       ist --token Pflicht — sonst könnte jeder im Netz rechnen lassen.
      --token TEXT     Erwartetes "Authorization: Bearer TEXT" (alle Routen außer /health)
      --data DIR       Jobs, Uploads, Batch-Ausgabeordner (Default: <repo>/api-daten)
      --models DIR     Modellverzeichnis (Default: <projekt>/models bzw. <repo>/models)
      --demo           Demo-Modus: Platzhalterbilder statt Modell, kein Download. Zum Ausprobieren
                       der Oberfläche; Daten landen getrennt in <repo>/api-daten-demo.
      --build          Immer bauen (sonst nur, wenn das Binary fehlt oder veraltet ist)
      --hf-token-file F  Hugging-Face-Token aus dieser Datei lesen (Default: hg-token.env oder
                       hf-token.env im Projektstamm, falls HF_TOKEN nicht gesetzt ist)
      --no-build       Nie bauen, das vorhandene Binary starten
  -n, --dry-run        Nur zeigen, mit welchen Einstellungen gestartet würde
  -h, --help           Diese Hilfe

Weitere Einstellungen kommen aus der Umgebung und bleiben erhalten, wenn gesetzt:
HF_TOKEN (gated Repos), THREADS, MMAP, FLASH_ATTENTION, VAE_TILING, LOG, REF_BG,
METRICS_MS (Abstand der Speicher-Messung in ms, Standard 1000, 0 = aus; Ausgabe in
jobs/<id>/metrics.jsonl).
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --port)       port="$2"; shift 2 ;;
        --bind)       bind_adresse="$2"; shift 2 ;;
        --token)      token="$2"; shift 2 ;;
        --data)       daten="$2"; shift 2 ;;
        --models)     modelle="$2"; shift 2 ;;
        --hf-token-file) hf_datei="$2"; shift 2 ;;
        --demo)       demo=1; shift ;;
        --build)      build=1; shift ;;
        --no-build)   build=0; shift ;;
        -n|--dry-run) dry_run=1; shift ;;
        -h|--help)    usage; exit 0 ;;
        *)            echo "Unbekannte Option: $1" >&2; usage >&2; exit 2 ;;
    esac
done

if [ -n "$port" ] && [ -n "$bind_adresse" ]; then
    die "--port und --bind schließen sich aus — --bind enthält den Port schon."
fi
case "$port" in
    ''|*[!0-9]*) [ -z "$port" ] || die "--port erwartet eine Zahl, bekam: '$port'" ;;
esac

# --- Einstellungen: Option > bereits gesetzte Variable > Default ---------------
[ -z "$bind_adresse" ] || export FLUX2_BIND="$bind_adresse"
[ -z "$port" ]         || export FLUX2_BIND="127.0.0.1:$port"
[ -z "$token" ]        || export FLUX2_API_TOKEN="$token"
[ -z "$daten" ]        || export FLUX2_DATA="$daten"
[ -z "$modelle" ]      || export MODELS_DIR="$modelle"

export FLUX2_BIND="${FLUX2_BIND:-127.0.0.1:8080}"
export FLUX2_PROJECT="${FLUX2_PROJECT:-$repo_root}"
# Demo-Bilder bekommen ein eigenes Verzeichnis: in fullbody/ & Co. würden sie später
# als "schon vorhanden" gelten und echte Bilder beim Batch überspringen lassen.
if [ "$demo" -eq 1 ]; then
    export FLUX2_DEMO=1
    export FLUX2_DATA="${FLUX2_DATA:-$repo_root/api-daten-demo}"
fi
export FLUX2_DATA="${FLUX2_DATA:-$repo_root/api-daten}"
export MODELS_DIR="${MODELS_DIR:-$(standard_modelle "$repo_root")}"

# Metal/CUDA: mmap bindet die Gewichte an einen CPU-Buffer, die GPU rechnet dann
# mit Nullen und liefert ein einfarbig graues Bild — ohne Fehlermeldung. Deshalb
# hier ausdrücklich aus; auf einem 32-GB-Mac sind auch Flash-Attention und
# gekacheltes VAE-Decoding unnötig.
export MMAP="${MMAP:-0}"
export FLASH_ATTENTION="${FLASH_ATTENTION:-0}"
export VAE_TILING="${VAE_TILING:-0}"

case "$FLUX2_BIND" in
    127.*|localhost:*|\[::1\]:*) ;;
    *) [ -n "${FLUX2_API_TOKEN:-}" ] || die "$FLUX2_BIND ist von außen erreichbar — dafür --token (oder FLUX2_API_TOKEN) setzen." ;;
esac

# Liest ein Token aus einer Datei: erste Zeile, die weder leer noch Kommentar ist. Erlaubt
# sind "TOKEN", "HF_TOKEN=TOKEN" und "export HF_TOKEN=TOKEN", mit oder ohne Anführungszeichen.
# Die Datei wird nicht ausgeführt (kein source) — sie könnte beliebigen Code enthalten.
hf_token_lesen() {
    zeile="$(sed -e 's/\r$//' -e '/^[[:space:]]*#/d' -e '/^[[:space:]]*$/d' "$1" | head -n 1)"
    zeile="${zeile#export }"
    case "$zeile" in HF_TOKEN=*|HUGGING_FACE_HUB_TOKEN=*) zeile="${zeile#*=}" ;; esac
    zeile="$(printf '%s' "$zeile" | sed -e "s/^[[:space:]\"']*//" -e "s/[[:space:]\"']*\$//")"
    printf '%s' "$zeile"
}

hf_quelle=""
if [ -n "${HF_TOKEN:-}" ]; then
    hf_quelle="Umgebung"
else
    kandidaten="$hf_datei $projekt_root/hg-token.env $projekt_root/hf-token.env"
    for datei in $kandidaten; do
        [ -n "$datei" ] && [ -f "$datei" ] || continue
        token_aus_datei="$(hf_token_lesen "$datei")"
        if [ -n "$token_aus_datei" ]; then
            export HF_TOKEN="$token_aus_datei"
            hf_quelle="$(basename "$datei")"
            break
        fi
        echo "Hinweis: $datei enthält kein Token." >&2
    done
    unset token_aus_datei
fi
[ -z "$hf_datei" ] || [ -f "$hf_datei" ] || die "--hf-token-file: Datei nicht gefunden: $hf_datei"

bin="$repo_root/target/release/flux2-api"

# Ist das Binary veraltet? Es fehlt, oder eine Quelldatei ist neuer als es.
# web/ gehört dazu: die Oberfläche steckt per include_str! im Binary.
binary_veraltet() {
    [ -x "$bin" ] || return 0
    [ -n "$(find "$repo_root/src" "$repo_root/web" "$repo_root/Cargo.toml" "$repo_root/Cargo.lock" \
        -type f -newer "$bin" -print -quit 2>/dev/null)" ]
}
if [ "$build" = auto ]; then
    if binary_veraltet; then build=1; else build=0; fi
fi

if [ "$dry_run" -eq 1 ]; then
    echo "Würde starten:  $bin"
    echo "  FLUX2_BIND=$FLUX2_BIND"
    echo "  FLUX2_PROJECT=$FLUX2_PROJECT"
    echo "  FLUX2_DATA=$FLUX2_DATA"
    echo "  MODELS_DIR=$MODELS_DIR"
    echo "  MMAP=$MMAP FLASH_ATTENTION=$FLASH_ATTENTION VAE_TILING=$VAE_TILING"
    echo "  FLUX2_API_TOKEN=$([ -n "${FLUX2_API_TOKEN:-}" ] && echo '<gesetzt>' || echo '<nicht gesetzt>')"
    echo "  HF_TOKEN=$([ -n "${HF_TOKEN:-}" ] && echo "<gesetzt, Quelle: $hf_quelle>" || echo '<nicht gesetzt>')"
    echo "  Demo-Modus: $([ "$demo" -eq 1 ] && echo ja || echo nein)"
    echo "  Bauen: $([ "$build" -eq 1 ] && echo ja || echo nein)"
    exit 0
fi

umgebung_pruefen
# Threads: Performance-Kerne, wie bei generate-macos.sh — außer jemand gibt sie vor.
export THREADS="${THREADS:-$(sysctl -n hw.perflevel0.physicalcpu 2>/dev/null || sysctl -n hw.physicalcpu 2>/dev/null || echo 8)}"

if [ "$build" -eq 1 ]; then
    # Das erste Mal dauert der C++-Teil von stable-diffusion.cpp lange.
    bauen "$repo_root"
else
    echo "Binary ist aktuell, kein Build nötig (erzwingen: --build)."
fi
[ -x "$bin" ] || die "Binary fehlt: $bin — ohne --no-build starten, damit es gebaut wird."

mkdir -p "$FLUX2_DATA"
say "flux2-api"
echo "Oberfläche und API: http://$FLUX2_BIND/"
if [ "$demo" -eq 1 ]; then
    echo "DEMO-MODUS: Platzhalterbilder, es werden keine Modelle geladen oder benutzt."
else
    echo "Modelle: $MODELS_DIR (fehlende lädt der erste Job herunter)"
fi
echo "Daten:   $FLUX2_DATA"
[ -z "$hf_quelle" ] || echo "HF_TOKEN: gesetzt (Quelle: $hf_quelle)"
echo "Stoppen: Ctrl-C — ein gerade laufendes Bild geht dabei verloren, wartende Jobs laufen beim nächsten Start weiter."

# exec: Ctrl-C und kill erreichen direkt die API, keine Shell dazwischen.
exec "$bin"
