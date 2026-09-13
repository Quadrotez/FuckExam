#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"
git pull --ff-only origin master
rm -rf AppDir build dist
./build_appimage.sh
exec "$ROOT/AppDir/AppRun"
