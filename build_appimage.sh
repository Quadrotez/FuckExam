#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

BUILD_VENV="$ROOT/.build-venv"
if [[ ! -x "$BUILD_VENV/bin/python" ]]; then
  python3 -m venv "$BUILD_VENV"
fi
"$BUILD_VENV/bin/python" -m pip install --upgrade pip pyinstaller Pillow obsws-python websocket-client
rm -rf build dist AppDir
"$BUILD_VENV/bin/python" -m PyInstaller --noconfirm --clean --windowed --name FuckExam --paths src --collect-all PIL --collect-all obsws_python --collect-all websocket src/FuckExam/__main__.py

mkdir -p AppDir/usr/bin AppDir/usr/share/applications AppDir/usr/share/icons/hicolor/256x256/apps
cp -a dist/FuckExam/. AppDir/usr/bin/
cp fuckexam.svg AppDir/fuckexam.svg
cp fuckexam.svg AppDir/usr/share/icons/hicolor/256x256/apps/fuckexam.svg
cat > AppDir/fuckexam.desktop <<'DESKTOP'
[Desktop Entry]
Name=FuckExam
Comment=Remote VM observer prototype
Exec=FuckExam
Icon=fuckexam
Type=Application
Categories=Utility;
Terminal=false
DESKTOP
cp AppDir/fuckexam.desktop AppDir/usr/share/applications/fuckexam.desktop
cat > AppDir/AppRun <<'RUN'
#!/usr/bin/env bash
HERE="$(dirname "$(readlink -f "$0")")"
export FUCKEXAM_PORTABLE_ROOT="$HERE"
exec "$HERE/usr/bin/FuckExam" "$@"
RUN
chmod +x AppDir/AppRun

APPIMAGETOOL="${APPIMAGETOOL:-}"
if [[ -z "$APPIMAGETOOL" && -x "$ROOT/tools/appimagetool" ]]; then APPIMAGETOOL="$ROOT/tools/appimagetool"; fi
if [[ -z "$APPIMAGETOOL" ]]; then
  mkdir -p tools
  ARCH="$(uname -m)"
  if [[ "$ARCH" != "x86_64" ]]; then echo "Automatic appimagetool download supports x86_64 only; set APPIMAGETOOL manually." >&2; exit 2; fi
  APPIMAGETOOL="$ROOT/tools/appimagetool"
  curl -L --fail --retry 3 -o "$APPIMAGETOOL" "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
  chmod +x "$APPIMAGETOOL"
fi

ARCH="$(uname -m)"
"$APPIMAGETOOL" AppDir "dist/FuckExam-${ARCH}.AppImage"
echo "Portable build ready: $ROOT/dist/FuckExam-${ARCH}.AppImage"
echo "Runtime data will be created beside the AppImage in FuckExamData."
