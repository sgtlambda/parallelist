#!/bin/sh
# Renders every icon file the bundle ships from icon.svg and icon-small.svg.
# Sizes up to 32 px come from icon-small.svg, which is drawn for them; scaling
# icon.svg down that far turns the prompt into a smudge.
#
# Needs rsvg-convert and magick (brew install librsvg imagemagick) and
# iconutil, which ships with macOS.
set -eu
cd "$(dirname "$0")"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

render() { # size, output
  if [ "$1" -le 32 ]; then src=icon-small.svg; else src=icon.svg; fi
  rsvg-convert -w "$1" -h "$1" "$src" -o "$2"
}

render 32 32x32.png
render 64 64x64.png
render 128 128x128.png
render 256 128x128@2x.png
render 512 icon.png

# The Dock icon with its cursor blinked off (src/dock.rs swaps it with
# icon.png): icon.svg without the cursor's subpath, at icon.png's size, so
# nothing but the cursor moves.
cursor='M440 630.5H532'
grep -q "$cursor" icon.svg || { echo "icon.svg: no cursor subpath '$cursor'" >&2; exit 1; }
sed "s/$cursor//" icon.svg >"$tmp/cursor-off.svg"
rsvg-convert -w 512 -h 512 "$tmp/cursor-off.svg" -o dock-cursor-off.png

set_=$tmp/icon.iconset
mkdir "$set_"
for size in 16 32 128 256 512; do
  render "$size" "$set_/icon_${size}x${size}.png"
  render $((size * 2)) "$set_/icon_${size}x${size}@2x.png"
done
iconutil -c icns -o icon.icns "$set_"

for size in 16 24 32 48 64 256; do render "$size" "$tmp/ico-$size.png"; done
magick "$tmp"/ico-16.png "$tmp"/ico-24.png "$tmp"/ico-32.png \
  "$tmp"/ico-48.png "$tmp"/ico-64.png "$tmp"/ico-256.png icon.ico
