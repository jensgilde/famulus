#!/usr/bin/env bash
# Famulus – CLI-Binaries bauen UND stabil signieren.
#
# Immer dieses Skript benutzen statt nacktem `cargo build --release`,
# sonst sind die Binaries danach adhoc-signiert und macOS vergisst alle
# Ordner-Freigaben (TCC-Identität = Code-Hash, ändert sich pro Build).
# Details: scripts/sign-cli.sh und MAC-SETUP.md.

set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

echo "==> cargo build --release (famulus + famulus-telegram) ..."
cargo build --release --bins

echo "==> Stabil signieren ..."
./scripts/sign-cli.sh
