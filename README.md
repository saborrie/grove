# grove

**A file tree pinned to one root, with live previews, in a single herdr pane.**

```
┌─ grove ───────┬─ gradient.png   640 × 400   image · 3.9 KB ─┐
│  ▸ clips      │                                             │
│  ▾ pictures   │                                             │
│      badge.png│              [ the picture ]                │
│      gradient…│                                             │
│  ▸ src        │                                             │
│    notes.pdf  │                                             │
│    README.md  │                                             │
└───────────────┴─────────────────────────────────────────────┘
   ↑ ↓ ← → Enter                    click · wheel
```

Yazi's previews and a VS Code tree, and nothing else at all.

> **grove is built for [herdr](https://herdr.dev).** Pictures are drawn through
> herdr's pane graphics API, so herdr is what makes the previews possible — and
> the previews are the point. Run grove in a herdr pane. Outside one it still
> starts, and you get the tree, the keys and text previews, but every image,
> video and PDF degrades to a text card.

## Why "grove"?

A grove is a stand of trees with no undergrowth — small enough to know, open
enough to see all the way through. That is the whole design brief: **one** tree,
rooted where you left it, with a clear view of whatever you are standing in front
of. No tabs, no panels, no modes, no undergrowth.

## What it is

Terminal file managers make you choose. Yazi has the best previews of anything in
a terminal, but its Miller columns mean the view slides sideways as you walk and
there is no tree — a shape its maintainers have declined more than once. Tree-first
tools like broot re-root when you press Enter and trim the tree to fit rather than
letting you fold it yourself. VS Code's Explorer has exactly the right shape, and
lives inside an editor.

grove is the small intersection: the tree shape, one fixed root, and real pictures.

- **Made for a herdr pane.** No plugin manifest, no install step, no hooks — it is
  a plain binary you run in whichever pane you happen to be in, and it stays in
  that one pane.
- **The root never moves.** It is wherever you launched grove (`grove [path]`), not
  wherever a neighbouring shell wandered off to. Point it at a directory full of
  repos and worktrees and it stays pointed there.
- **One pane.** The tree and the preview live together. Nothing ever opens a tab,
  splits a pane, or steals your focus.
- **Previews that earn the pane** — syntax-highlighted text with line numbers,
  images, video, the first page of a PDF, and a listing for directories. The
  preview follows the cursor, so walking the tree *is* browsing.
- **Arrows and Enter.** That is the entire keyboard surface, on purpose.

## Keys and mouse

| | |
| --- | --- |
| `↑` `↓` | Move |
| `→` | Expand a folder |
| `←` | Collapse it, or step out to the parent |
| `Enter` | Toggle a folder — or re-read the selected file from disk, which is what you want while an agent is writing to it |
| `Ctrl+C` | Quit |
| Click | Select a row; on a folder, fold it |
| Wheel | Scroll whichever half the pointer is over — the tree stays where you put it until the arrows move again |

There is nothing else to learn, and nothing else to accidentally press.

## Pictures

**This is the part that requires herdr.** Images go through herdr's pane graphics
API (`pane.graphics.set`), so herdr owns the kitty protocol, the outer terminal
and the SSH bridge — grove just hands over a PNG and a cell rectangle. That is why
grove needs no graphics stack of its own, no terminal capability detection and no
temp files, and why previews work unchanged over `herdr --remote` and on mobile
clients.

It also means grove draws no pictures anywhere else. Outside a herdr pane the tree,
the keys and text previews all work, and media files show a text card explaining
why — useful enough to debug with, but not what grove is for.

The canvas is padded to a whole number of cells and placed on a rectangle of
exactly that shape, so pictures keep their aspect ratio instead of stretching to
fill the pane, and are never upscaled past their own resolution.

| Kind | Needs | Notes |
| --- | --- | --- |
| Images | — | png, jpeg, gif, webp, bmp, tiff built in |
| Images (avif, heic, jxl…) | `ffmpeg` | Used automatically when the built-in decoders decline |
| Video | `ffmpeg`, `ffprobe` | A frame from 15% in, because first frames are usually black, plus duration and codec |
| PDF | `pdftoppm`, `pdfinfo` (poppler-utils) | First page, and the page count |
| Audio | `ffprobe` | Duration and codec |

Each is optional. Without one, that file type shows the reason instead of a picture.

## Install

Requires [herdr](https://herdr.dev) (0.8 or newer, with pane graphics left on —
they are on by default) and a Rust toolchain to build.

```sh
git clone https://github.com/saborrie/grove
cd grove
cargo build --release
install -m755 target/release/grove ~/.local/bin/
```

Then run it in any herdr pane:

```sh
grove            # rooted here
grove ~/work     # rooted there
```

Split a pane for it and leave it there — that is the intended shape:

```sh
herdr pane split --current --direction right --ratio 0.5
```

## Configuration

There is none worth the name, deliberately:

| Variable | Effect |
| --- | --- |
| `GROVE_THEME=light` | Light palette and a light syntax theme |
| `GROVE_ICONS=emoji` / `material` | Force an icon set instead of probing for a Nerd Font |

## Where the code came from

grove began as a strip-down of [herdr-sidebar](https://github.com/alexarthurs/herdr-sidebar)
by Alex Arthurs, and four files are taken from it essentially unchanged:

| File | What it does |
| --- | --- |
| `src/tree.rs` | The tree model — expansion state, cached listings, VS Code ordering |
| `src/wrap.rs` | Width-aware wrapping of styled lines, so continuations stay scrollable |
| `src/icons.rs` | The file-type icon set, in Nerd Font and emoji themes |
| `src/syntax.rs` | syntect highlighting with bat's extended grammars |

They arrived with their tests, which still run here. What did **not** come across is
most of it: the source control panel, activity bar, editor, quick-open, settings
overlay, font installer, folder-following and all the pane and tab orchestration.
grove adds the single pane layout, the preview document model, and the media
previews.

If you want a file tree *plus* source control, staging, diffs, AI commit messages
and an editor, docked as a real sidebar — use herdr-sidebar. It is the more capable
tool. grove is the one that does one thing.

## License

MIT — see [LICENSE](LICENSE), which carries both copyright lines.
