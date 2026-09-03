#!/usr/bin/env python3
"""Build unpacked and ZIP browser-extension bundles for Chromium and Firefox."""
from __future__ import annotations

import json
import shutil
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "extension"
DIST = ROOT / "dist"
FILES = [
    "background.js",
    "content.js",
    "tasks.html",
    "tasks.css",
    "tasks.js",
    "options.html",
]


def build(browser: str, manifest: dict) -> None:
    target = DIST / browser
    archive = DIST / f"mesh-tasks-{browser}.zip"
    if target.exists():
        shutil.rmtree(target)
    target.mkdir(parents=True)
    for name in FILES:
        shutil.copy2(SOURCE / "src" / name, target / name)
    (target / "manifest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    if archive.exists():
        archive.unlink()
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as bundle:
        for path in sorted(target.rglob("*")):
            if path.is_file():
                bundle.write(path, path.relative_to(target))
    print(f"built {browser}: {target} and {archive}")


def main() -> None:
    base = json.loads((SOURCE / "manifest.base.json").read_text(encoding="utf-8"))
    chrome = dict(base)
    chrome["name"] = "МЭШ: все задания (Chromium)"
    firefox = dict(base)
    firefox["name"] = "МЭШ: все задания (Firefox/LibreWolf)"
    firefox["browser_specific_settings"] = {
        "gecko": {
            "id": "mesh-tasks@quadrotez.local",
            "strict_min_version": "109.0",
        }
    }
    build("chrome", chrome)
    build("firefox", firefox)


if __name__ == "__main__":
    main()
