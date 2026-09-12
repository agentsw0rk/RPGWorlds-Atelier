#!/usr/bin/env bash
#
# Ortsillustration im Reisejournal-Stil erzeugen — eine vollständige Landschaft
# statt eines freizustellenden Charakter-Tokens.
#
# Der Stil steht hier fest, der Ort kommt als Argument:
#
#   scripts/location.sh "a small remote medieval village, stone church with a \
#       tall narrow bell tower, five timber and stone houses, pine forest \
#       behind the settlement, distant rugged mountains, open village square \
#       in foreground, light morning mist"
#
# Anders als token.sh gibt es hier kein Freistellen: eine Ortsillustration ist
# eine vollständige Szene, kein auszuschneidendes Motiv. --keep-bg ist deshalb
# fest verdrahtet, nicht per Flag abschaltbar.
#
# Bewusst nur POSIX-nahe Bash-Konstrukte: macOS liefert bis heute Bash 3.2 aus.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$repo_root/scripts/umgebung.sh"

# --- Stil: ändern heißt, die Reihe neu zu beginnen ----------------------------
#
# Die sechs Verneinungen aus dem ursprünglichen Entwurf ("no text", "no UI",
# "no modern objects", "no labels", "no frame", "no people in foreground")
# fehlen hier absichtlich. Bei cfg_scale 1.0 (klein ist distilliert) gibt es
# keinen Negativ-Prompt — eine Verneinung im Positiv-Prompt kodiert nur das
# Wort, das sie ausschließen soll. Das haben wir bei den Charakter-Tokens
# zweimal erlebt (Schlagschatten, Kamerawinkel) und hier ist das Risiko eher
# größer: "no text"/"no labels" auf eine "travel journal illustration"
# anzuwenden kann eher Text ins Bild ziehen als ihn fernhalten. Wo eine
# positive Formulierung existiert, steht sie stattdessen in der
# Ortsbeschreibung selbst ("open village square in foreground" statt
# "no people in foreground").
STYLE="small rectangular fantasy location illustration, hand-drawn pen and ink \
engraving, fine dark sepia outlines, subtle watercolor wash, warm aged \
parchment paper, muted beige and brown palette, medieval fantasy travel \
journal illustration, architectural line drawing, fine cross-hatching, \
slightly imperfect hand-drawn lines, flat frontal composition, wide \
landscape view"

# --- Vorgaben für jede Illustration --------------------------------------------
# 1024x512 statt 1536x768: bei doppelter Fläche ist nicht die Bildgröße der
# Engpass, sondern die Wartezeit — --large schaltet auf die größere Kante um,
# wenn es die Detailtiefe braucht.
width=1024
height=512
# Vier Steps (klein-Default) reichen für flächige Farben, aber Kreuzschraffur
# und feine Linien brauchen mehr Gelegenheiten zum Verfeinern — wie bei den
# Charakter-Tokens (token.sh) steht der Wert deshalb höher als der Default.
steps=8

ort=""
out=""
dry_run=0
extra=()

usage() {
    cat <<'USAGE'
Aufruf: scripts/location.sh [Optionen] "ortsbeschreibung"

  -o, --out DATEI    Zieldatei (Default: aus der Beschreibung abgeleitet)
      --style TEXT   Stil ersetzen — nur für Experimente, sonst bricht die Reihe
      --large        1536x768 statt 1024x512
      --models DIR   Modellverzeichnis (Default: <projekt>/models bzw. <repo>/models)
  -n, --dry-run      Nur zeigen, was aufgerufen würde
  -h, --help         Diese Hilfe

Alle weiteren Optionen gehen unverändert an generate-macos.sh und überschreiben
dessen Vorgaben, z. B. --seeds 5, --seed -1, --steps 12, -W 1280 -H 640.

Beispiele:
  scripts/location.sh "a small remote medieval village, stone church with a tall \
narrow bell tower, five timber and stone houses, pine forest behind the settlement, \
distant rugged mountains, open village square in foreground, light morning mist"

  scripts/location.sh --large --seeds 3 --seed -1 "a ruined watchtower on a sea \
cliff, crumbling stone walls, crashing waves below, a single crooked pine, \
overcast sky"
USAGE
}

