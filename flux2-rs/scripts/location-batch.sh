#!/usr/bin/env bash
#
# Arbeitet orte.txt ab und erzeugt für jeden Eintrag eine Ortsillustration.
#
#   scripts/location-batch.sh                  # alles, was noch fehlt
#   scripts/location-batch.sh --dry-run        # nur zeigen, was zu tun wäre
#   scripts/location-batch.sh --from castle-ravenloft
#   scripts/location-batch.sh --only sharn --seeds 3 --seed -1
#
# Fortsetzen ist der Normalfall: fertige Illustrationen werden übersprungen. Der
# Lauf darf jederzeit mit Ctrl-C abgebrochen werden — die gerade entstehende
# Datei wird dabei weggeräumt, damit sie beim nächsten Mal nicht als "fertig" gilt.
#
# Fast identisch zu batch.sh (dort für charaktere.txt/token.sh) — hier für
# orte.txt/location.sh. Zwei Kopien derselben Ablaufsteuerung wären nach der
# ersten Änderung auseinandergelaufen, deshalb der Verweis: Änderungen an der
# Ablauflogik selbst (Fortsetzen, Abbruch, Fehlersammlung) bitte in beiden
# Scripts nachziehen, bis sie sich eine gemeinsame Bibliothek teilen.
#
# Bewusst nur POSIX-nahe Bash-Konstrukte: macOS liefert bis heute Bash 3.2 aus.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

liste="$repo_root/orte.txt"
out_dir="locations"
force=0
dry_run=0
von=""
nur=""
extra=()

usage() {
    cat <<'USAGE'
Aufruf: scripts/location-batch.sh [Optionen] [-- Optionen für location.sh]

      --liste DATEI   Ortsliste (Default: <repo>/orte.txt)
      --out-dir DIR   Zielverzeichnis (Default: locations)
      --from SLUG     erst ab diesem Eintrag beginnen
      --only SLUG     nur diesen einen Eintrag
      --force         auch vorhandene Illustrationen neu erzeugen
  -n, --dry-run       nur zeigen, was zu tun wäre
  -h, --help          diese Hilfe

Alle weiteren Optionen gehen an location.sh und damit an generate-macos.sh weiter,
z. B. --large, --seeds 3, --seed -1, --steps 12.

Fortsetzen: einfach erneut aufrufen. Vorhandene Dateien im Zielverzeichnis
gelten als erledigt und werden übersprungen.
USAGE
}

die() { echo "$*" >&2; exit 2; }
trim() { printf '%s' "$1" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//'; }

while [ $# -gt 0 ]; do
    case "$1" in
        --liste)      liste="$2"; shift 2 ;;
        --out-dir)    out_dir="$2"; shift 2 ;;
        --from)       von="$2"; shift 2 ;;
        --only)       nur="$2"; shift 2 ;;
        --force)      force=1; shift ;;
        -n|--dry-run) dry_run=1; shift ;;
        -h|--help)    usage; exit 0 ;;
        --)           shift; while [ $# -gt 0 ]; do extra+=("$1"); shift; done ;;
        *)            extra+=("$1"); shift ;;
    esac
done

[ -f "$liste" ] || die "Ortsliste nicht gefunden: $liste"
mkdir -p "$out_dir"

# Beim Abbruch die halbfertige Datei entfernen: sonst gilt sie beim nächsten
# Lauf als erledigt, und der Ort fehlt am Ende ohne Hinweis.
laufendes_ziel=""
abbruch() {
    echo
    if [ -n "$laufendes_ziel" ]; then
        rm -f "$laufendes_ziel" "${laufendes_ziel%.*}.raw.png"
        echo "Abgebrochen bei: $(basename "${laufendes_ziel%.png}") — halbfertige Datei entfernt."
    else
        echo "Abgebrochen."
    fi
    echo "Weitermachen mit demselben Aufruf; fertige Illustrationen werden übersprungen."
    exit 130
}
trap abbruch INT TERM

gesamt=0
while IFS='|' read -r slug _ _ _ <&3; do
    case "$(trim "${slug:-}")" in ''|\#*) continue ;; esac
    gesamt=$(( gesamt + 1 ))
done 3< "$liste"

nummer=0
erzeugt=0
uebersprungen=0
fehlgeschlagen=""
angekommen=0
[ -n "$von" ] || angekommen=1
start_gesamt=$SECONDS

while IFS='|' read -r roh_slug roh_name roh_beschreibung roh_prompt <&3; do
    slug="$(trim "${roh_slug:-}")"
    case "$slug" in ''|\#*) continue ;; esac
    nummer=$(( nummer + 1 ))

    name="$(trim "${roh_name:-}")"
    prompt="$(trim "${roh_prompt:-}")"
    [ -n "$prompt" ] || die "Zeile $nummer ($slug) hat keinen Prompt — vier Felder mit | erwartet."

    [ "$slug" = "$von" ] && angekommen=1
    [ "$angekommen" -eq 1 ] || continue
    if [ -n "$nur" ] && [ "$slug" != "$nur" ]; then
        continue
    fi

    ziel="$out_dir/$slug.png"
    if [ -f "$ziel" ] && [ "$force" -eq 0 ]; then
        printf '[%2d/%2d] %-22s vorhanden, übersprungen\n' "$nummer" "$gesamt" "$slug"
        uebersprungen=$(( uebersprungen + 1 ))
        continue
    fi

    printf '\n[%2d/%2d] %-22s %s\n' "$nummer" "$gesamt" "$slug" "$name"
    if [ "$dry_run" -eq 1 ]; then
        echo "         $prompt"
        continue
    fi

    laufendes_ziel="$ziel"
    if "$repo_root/scripts/location.sh" -o "$ziel" ${extra[@]+"${extra[@]}"} "$prompt"; then
        erzeugt=$(( erzeugt + 1 ))
    else
        fehlgeschlagen="$fehlgeschlagen$slug
"
        rm -f "$ziel"
    fi
    laufendes_ziel=""

    # Grobe Restschätzung aus dem Mittel der bisherigen Läufe.
    if [ "$erzeugt" -gt 0 ]; then
        schnitt=$(( (SECONDS - start_gesamt) / erzeugt ))
        offen=$(( gesamt - nummer ))
        rest=$(( schnitt * offen ))
        printf 'Schnitt %d:%02d min, noch %d offen, geschätzt %d:%02d h\n' \
            $(( schnitt / 60 )) $(( schnitt % 60 )) "$offen" \
            $(( rest / 3600 )) $(( (rest % 3600) / 60 ))
    fi
done 3< "$liste"

echo
echo "=== Fertig"
echo "  erzeugt:        $erzeugt"
echo "  übersprungen:   $uebersprungen"
if [ -n "$fehlgeschlagen" ]; then
    echo "  fehlgeschlagen:" >&2
    printf '%s' "$fehlgeschlagen" | sed 's/^/    /' >&2
    exit 1
fi
