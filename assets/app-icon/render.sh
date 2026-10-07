#!/bin/sh
# Render the app icon and the logo PNGs:
#   Jig-iOS-Default-1024@1x.png -> jig-1024.png and Jig.icns (the app icon, made in
#                        Icon Composer; macOS wants the tile at 824 of 1024, so it's inset)
#   jig-mark.svg      -> jig.png (the logo, for light backgrounds)
#   jig-mark-dark.svg  -> jig-dark.png (the logo, for dark backgrounds)
#   social-preview.svg -> social-preview.png (GitHub's Social preview, uploaded by hand)
# Quick Look composites onto opaque white, so each SVG is rendered over white
# and over black and the alpha is solved from the difference.
set -eu
cd "$(dirname "$0")"
tmp=$(mktemp -d)

transparent() { # svg, png
  cp "$1" "$tmp/white.svg"
  sed 's#</defs>#</defs><rect width="1024" height="1024" fill="\#000"/>#' "$1" > "$tmp/black.svg"
  for f in white black; do qlmanage -t -s 1024 -o "$tmp" "$tmp/$f.svg" >/dev/null 2>&1; done
  python3 - "$tmp" "$2" <<'PY'
import sys
import numpy as np
from PIL import Image
tmp, out = sys.argv[1:]
w = np.asarray(Image.open(f"{tmp}/white.svg.png").convert("RGB")).astype(float)
b = np.asarray(Image.open(f"{tmp}/black.svg.png").convert("RGB")).astype(float)
a = np.clip(1 - (w - b).mean(axis=2) / 255, 0, 1)
rgb = np.where(a[..., None] > 0, b / np.maximum(a[..., None], 1e-6), 0)
Image.fromarray(np.dstack([np.clip(rgb, 0, 255), a * 255]).astype(np.uint8)).save(out)
PY
}

python3 - <<'PY'
from PIL import Image
icon = Image.new("RGBA", (1024, 1024), (0, 0, 0, 0))
tile = Image.open("Jig-iOS-Default-1024@1x.png").convert("RGBA").resize((824, 824), Image.LANCZOS)
icon.paste(tile, (100, 100))
icon.save("jig-1024.png")
PY
transparent jig-mark.svg jig.png
transparent jig-mark-dark.svg jig-dark.png

qlmanage -t -s 1280 -o "$tmp" social-preview.svg >/dev/null 2>&1
python3 -c "from PIL import Image; Image.open('$tmp/social-preview.svg.png').convert('RGB').crop((0, 320, 1280, 960)).save('social-preview.png')"

set=$tmp/Jig.iconset
mkdir "$set"
for size in 16 32 128 256 512; do
  sips -z $size $size jig-1024.png --out "$set/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z $double $double jig-1024.png --out "$set/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$set" -o Jig.icns
rm -rf "$tmp"
echo "Rendered jig-1024.png, Jig.icns, jig.png, jig-dark.png and social-preview.png"
