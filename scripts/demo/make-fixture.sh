#!/bin/sh
# A small project for grove to be photographed browsing. Everything is generated,
# so the screenshot owes nothing to whatever happens to be on the machine.
set -eu
root="${1:?usage: make-fixture.sh DIR}"

# ImageMagick 7 renamed the tool; Debian still ships 6.
IM="$(command -v magick || command -v convert)"

# Annotating without an explicit font aborts outright in a slim image: there is
# no default font to resolve and ImageMagick does not degrade gracefully.
FONT="$(fc-match -f '%{file}' 'JetBrainsMono Nerd Font' 2>/dev/null || true)"
[ -n "$FONT" ] || FONT=/usr/share/fonts/truetype/nerd/JetBrainsMonoNerdFont-Regular.ttf
rm -rf "$root"
mkdir -p "$root/assets" "$root/clips" "$root/docs" "$root/src"

# A picture that reads as a real picture at thumbnail size.
"$IM" -size 1200x800 plasma:fractal -blur 0x6 -modulate 105,140 \
    "$root/assets/cover.png"
"$IM" -size 480x480 radial-gradient:'#89b4fa'-'#1e1e2e' \
    -font "$FONT" -fill '#cdd6f4' -pointsize 64 -gravity center -annotate 0 'grove' \
    "$root/assets/logo.png"

# A video, so the frame-grab path is on show too.
ffmpeg -v error -f lavfi -i testsrc=duration=8:size=960x540:rate=25 \
    -pix_fmt yuv420p "$root/clips/intro.mp4" -y

"$IM" -size 1240x1754 xc:white -font "$FONT" \
    -fill '#1e1e2e' -pointsize 64 -gravity north -annotate +0+180 'Design notes' \
    -pointsize 34 -annotate +0+320 'How the preview pane decides what to draw' \
    "$root/docs/architecture.pdf"

cat > "$root/src/main.rs" <<'RS'
//! grove — a file tree pinned to one root, with live previews.

fn main() -> std::io::Result<()> {
    let root = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or(std::env::current_dir()?);

    let mut app = App::new(root);
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    result
}
RS

cat > "$root/src/preview.rs" <<'RS'
/// Decide from the picture that was LOADED, never from the row the cursor is on.
fn picture_action(stale: bool, thumb: Option<Thumb>, body: Rect) -> Picture {
    if stale {
        return Picture::Leave;
    }
    match thumb {
        None => Picture::Hide,
        Some(thumb) => Picture::Show(centre(body, thumb.cells)),
    }
}
RS

cat > "$root/README.md" <<'MD'
# demo project

A directory that exists to be looked at.
MD

cat > "$root/Cargo.toml" <<'TOML'
[package]
name = "demo"
version = "0.1.0"
edition = "2024"
TOML
