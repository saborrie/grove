#!/bin/sh
# Runs inside the container: bring up a virtual display, put a real kitty on it,
# run herdr inside that, run grove inside that, walk it to something worth
# looking at, and photograph the screen.
set -eu

OUT="${OUT:-/out/demo.png}"
WIDTH="${WIDTH:-1500}"
HEIGHT="${HEIGHT:-860}"
SCENARIO="${SCENARIO:-image}"
# herdr's own sidebar occupies the left of the window. It is not what the shot is
# about, so it is cropped off; set to 0 to keep the whole herdr frame.
CROP_LEFT="${CROP_LEFT:-300}"
FIXTURE=/work/demo-project

IM="$(command -v magick || command -v convert)"

say() { printf '\033[36m==>\033[0m %s\n' "$*"; }

say "starting the virtual display (${WIDTH}x${HEIGHT})"
Xvfb :99 -screen 0 "${WIDTH}x${HEIGHT}x24" -nolisten tcp &
for _ in $(seq 1 40); do
    xdpyinfo -display :99 >/dev/null 2>&1 && break
    sleep 0.25
done
xdpyinfo -display :99 >/dev/null 2>&1 || { echo "Xvfb never came up" >&2; exit 1; }

say "building the fixture project"
/opt/demo/make-fixture.sh "$FIXTURE"

mkdir -p "$HOME/.config/herdr"
cat > "$HOME/.config/herdr/config.toml" <<TOML
onboarding = false
[terminal]
kitty_graphics = true
TOML

say "launching kitty + herdr"
kitty --config /opt/demo/kitty.conf \
      -o "initial_window_width=${WIDTH}" \
      -o "initial_window_height=${HEIGHT}" \
      --directory "$FIXTURE" \
      -- herdr >/tmp/kitty.log 2>&1 &

# herdr is up once it answers on its socket with a pane to talk to.
pane=""
for _ in $(seq 1 60); do
    pane="$(herdr pane list 2>/dev/null | python3 -c '
import sys, json
try:
    panes = json.load(sys.stdin)["result"]["panes"]
    print(panes[0]["pane_id"] if panes else "")
except Exception:
    print("")
' 2>/dev/null || true)"
    [ -n "$pane" ] && break
    sleep 1
done
[ -n "$pane" ] || { echo "herdr never came up:" >&2; cat /tmp/kitty.log >&2; exit 1; }
say "herdr pane $pane"

say "starting grove"
herdr pane run "$pane" grove >/dev/null 2>&1
sleep 4

# Each scenario is just the arrow keys someone would press, given the fixture's
# layout: assets, clips, docs, src, Cargo.toml, README.md.
case "$SCENARIO" in
    image) keys="right down" ;;                    # assets/cover.png
    video) keys="down right down" ;;               # clips/intro.mp4
    pdf)   keys="down down right down" ;;          # docs/architecture.pdf
    code)  keys="down down down right down" ;;     # src/main.rs
    *) echo "unknown scenario: $SCENARIO (image, video, pdf, code)" >&2; exit 1 ;;
esac
say "walking the tree: $SCENARIO"
# shellcheck disable=SC2086
herdr pane send-keys "$pane" $keys >/dev/null 2>&1
# ffmpeg and pdftoppm take a moment; the picture must be up before the shutter.
sleep 6

say "capturing"
mkdir -p "$(dirname "$OUT")"
import -display :99 -window root -screen "$OUT"
if [ "$CROP_LEFT" -gt 0 ]; then
    "$IM" "$OUT" -crop "$((WIDTH - CROP_LEFT))x${HEIGHT}+${CROP_LEFT}+0" +repage -strip "$OUT"
else
    "$IM" "$OUT" -strip "$OUT"
fi
say "wrote $OUT ($(du -h "$OUT" | cut -f1))"
