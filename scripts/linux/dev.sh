#!/usr/bin/env bash
# Starts Audio Graph in development mode: interface and Rust changes are picked up while it runs.
set -euo pipefail
cd "$(dirname "$0")/../.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

[ -d node_modules ] || npm install --no-audit --no-fund
exec npm run tauri dev
