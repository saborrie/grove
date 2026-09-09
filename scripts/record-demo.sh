#!/usr/bin/env bash
# Regenerate the README screenshot.
#
#   ./scripts/record-demo.sh [scenario] [output.png]
#
# Scenarios pick what the tree is walked to before the shutter: image (default),
# video, pdf, code.
#
# grove's previews are composited by herdr as kitty graphics, so they are not in
# the escape-sequence stream that asciinema-style recorders capture — a recording
# would show the tree beside an empty rectangle. This renders the real thing
# instead: a real kitty on a virtual X display, with herdr inside it and grove
# inside that, photographed from the X root window.
#
# Everything happens in a container, so the only requirement is Docker, and the
# result does not depend on your fonts, terminal or theme.
#
# Environment:
#   WIDTH / HEIGHT   window size in pixels (default 1500x860)
#   CROP_LEFT        pixels of herdr's frame to crop off the left (default 0)
set -euo pipefail

cd "$(dirname "$0")/.."

SCENARIO="${1:-image}"
OUT="${2:-assets/demo-${SCENARIO}.png}"
WIDTH="${WIDTH:-1500}"
HEIGHT="${HEIGHT:-860}"
IMAGE="grove-demo"

echo "==> building the demo image (first run pulls a Rust toolchain, ~2 min)"
docker build -q -f scripts/demo/Dockerfile -t "$IMAGE" . >/dev/null

mkdir -p "$(dirname "$OUT")"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "==> recording"
docker run --rm \
    -e WIDTH="$WIDTH" -e HEIGHT="$HEIGHT" -e SCENARIO="$SCENARIO" \
    ${CROP_LEFT:+-e CROP_LEFT="$CROP_LEFT"} \
    -v "$tmp:/out" "$IMAGE"

cp "$tmp/demo.png" "$OUT"
echo "==> wrote $OUT"