die() { echo "$*" >&2; exit 2; }

while [ $# -gt 0 ]; do
    case "$1" in
        -o|--out)     out="$2"; shift 2 ;;
        --style)      STYLE="$2"; shift 2 ;;
        --large)      width=1536; height=768; shift ;;
        --models)     extra+=("--models" "$2"); shift 2 ;;
        -n|--dry-run) dry_run=1; shift ;;
        -h|--help)    usage; exit 0 ;;
        # Optionen mit Wert müssen den Wert mitnehmen, Schalter nicht.
        -s|--size|-W|--width|-H|--height|--steps|--seed|--seeds|\
        --cutoff|--threads|--quant|--cfg|--guidance|--strength|--ref-bg|-r|--ref|--init|\
        --preset|--wtype|--llm|--key-color|--key-innen|--key-aussen|--key-loch|--key-loch-min)
            extra+=("$1" "$2"); shift 2 ;;
        -*)           extra+=("$1"); shift ;;
        *)            ort="$1"; shift ;;
    esac
done

[ -n "$ort" ] || { usage >&2; die "Keine Ortsbeschreibung angegeben."; }

# Dateiname aus dem ersten Teil der Beschreibung: "a small remote medieval
# village, …" wird zu a-small-remote-medieval-village.png.
if [ -z "$out" ]; then
    slug="$(printf '%s' "${ort%%,*}" \
        | tr '[:upper:]' '[:lower:]' \
        | tr -c '[:alnum:]' '-' \
        | sed -e 's/--*/-/g' -e 's/^-//' -e 's/-$//')"
    [ -n "$slug" ] || die "Aus '$ort' lässt sich kein Dateiname ableiten — bitte -o angeben."
    out="$slug.png"
fi

models_default="$(standard_modelle "$repo_root")"

prompt="$ort, $STYLE"

set -- \
    --models "$models_default" \
    -W "$width" -H "$height" --steps "$steps" \
    -o "$out" --keep-bg \
    ${extra[@]+"${extra[@]}"} \
    "$prompt"

# Zeigt den Aufruf zum Kopieren: Option und Wert je Zeile, alles mit Leerzeichen
# in Anführungszeichen. %q wäre korrekt, aber bei einem Prompt aus vielen
# Wörtern unlesbar — und kopieren will man ihn ja gerade.
if [ "$dry_run" -eq 1 ]; then
    zitiere() {
        case "$1" in
            *[[:space:]\'\"]*) printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")" ;;
            *)                 printf '%s' "$1" ;;
        esac
    }
    printf '%s' "$repo_root/scripts/generate-macos.sh"
    wert_folgt=0
    for arg in "$@"; do
        if [ "$wert_folgt" -eq 1 ]; then
            # Wert gehört auf dieselbe Zeile wie seine Option — sonst stünde
            # "--seed" und "-1" untereinander.
            printf ' %s' "$(zitiere "$arg")"
            wert_folgt=0
            continue
        fi
        printf ' \\\n    %s' "$(zitiere "$arg")"
        case "$arg" in
            -o|--out|--models|-s|--size|-W|--width|-H|--height|--steps|--seed|--seeds|\
            --out-w|--out-h|--cutoff|--threads|--quant|--cfg|--guidance|--strength|\
            --ref-bg|-r|--ref|--init|--preset|--wtype|--llm|--key-color|--key-innen|\
            --key-aussen|--key-loch|--key-loch-min)
                wert_folgt=1 ;;
        esac
    done
    printf '\n'
    exit 0
fi

exec "$repo_root/scripts/generate-macos.sh" "$@"
