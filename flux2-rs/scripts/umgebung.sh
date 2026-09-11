# Gemeinsame Grundlage von generate-macos.sh und build.sh.
#
# Kein eigenständiges Script: wird mit `.` eingebunden. Zweck ist, die
# Werkzeugsuche und den Build nur an einer Stelle zu pflegen — zwei Kopien der
# libclang-Suche wären nach der zweiten Änderung auseinandergelaufen.
#
# Bewusst nur POSIX-nahe Bash-Konstrukte: macOS liefert bis heute Bash 3.2 aus.

say() { printf '\n=== %s\n' "$1"; }
die() { echo "$*" >&2; exit 2; }

# Prüft Werkzeuge, setzt LIBCLANG_PATH und CMAKE_GENERATOR, füllt $THREADS.
umgebung_pruefen() {
    say "Umgebung"
    [ "$(uname -s)" = "Darwin" ] || echo "Hinweis: Script ist für macOS gedacht, läuft hier auf $(uname -s)."

    missing=""
    for tool in cargo cmake curl; do
        command -v "$tool" >/dev/null 2>&1 || missing="$missing $tool"
    done
    if [ -n "$missing" ]; then
        echo "Fehlende Werkzeuge:$missing" >&2
        echo "  cargo -> curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" >&2
        echo "  cmake -> brew install cmake" >&2
        exit 1
    fi

    # Ninja ist optional, aber spürbar schneller als Unix Makefiles.
    if command -v ninja >/dev/null 2>&1; then
        export CMAKE_GENERATOR="${CMAKE_GENERATOR:-Ninja}"
    fi

    # bindgen braucht libclang: in Xcode, in den Command Line Tools oder in Homebrews llvm.
    if [ -z "${LIBCLANG_PATH:-}" ]; then
        clt="$(xcode-select -p 2>/dev/null || true)"
        llvm="$(brew --prefix llvm 2>/dev/null || true)"
        # Reihenfolge: Xcode-Toolchain, Command Line Tools, Homebrew-llvm.
        for candidate in \
            "${clt:+$clt/Toolchains/XcodeDefault.xctoolchain/usr/lib}" \
            "${clt:+$clt/usr/lib}" \
            "${llvm:+$llvm/lib}"
        do
            if [ -f "$candidate/libclang.dylib" ]; then
                export LIBCLANG_PATH="$candidate"
                break
            fi
        done
    fi

    # Auf Apple Silicon bringen die Effizienzkerne für ggml wenig — P-Kerne zählen.
    THREADS="$(sysctl -n hw.perflevel0.physicalcpu 2>/dev/null \
        || sysctl -n hw.physicalcpu 2>/dev/null \
        || nproc 2>/dev/null \
        || echo 8)"

    echo "cargo:    $(command -v cargo)"
    echo "cmake:    $(command -v cmake)"
    echo "libclang: ${LIBCLANG_PATH:-<Standardpfad>}"
    echo "Threads:  $THREADS"
    echo "Backend:  Metal (auf Apple-Targets automatisch aktiv)"
}

# Baut beide Binaries im Release-Profil. $1 = Repo-Wurzel.
#
# Fängt die bekannte bindgen-Falle ab: mit clang >= 23 erzeugt diffusion-rs-sys
# Layout-Asserts gegen opake Typen (error[E0080]). Die Asserts stehen in der
# *erzeugten* bindings.rs, nicht in der Quelle — deshalb der Nachschlag im
# Build-Verzeichnis und ein zweiter Versuch.
bauen() {
    wurzel="$1"
    say "Build (beim ersten Mal dauert der C++-Teil von stable-diffusion.cpp lange)"
    # mktemp -t erwartet unter BSD einen Präfix, unter GNU eine Vorlage mit X-en.
    # Ein vollständiger Pfad mit XXXXXX funktioniert auf beiden.
    build_log="$(mktemp "${TMPDIR:-/tmp}/flux2-build.XXXXXX")"
    if ! (cd "$wurzel" && cargo build --release 2>&1 | tee "$build_log"); then
        if grep -q 'E0080' "$build_log" && grep -q 'bindings.rs' "$build_log"; then
            say "bindgen-Layout-Asserts gefunden — entfernen und erneut bauen"
            if ! "$wurzel/scripts/fix-bindgen-layout-tests.sh" "$wurzel/target/release"; then
                echo "Patchen der bindings.rs fehlgeschlagen. Build-Log: $build_log" >&2
                exit 1
            fi
            (cd "$wurzel" && cargo build --release)
        else
            echo "Build fehlgeschlagen, Log: $build_log" >&2
            exit 1
        fi
    fi
    rm -f "$build_log"
}
