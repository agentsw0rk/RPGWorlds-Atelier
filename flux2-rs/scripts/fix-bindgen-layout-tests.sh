#!/usr/bin/env bash
# Entfernt die Layout-Asserts aus den von bindgen erzeugten Bindings.
#
# Hintergrund: bindgen 0.71 erzeugt für opake Typen (z. B. glibcs `_IO_FILE`)
# Asserts der Form `["Size of _IO_FILE"][size_of::<_IO_FILE>() - 216usize]`,
# während der Typ selbst mit Größe 1 generiert wird. Ergebnis:
# `error[E0080]: attempt to compute 1_usize - 216_usize`.
#
# Das ist ein Notnagel am *Build-Output*, keine Reparatur der Quelle: sobald
# Cargo das build.rs von diffusion-rs-sys erneut ausführt, ist der Fehler zurück.
# Dauerhafte Lösung: diffusion-rs-sys vendoren und `.layout_tests(false)` an den
# `bindgen::Builder` hängen.
set -euo pipefail

target_dir="${1:-target}"
found=0

while IFS= read -r bindings; do
    # Ohne Treffer liefert grep 1 zurück und würde mit `set -o pipefail` den
    # ganzen Lauf beenden — genau im Erfolgsfall, wenn nichts mehr übrig ist.
    count=$( { grep -Fo "const _ : () = {" "$bindings" || true; } | wc -l | tr -d " ")
    [ "$count" -eq 0 ] && continue
    perl -0pi -e 's/#\s*\[\s*allow\s*\(\s*clippy\s*::\s*unnecessary_operation\s*,\s*clippy\s*::\s*identity_op\s*\)\s*\]\s*const\s+_\s*:\s*\(\)\s*=\s*\{[^{}]*\}\s*;//g' "$bindings"
    rest=$( { grep -Fo "const _ : () = {" "$bindings" || true; } | wc -l | tr -d " ")
    echo "  $bindings: $((count - rest)) Layout-Asserts entfernt, $rest verblieben"
    found=$((found + 1))
done < <(find "$target_dir" -path '*diffusion-rs-sys-*/out/bindings.rs' 2>/dev/null)

if [ "$found" -eq 0 ]; then
    echo "Keine bindgen-Ausgabe mit Layout-Asserts unter $target_dir gefunden." >&2
    exit 1
fi
