#!/usr/bin/env bash
# build.sh — сборка FuckExam под Linux.
#
# Флаги:
#   --binary    только бинарник (cargo build --release)
#   --deb       только .deb пакет
#   --appimage  только .AppImage
#   --no-clean  не пересобирать бинарник заранее (бандлы всё равно подтянут его)
# Без флагов — всё сразу (бинарник + deb + AppImage).
#
# Результат кладётся в ./dist/
set -euo pipefail

cd "$(dirname "$0")"

DIST="dist"
TAURI_DIR="src-tauri"
BIN_SRC="$TAURI_DIR/target/release/fuck-exam"

DO_BINARY=0
DO_DEB=0
DO_APPIMAGE=0
NO_PREBUILD=0

usage() {
    sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'
    exit 1
}

for arg in "$@"; do
    case "$arg" in
        --binary)   DO_BINARY=1 ;;
        --deb)      DO_DEB=1 ;;
        --appimage) DO_APPIMAGE=1 ;;
        --no-clean) NO_PREBUILD=1 ;;
        --help|-h)  usage ;;
        *)          echo "неизвестный флаг: $arg"; usage ;;
    esac
done

if [ "$DO_BINARY" -eq 0 ] && [ "$DO_DEB" -eq 0 ] && [ "$DO_APPIMAGE" -eq 0 ]; then
    DO_BINARY=1
    DO_DEB=1
    DO_APPIMAGE=1
fi

command -v cargo >/dev/null 2>&1 || {
    echo "ОШИБКА: cargo не найден. Установи Rust: https://rustup.rs"; exit 1
}

if [ "$DO_DEB" -eq 1 ] || [ "$DO_APPIMAGE" -eq 1 ]; then
    if ! cargo tauri --version >/dev/null 2>&1 && ! "$TAURI_DIR/target/bin/tauri" --version >/dev/null 2>&1; then
        echo "tauri-cli не установлен. Установи его:"
        echo "  cargo install tauri-cli --version \"^2\""
        exit 1
    fi
fi

mkdir -p "$DIST"

if [ "$DO_BINARY" -eq 1 ]; then
    echo "==> сборка бинарника"
    (cd "$TAURI_DIR" && cargo build --release)
    cp "$BIN_SRC" "$DIST/fuck-exam"
    echo "    -> $DIST/fuck-exam ($(( $(stat -c%s "$DIST/fuck-exam") / 1024 )) КБ)"
    # KDE/Wayland uses the desktop entry and WM_CLASS to associate a
    # directly launched binary with its taskbar icon. Install a per-user
    # launcher so ./dist/fuck-exam gets the same integration as a .deb.
    if [ -n "${XDG_CURRENT_DESKTOP:-}${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]; then
        DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
        ICON_DIR="$DATA_HOME/icons/hicolor/512x512/apps"
        APP_DIR="$DATA_HOME/applications"
        mkdir -p "$ICON_DIR" "$APP_DIR"
        cp "$PWD/src-tauri/icons/icon.png" "$ICON_DIR/fuck-exam.png"
        DIST_ABS="$(realpath "$DIST")"
        cat > "$APP_DIR/fuck-exam.desktop" <<EOF
[Desktop Entry]
Name=FuckExam
Comment=Запись и трансляция окон
Exec=$DIST_ABS/fuck-exam
Icon=fuck-exam
Terminal=false
Type=Application
Categories=Utility;
StartupWMClass=fuck-exam
X-GNOME-WMClass=fuck-exam
EOF
        command -v update-desktop-database >/dev/null 2>&1 && \
            update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
        echo "    -> desktop launcher: $APP_DIR/fuck-exam.desktop"
    fi
fi

if [ "$DO_DEB" -eq 1 ] || [ "$DO_APPIMAGE" -eq 1 ]; then
    if [ "$NO_PREBUILD" -eq 1 ] && [ -x "$BIN_SRC" ]; then
        echo "==> бандлы используют существующий бинарник (--no-clean)"
    else
        echo "==> сборка бинарника для бандлов"
        (cd "$TAURI_DIR" && cargo build --release)
    fi
fi

if [ "$DO_DEB" -eq 1 ]; then
    echo "==> сборка .deb (нужны libwebkit2gtk-4.1-dev и другое)"
    (cd "$TAURI_DIR" && cargo tauri build --bundles deb)
    cp "$TAURI_DIR"/target/release/bundle/deb/*.deb "$DIST/"
    echo "    -> dist/*.deb"
fi

if [ "$DO_APPIMAGE" -eq 1 ]; then
    echo "==> сборка .AppImage (NO_STRIP=1 — старый strip в linuxdeploy не понимает .relr.dyn)"
    # linuxdeploy тянет свой устаревший strip, который падает на кофайлах с .relr.dyn
    # (Arch и свежие дистры). Отключаем его — на размер влияет мало.
    (cd "$TAURI_DIR" && NO_STRIP=1 cargo tauri build --bundles appimage)
    cp "$TAURI_DIR"/target/release/bundle/appimage/*.AppImage "$DIST/"
    echo "    -> dist/*.AppImage"
fi

echo ""
echo "Готово. Артефакты в ./$DIST/:"
ls -lh "$DIST" | grep -vE '^total' | sed 's/^/  /'
