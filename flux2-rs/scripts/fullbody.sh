#!/usr/bin/env bash
#
# Ganzkörper-Illustration erzeugen — Figur von Kopf bis Fuß in einer Szene,
# statt Kopf-und-Schultern (portrait.sh) oder freizustellendem Token (token.sh).
#
# Der Stil steht hier fest, Figur und Szene kommen als Argument:
#
#   scripts/fullbody.sh "sad young orphan girl standing barefoot in a snowy \
#       medieval town street, worn patched blue-gray dress, messy dark brown \
#       hair, pale face, melancholic expression, timber-framed houses, small \
#       winter market, villagers in background, snow-covered cobblestones"
#
# Anders als token.sh gibt es hier kein Freistellen: die Szene gehört zum
# Bild, kein Hintergrund zum Ausschneiden. --keep-bg ist deshalb fest
# verdrahtet, wie bei location.sh und portrait.sh.
#
# Bewusst nur POSIX-nahe Bash-Konstrukte: macOS liefert bis heute Bash 3.2 aus.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$repo_root/scripts/umgebung.sh"

# --- Stil: ändern heißt, die Reihe neu zu beginnen ----------------------------
#
# Verneinungen fehlen hier absichtlich — derselbe Grund wie bei Tokens, Orten
# und Porträts: bei cfg_scale 1.0 (klein ist distilliert) gibt es keinen
# Negativ-Prompt, eine Verneinung im Positiv-Prompt kodiert nur das
# ausgeschlossene Wort. Was ausgeschlossen werden soll, wird positiv gesagt.
#
# "entire figure shown from head to toe" steht bewusst so explizit da —
# "full-body" allein sagt nur, DASS die ganze Figur zu sehen ist, nicht dass
# sie den Rahmen von oben bis unten ausfüllen soll. Dasselbe Problem wie bei
# PORTRAIT_STYLE in portrait.sh, nur mit der senkrechten statt der
# horizontalen Achse.
#
# Stil seit Asandra.jpg (examples/): malerische, halbrealistische Fantasy-
# Splash-Art statt Anime-Linework. Beschrieben werden nur Technik, Palette,
# Licht und Tiefe — nichts von Asandras Figur, Pose oder Kleidung, sonst
# käme jede Figur als Variation von ihr heraus. Figur und Szene liefert allein
# die Zeile in fullbody.txt bzw. das Argument.
FULLBODY_STYLE="full-body fantasy character illustration, vertical portrait \
orientation, entire figure shown from head to toe filling the frame, \
natural dynamic pose, painterly semi-realistic concept art in the style of \
modern fantasy game splash art, soft visible brushwork, expressive face with \
large detailed eyes, worn practical clothing with believable fabric folds, \
muted desaturated palette of cool blue-grey tones with warm brown accents, \
soft diffused daylight, gentle atmospheric perspective, softly blurred \
background scene with pale hazy depth and small figures far behind, shallow \
depth of field, subtle dust in the air, polished professional illustration"

# Anweisung vor dem Prompt, wenn --style-ref ein Referenzbild mitgibt. Ohne
# Anweisung übernimmt FLUX.2 bei einer Referenz Figur, Pose und Komposition —
# hier soll nur die Malweise einfließen. Positiv formuliert (kein Negativ-Prompt
# bei cfg_scale 1.0): erst sagen, was übernommen wird, dann, dass alles andere
# neu entsteht.
STYLE_REF_HINT="Use the reference image only as a style guide for its painterly \
brushwork, color palette, lighting and atmospheric depth. Create a completely \
new character and scene, with a different face, figure, pose, clothing and setting."

# --- Vorgaben für jede Illustration --------------------------------------------
# Seitenverhältnis 1:2 statt quadratisch wie bei token.sh/portrait.sh — eine
# stehende Figur von Kopf bis Fuß braucht die Höhe, nicht die Breite.
width=768
height=1536
steps=8

charakter=""
out=""
style_ref=""
dry_run=0
extra=()

usage() {
    cat <<'USAGE'
Aufruf: scripts/fullbody.sh [Optionen] "charakter- und szenenbeschreibung"

  -o, --out DATEI    Zieldatei (Default: aus der Beschreibung abgeleitet)
      --style TEXT   Stil ersetzen — nur für Experimente, sonst bricht die Reihe
      --style-ref DATEI
                     Bild als reine Stilvorlage (Malweise, Palette, Licht); Figur
                     und Szene entstehen neu aus der Beschreibung
      --models DIR   Modellverzeichnis (Default: <projekt>/models bzw. <repo>/models)
  -n, --dry-run      Nur zeigen, was aufgerufen würde
  -h, --help         Diese Hilfe

Alle weiteren Optionen gehen unverändert an generate-macos.sh und überschreiben
dessen Vorgaben, z. B. --seeds 5, --seed -1, -W 1024 -H 2048, --steps 12.

Beispiele:
  scripts/fullbody.sh "sad young orphan girl standing barefoot in a snowy \
medieval town street, worn patched blue-gray dress, messy dark brown hair, \
pale face, melancholic expression, timber-framed houses, small winter \
market, villagers in background, snow-covered cobblestones"

  scripts/fullbody.sh --seeds 3 --seed -1 "male mountain dwarf cleric, dark \
iron mail, warhammer, long braided beard, standing in a torch-lit stone hall"

  scripts/fullbody.sh --style-ref examples/Asandra.jpg "female dwarf blacksmith, \
red braided hair, leather apron, carrying a hammer, busy harbor town street"
USAGE
}

die() { echo "$*" >&2; exit 2; }

while [ $# -gt 0 ]; do
    case "$1" in
        -o|--out)     out="$2"; shift 2 ;;
        --style)      FULLBODY_STYLE="$2"; shift 2 ;;
        --style-ref)  style_ref="$2"; shift 2 ;;
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

[ -n "$charakter" ] || { usage >&2; die "Keine Charakter- und Szenenbeschreibung angegeben."; }

# Dateiname aus dem ersten Teil der Beschreibung: "sad young orphan girl
# standing barefoot …" wird zu sad-young-orphan-girl-standing-barefoot.png.
if [ -z "$out" ]; then
    slug="$(printf '%s' "${charakter%%,*}" \
        | tr '[:upper:]' '[:lower:]' \
        | tr -c '[:alnum:]' '-' \
        | sed -e 's/--*/-/g' -e 's/^-//' -e 's/-$//')"
    [ -n "$slug" ] || die "Aus '$charakter' lässt sich kein Dateiname ableiten — bitte -o angeben."
    out="$slug.png"
fi

models_default="$(standard_modelle "$repo_root")"

prompt="$charakter, $FULLBODY_STYLE"

# Stilvorlage: Referenzbild plus Anweisung vor der Beschreibung. Das Bild muss
# existieren — diffusion-rs würde eine fehlende Referenz sonst still überspringen
# und minutenlang ohne sie rechnen.
ref_args=()
if [ -n "$style_ref" ]; then
    [ -f "$style_ref" ] || die "--style-ref: Datei nicht gefunden: $style_ref"
    prompt="$STYLE_REF_HINT $prompt"
    ref_args=(-r "$style_ref")
fi

set -- \
    --models "$models_default" \
    -W "$width" -H "$height" --steps "$steps" \
    -o "$out" --keep-bg \
    ${ref_args[@]+"${ref_args[@]}"} \
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
