#!/usr/bin/env bash
#
# Charakter-Token für die Asset-Bibliothek erzeugen.
#
# Stil und Pose stehen hier fest, damit jedes Token derselben Kunstrichtung folgt —
# eine Variable statt Copy-Paste ist die einzige Art, einen Prompt über viele Läufe
# hinweg wirklich zeichengenau gleich zu halten. Von außen kommt nur der Charakter.
#
#   scripts/token.sh "female human warrior, weathered steel plate armor, longsword"
#   scripts/token.sh --seeds 5 --seed -1 "dwarf cleric, dark iron mail, warhammer"
#
# Alles, was dieses Script nicht selbst kennt, reicht es unverändert an
# generate-macos.sh weiter und zwar *nach* den eigenen Vorgaben — dadurch
# gewinnt deine Angabe: `scripts/token.sh -s 512 "..."` überschreibt die 1024.
#
# Bewusst nur POSIX-nahe Bash-Konstrukte: macOS liefert bis heute Bash 3.2 aus.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# --- Kunstrichtung: ändern heißt, die Bibliothek neu zu beginnen ---------------
#
# Zum Licht: "soft upper-left lighting" erzeugt zuverlässig auch einen
# Schlagschatten auf dem Boden. Eine Verneinung hilft dagegen nicht — bei
# cfg_scale 1.0 gibt es keinen Negativ-Prompt, und "no cast shadow" kodiert im
# Positiv-Prompt das Wort *shadow*. Der Weg ist, flaches Licht zu verlangen:
# die Plastik der Figur kommt dann aus dem Cel-Shading, nicht aus einer
# gerichteten Lichtquelle. Was doch an Schatten entsteht, entfernt das
# Farb-Keying beim Freistellen.
# Zur Kamera: "30-degree elevated three-quarter view" ist eine Fachbeschreibung,
# kein Begriff aus den Trainingsdaten — das Modell malt darauf meist eine
# Frontalansicht auf Augenhöhe. Was wirkt, sind geläufige Formulierungen:
# "high angle" für die Kamerahöhe und "turned three-quarters" für die Drehung
# der Figur. Beides muss getrennt gesagt werden, sonst kommt nur eins davon.
STYLE="hand-painted fantasy character illustration, clean dark-brown ink outlines, \
soft painterly cel shading, matte finish, warm desaturated medieval colors, \
flat even ambient lighting, isolated asset, high angle view looking down on the \
figure from above, tabletop miniature standing on a pale elliptical sandstone base \
seen from above, full body, realistic proportions, solid mid-grey background"

# Zur Pose: die Drehung kostet Anatomie. Ein verdrehter Rumpf mit zurück-
# gedrehtem Kopf ist für das Modell der schwerste Fall, und es bricht zuerst an
# Händen und Füßen — verdrehte Stiefel sind die häufigste Folge. Deshalb dreht
# sich der Kopf jetzt mit dem Körper statt dagegen, und die Füße werden
# ausdrücklich benannt: was im Prompt steht, platziert das Modell bewusst.
POSE="calm upright pose, body turned three-quarters to the left, \
head facing the same way as the body, standing evenly on both feet, \
both boots flat on the base and pointing the same way as the body, \
arms and equipment close to body"

# --- Vorgaben für jedes Token -------------------------------------------------
size=1024
steps=8
token_w=138
token_h=244

# Der Hintergrund ist in STYLE festgelegt, also ist Farb-Keying hier die richtige
# Wahl: es behält den Sockel, den u2netp als Beiwerk wegschneiden würde.
keying=1

charakter=""
out=""
dry_run=0
extra=()

usage() {
    cat <<'USAGE'
Aufruf: scripts/token.sh [Optionen] "charakterbeschreibung"

  -o, --out DATEI    Zieldatei (Default: aus der Beschreibung abgeleitet)
      --pose TEXT    Pose ersetzen
      --style TEXT   Stil ersetzen — nur für Experimente, sonst bricht die Reihe
      --models DIR   Modellverzeichnis (Default: <projekt>/models bzw. <repo>/models)
  -n, --dry-run      Nur zeigen, was aufgerufen würde
  -h, --help         Diese Hilfe

Alle weiteren Optionen gehen unverändert an generate-macos.sh und überschreiben
dessen Vorgaben, z. B. --seeds 5, --seed -1, -s 512, --steps 12, --keep-bg.

Beispiele:
  scripts/token.sh "female human warrior, weathered steel plate armor, longsword"
  scripts/token.sh --seeds 5 --seed -1 "dwarf cleric, dark iron mail, warhammer"
  scripts/token.sh -r kriegerin.png "the same warrior, kneeling, shield raised"
USAGE
}

die() { echo "$*" >&2; exit 2; }

while [ $# -gt 0 ]; do
    case "$1" in
        -o|--out)     out="$2"; shift 2 ;;
        --pose)       POSE="$2"; shift 2 ;;
        --style)      STYLE="$2"; shift 2 ;;
        --models)     extra+=("--models" "$2"); shift 2 ;;
        -n|--dry-run) dry_run=1; shift ;;
        --no-key)     keying=0; shift ;;
        -h|--help)    usage; exit 0 ;;
        # Optionen mit Wert müssen den Wert mitnehmen, Schalter nicht.
        -s|--size|-W|--width|-H|--height|--steps|--seed|--seeds|--out-w|--out-h|\
        --cutoff|--threads|--quant|--cfg|--guidance|--strength|--ref-bg|-r|--ref|--init|\
        --preset|--wtype|--llm|--key-color|--key-innen|--key-aussen|--key-loch|--key-loch-min)
            extra+=("$1" "$2"); shift 2 ;;
        -*)           extra+=("$1"); shift ;;
        *)            charakter="$1"; shift ;;
    esac
done

[ -n "$charakter" ] || { usage >&2; die "Keine Charakterbeschreibung angegeben."; }

# Dateiname aus dem ersten Teil der Beschreibung: "female human warrior, …"
# wird zu female-human-warrior.png. Reproduzierbar und ohne Überraschungen.
if [ -z "$out" ]; then
    slug="$(printf '%s' "${charakter%%,*}" \
        | tr '[:upper:]' '[:lower:]' \
        | tr -c '[:alnum:]' '-' \
        | sed -e 's/--*/-/g' -e 's/^-//' -e 's/-$//')"
    [ -n "$slug" ] || die "Aus '$charakter' lässt sich kein Dateiname ableiten — bitte -o angeben."
    out="$slug.png"
fi

# Modelle: erst neben dem Projekt suchen (dort liegen sie bei geteiltem
# Container-Mount), sonst im Repo selbst.
if [ -d "$repo_root/../models" ]; then
    models_default="$(cd "$repo_root/.." && pwd)/models"
else
    models_default="$repo_root/models"
fi

prompt="$charakter, $POSE, $STYLE"

set -- \
    --models "$models_default" \
    -s "$size" --steps "$steps" \
    -o "$out" --out-w "$token_w" --out-h "$token_h" \
    $( [ "$keying" -eq 1 ] && echo --key ) \
    ${extra[@]+"${extra[@]}"} \
    "$prompt"

# Zeigt den Aufruf zum Kopieren: Option und Wert je Zeile, alles mit Leerzeichen
# in Anführungszeichen. %q wäre korrekt, aber bei einem Prompt aus 60 Wörtern
# unlesbar — und kopieren will man ihn ja gerade.
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
