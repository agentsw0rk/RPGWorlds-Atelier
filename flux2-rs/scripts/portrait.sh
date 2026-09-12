#!/usr/bin/env bash
#
# Porträt-Illustration erzeugen — Kopf-und-Schultern statt Ganzkörper-Token.
#
# Der Stil steht hier fest, die Charakterbeschreibung kommt als Argument:
#
#   scripts/portrait.sh "male mountain dwarf cleric, dark iron mail, warhammer, \
#       long braided beard, holy symbol on chest"
#
# Anders als token.sh gibt es hier kein Freistellen: das Pergament-Beige ist
# Teil des Bildes, kein Hintergrund zum Ausschneiden. --keep-bg ist deshalb
# fest verdrahtet, wie bei location.sh.
#
# Bewusst nur POSIX-nahe Bash-Konstrukte: macOS liefert bis heute Bash 3.2 aus.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$repo_root/scripts/umgebung.sh"

# --- Stil: ändern heißt, die Reihe neu zu beginnen ----------------------------
#
# Die drei Verneinungen aus dem ursprünglichen Entwurf ("no scenery", "no
# text", "no frame") fehlen hier absichtlich — derselbe Grund wie bei den
# Charakter-Tokens und den Ortsillustrationen: bei cfg_scale 1.0 (klein ist
# distilliert) gibt es keinen Negativ-Prompt, eine Verneinung im Positiv-Prompt
# kodiert nur das ausgeschlossene Wort. "plain warm beige parchment background"
# trägt "no scenery" schon positiv; für den Rest gibt es hier ohnehin kein
# Freistellen, das einen Rest aufräumen könnte — anders als bei token.sh ist
# eine Verneinung hier also nicht einmal durch Nachbearbeitung abgesichert.
PORTRAIT_STYLE="D&D fantasy character portrait, head and shoulders, \
three-quarter view, face clearly visible, centered composition, hand-painted \
medieval fantasy illustration, fine dark brown ink outlines, soft painterly \
cel shading, subtle watercolor and parchment texture, natural facial \
features, expressive eyes, realistic proportions, detailed hair and \
clothing, restrained medieval fantasy design, warm earthy colors, muted \
brown green ochre and cream palette, soft warm light from the upper left, \
gentle shadows around the face, plain warm beige parchment background"

# --- Vorgaben für jedes Porträt ------------------------------------------------
size=1024
steps=8

charakter=""
out=""
dry_run=0
extra=()

usage() {
    cat <<'USAGE'
Aufruf: scripts/portrait.sh [Optionen] "charakterbeschreibung"

  -o, --out DATEI    Zieldatei (Default: aus der Beschreibung abgeleitet)
      --style TEXT   Stil ersetzen — nur für Experimente, sonst bricht die Reihe
      --models DIR   Modellverzeichnis (Default: <projekt>/models bzw. <repo>/models)
  -n, --dry-run      Nur zeigen, was aufgerufen würde
  -h, --help         Diese Hilfe

Alle weiteren Optionen gehen unverändert an generate-macos.sh und überschreiben
dessen Vorgaben, z. B. --seeds 5, --seed -1, -s 1280, --steps 12.

Beispiele:
  scripts/portrait.sh "male mountain dwarf cleric, dark iron mail, warhammer, \
long braided beard, holy symbol on chest"

  scripts/portrait.sh --seeds 3 --seed -1 "female tiefling warlock, dark violet \
skin, long curved horns, tattered robes"
USAGE
}

die() { echo "$*" >&2; exit 2; }

while [ $# -gt 0 ]; do
    case "$1" in
        -o|--out)     out="$2"; shift 2 ;;
        --style)      PORTRAIT_STYLE="$2"; shift 2 ;;
        --models)     extra+=("--models" "$2"); shift 2 ;;
        -n|--dry-run) dry_run=1; shift ;;
        -h|--help)    usage; exit 0 ;;
        # Optionen mit Wert müssen den Wert mitnehmen, Schalter nicht.
        -s|--size|-W|--width|-H|--height|--steps|--seed|--seeds|\
        --cutoff|--threads|--quant|--cfg|--guidance|--strength|--ref-bg|-r|--ref|--init|\
        --preset|--wtype|--llm|--key-color|--key-innen|--key-aussen|--key-loch|--key-loch-min)
            extra+=("$1" "$2"); shift 2 ;;
        -*)           extra+=("$1"); shift ;;
        *)            charakter="$1"; shift ;;
    esac
done

[ -n "$charakter" ] || { usage >&2; die "Keine Charakterbeschreibung angegeben."; }

# Dateiname aus dem ersten Teil der Beschreibung: "male mountain dwarf
# cleric, …" wird zu male-mountain-dwarf-cleric.png.
if [ -z "$out" ]; then
    slug="$(printf '%s' "${charakter%%,*}" \
        | tr '[:upper:]' '[:lower:]' \
        | tr -c '[:alnum:]' '-' \
        | sed -e 's/--*/-/g' -e 's/^-//' -e 's/-$//')"
    [ -n "$slug" ] || die "Aus '$charakter' lässt sich kein Dateiname ableiten — bitte -o angeben."
    out="$slug.png"
fi

models_default="$(standard_modelle "$repo_root")"

prompt="$charakter, $PORTRAIT_STYLE"

set -- \
    --models "$models_default" \
    -s "$size" --steps "$steps" \
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
