#!/usr/bin/env bash
# Famulus – FFI-Bindings erzeugen (v0.11.0).
# Baut den Kern als statische Bibliothek und generiert daraus
# die Swift-Bindings (UniFFI) nach swift-app/Generated.
# Läuft automatisch in build-app.sh; hier einzeln aufrufbar.
# Muster: Famulus Games scripts/build-ffi.sh (dort bewährt seit v0.2.1).

set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

echo "==> Baue Kern + uniffi-bindgen (Release)..."
cargo build --release -p famulus-core --lib
cargo build --release -p famulus-core --bin uniffi-bindgen

LIB="target/release/libfamulus_core.a"
OUT="swift-app/Generated"
if [ ! -f "$LIB" ]; then
    echo "Fehler: $LIB wurde nicht gebaut." >&2
    exit 1
fi

mkdir -p "$OUT"
echo "==> Erzeuge Swift-Bindings via uniffi-bindgen..."
./target/release/uniffi-bindgen generate \
    -l swift \
    -o "$OUT" \
    src/ffi.udl \
    --crate famulus_core

echo "Fertig: $OUT/"
ls "$OUT"
