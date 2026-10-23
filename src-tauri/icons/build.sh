#!/bin/sh
# Renders every icon file the bundle ships from icon.svg and icon-small.svg.
# Sizes up to 32 px come from icon-small.svg, which is drawn for them; scaling
# icon.svg down that far turns the branch into a smudge.
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
