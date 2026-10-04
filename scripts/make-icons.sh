#!/usr/bin/env bash
# Regenerates every app icon (macOS, Android launcher, Windows) from the SVGs in crates/flint-tauri/icon-src.
# Needs rsvg-convert (brew install librsvg). Edit glyph.svgfrag, then run this and rebuild the apps.
set -euo pipefail
cd "$(dirname "$0")/../crates/flint-tauri"
python3 - <<'PY'
frag = open("icon-src/glyph.svgfrag").read()
head = '<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">\n'
bg = '<defs><linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#2a3340"/><stop offset="1" stop-color="#171c24"/></linearGradient></defs>'
# desktop: a rounded square with the usual margin; Android: a full-bleed background plus the glyph alone, scaled into the safe zone
open("icon-src/app-icon.svg", "w").write(head + bg + '<rect x="100" y="100" width="824" height="824" rx="185" fill="url(#bg)"/>\n' + frag + "</svg>\n")
open("icon-src/android-bg.svg", "w").write(head + bg + '<rect width="1024" height="1024" fill="url(#bg)"/></svg>\n')
open("icon-src/android-fg.svg", "w").write(head + '<g transform="translate(512 512) scale(0.82) translate(-512 -512)">' + frag + "</g></svg>\n")
PY
for name in app-icon android-bg android-fg; do
  rsvg-convert -w 1024 -h 1024 "icon-src/$name.svg" -o "icon-src/$name.png"
done
cargo tauri icon icon-src/icon.json
rm -rf icons/ios
