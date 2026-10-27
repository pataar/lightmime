#!/bin/sh
# Renders the app icon to PNGs and copies the SVGs into a hicolor icon theme
# tree for packaging.
# Usage: assets/brand/export.sh [outdir]   (default: target/icons)
set -eu
icons=$(dirname "$0")/../icons/hicolor
out=${1:-target/icons}
for size in 16 24 32 48 64 128 256 512; do
    dir="$out/hicolor/${size}x${size}/apps"
    mkdir -p "$dir"
    rsvg-convert -w "$size" -h "$size" "$icons/scalable/apps/lightmime.svg" -o "$dir/lightmime.png"
done
mkdir -p "$out/hicolor/scalable/apps" "$out/hicolor/symbolic/apps"
cp "$icons/scalable/apps/lightmime.svg" "$out/hicolor/scalable/apps/"
cp "$icons/symbolic/apps/lightmime-symbolic.svg" "$out/hicolor/symbolic/apps/"
