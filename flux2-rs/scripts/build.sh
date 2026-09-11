#!/usr/bin/env bash
#
# Baut die beiden Binaries — sonst nichts. Keine Modelle, keine Generierung.
#
#   scripts/build.sh              # Release bauen (inkrementell)
#   scripts/build.sh --tests      # vorher die Unit-Tests laufen lassen
#   scripts/build.sh --clean      # target/ wegwerfen und neu bauen
#
# Der lange C++-Teil (stable-diffusion.cpp) läuft nur, wenn sich an den
# Abhängigkeiten etwas geändert hat. Eine Änderung an src/*.rs kostet Sekunden.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$repo_root/scripts/umgebung.sh"

clean=0
tests=0

usage() {
    cat <<'USAGE'
Aufruf: scripts/build.sh [Optionen]

      --tests   Vor dem Build die Unit-Tests laufen lassen (Sekunden, ohne C++)
      --clean   target/ vorher löschen und alles neu bauen (dauert lange)
  -h, --help    Diese Hilfe

Ohne Optionen wird inkrementell gebaut: nach einer Änderung an src/*.rs sind das
Sekunden, nur beim ersten Mal oder nach --clean der volle C++-Build.
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --tests)   tests=1; shift ;;
        --clean)   clean=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *)         echo "Unbekannte Option: $1" >&2; usage >&2; exit 2 ;;
    esac
done

umgebung_pruefen

if [ "$tests" -eq 1 ]; then
    # --no-default-features lässt diffusion-rs/sd.cpp außen vor: die testbare
    # Hälfte (Freistellen, Keying, Parameter) baut und läuft in Sekunden.
    say "Tests (ohne C++-Teil)"
    (cd "$repo_root" && cargo test --no-default-features)
fi

if [ "$clean" -eq 1 ]; then
    say "target/ löschen"
    # Achtung: danach läuft das build.rs von diffusion-rs-sys neu, und damit ist
    # die gepatchte bindings.rs weg. bauen() fängt den Fehler ab und patcht neu.
    rm -rf "$repo_root/target"
fi

started=$SECONDS
bauen "$repo_root"
elapsed=$(( SECONDS - started ))

say "Fertig"
printf 'Dauer: %d:%02d min\n' $(( elapsed / 60 )) $(( elapsed % 60 ))
for bin in flux2-rs matte; do
    pfad="$repo_root/target/release/$bin"
    if [ -x "$pfad" ]; then
        echo "  $pfad"
    else
        echo "  FEHLT: $pfad" >&2
    fi
done
