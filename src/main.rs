//! grove — a file tree pinned to one root, with live previews, in a single pane.
//!
//! The tree model, wrapping, icons and syntax highlighting are borrowed from
//! herdr-sidebar (MIT, Alex Arthurs); everything about panes, tabs, source
//! control and editing is deliberately absent. The root is wherever grove was
//! launched and never moves.
//!
//! Keys are the whole surface: arrows and Enter. The mouse wheel scrolls
//! whichever half it is over. Ctrl+C quits.

mod doc;
mod graphics;
mod icons;
mod media;
mod syntax;
mod theme;
mod tree;
mod wrap;

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseButton, MouseEventKind,
};
use crossterm::execute;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::doc::Doc;
use crate::icons::IconTheme;
use crate::media::Thumb;
use crate::tree::{Row, Tree};

/// The tree takes a third of the pane, clamped to something usable at either end.
const TREE_MIN: u16 = 16;
const TREE_MAX: u16 = 46;
const PREVIEW_MIN: u16 = 24;

struct App {
    tree: Tree,
    rows: Vec<Row>,
    icons: IconTheme,
    selected: usize,
    offset: usize,
    doc: Doc,
    /// Body rectangle of the preview half, captured during the draw — the
    /// thumbnail is built to fit it, so it has to be known before loading.
    body: Rect,
    /// Body rectangle of the tree half, captured during the draw, so a click can
    /// be turned back into the row under it.
    tree_body: Rect,
    /// Set when the selection moved and the preview has not caught up yet.
    stale: bool,
    /// Whether the viewport should chase the cursor. The wheel scrolls the tree
    /// away from the selection on purpose, so chasing is off until the arrows
    /// move again — otherwise every scroll snaps straight back.
    follow: bool,
    herdr: Option<graphics::Herdr>,
    /// The loaded picture and the file it came from. The path travels WITH the
    /// picture: the selection has already moved on by the time it is published.
    thumb: Option<(PathBuf, Thumb)>,
    /// The preview body the picture was built for, so a resize rebuilds it at
    /// the new size instead of rescaling it.
    thumb_body: Rect,
    /// What the graphics layer is actually showing, and where.
    placed: Option<(PathBuf, Rect)>,
    trouble: Option<String>,
    quit: bool,
}

impl App {
    fn new(root: PathBuf) -> Self {
        let mut tree = Tree::new(root);
        let rows = tree.rows();
        Self {
            tree,
            rows,
            icons: IconTheme::resolve(std::env::var("GROVE_ICONS").ok().as_deref(), None),
            selected: 0,
            offset: 0,
            doc: Doc::empty(),
            body: Rect::ZERO,
            tree_body: Rect::ZERO,
            stale: true,
            follow: true,
            herdr: graphics::Herdr::from_env(),
            thumb: None,
            thumb_body: Rect::ZERO,
            placed: None,
            trouble: None,
            quit: false,
        }
    }

    fn current(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    /// Rebuild the visible rows after an expand/collapse, keeping the cursor on
    /// the same path rather than the same index.
    fn rebuild(&mut self) {
        let keep = self.current().map(|r| r.path.clone());
        self.rows = self.tree.rows();
        if let Some(path) = keep {
            if let Some(index) = self.rows.iter().position(|r| r.path == path) {
                self.selected = index;
            }
        }
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }

    fn move_by(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() - 1;
        self.selected = self.selected.saturating_add_signed(delta).min(last);
        self.stale = true;
        self.follow = true;
    }

    fn expand(&mut self) {
        let Some(row) = self.current() else { return };
        if row.is_dir && !row.expanded {
            let path = row.path.clone();
            self.tree.expand(&path);
            self.rebuild();
        }
    }

    /// Collapse an open directory, otherwise step out to the parent row — the
    /// motion people expect from a tree even without a binding for it.
    fn collapse(&mut self) {
        let Some(row) = self.current() else { return };
        if row.is_dir && row.expanded {
            let path = row.path.clone();
            self.tree.collapse(&path);
            self.rebuild();
            return;
        }
        let depth = row.depth;
        if depth == 0 {
            return;
        }
        if let Some(parent) = self.rows[..self.selected].iter().rposition(|r| r.depth < depth) {
            self.selected = parent;
            self.stale = true;
            self.follow = true;
        }
    }

    fn enter(&mut self) {
        let Some(row) = self.current() else { return };
        if row.is_dir {
            let path = row.path.clone();
            self.tree.toggle(&path);
            self.rebuild();
            self.stale = true;
        } else {
            // A file is already previewed by arrowing onto it; Enter re-reads it
            // from disk, which is what makes it useful while an agent is writing.
            self.tree.refresh();
            self.rebuild();
            self.stale = true;
        }
    }

    fn load(&mut self) {
        self.stale = false;
        self.thumb = None;
        self.trouble = None;
        let Some(row) = self.current().cloned() else {
            self.doc = Doc::empty();
            return;
        };
        self.doc = Doc::load(&row.path, row.is_dir);
        let Some(kind) = self.doc.media else { return };
        // The picture is sized in pixels, so the body has to have been drawn once.
        if self.body.width == 0 || self.body.height == 0 {
            self.stale = true;
            return;
        }
        let Some(herdr) = &self.herdr else {
            self.trouble = Some("no picture outside herdr — run grove in a herdr pane".into());
            return;
        };
        match media::thumbnail(&row.path, kind, (self.body.width, self.body.height), herdr.cell) {
            Ok(thumb) => {
                self.doc.info = format!("{} × {}   {}", thumb.source.0, thumb.source.1, self.doc.info);
                self.thumb = Some((row.path.clone(), thumb));
                self.thumb_body = self.body;
            }
            Err(why) => self.trouble = Some(why),
        }
    }

    /// Put the picture on the pane, or take it off. Every set is a base64 round
    /// trip, so this only talks to herdr when something actually changed.
    fn sync_picture(&mut self) {
        let Some(herdr) = &self.herdr else { return };
        // A resize means the picture was built for a rectangle that no longer
        // exists; rebuild it rather than let the compositor rescale it.
        if self.thumb.is_some() && self.thumb_body != self.body {
            self.stale = true;
        }
        let action = picture_action(
            self.stale,
            self.thumb.as_ref().map(|(path, thumb)| (path.as_path(), thumb.cells)),
            self.body,
            self.placed.as_ref().map(|(path, rect)| (path.as_path(), *rect)),
        );
        match action {
            Picture::Leave => {}
            Picture::Hide => {
                herdr.clear();
                self.placed = None;
            }
            Picture::Show(rect) => {
                if let Some((path, thumb)) = &self.thumb {
                    herdr.set(&thumb.png, thumb.size, rect);
                    self.placed = Some((path.clone(), rect));
                }
            }
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let tree_width = (area.width / 3)
            .clamp(TREE_MIN, TREE_MAX)
            .min(area.width.saturating_sub(PREVIEW_MIN))
            .max(1);
        let columns =
            Layout::horizontal([Constraint::Length(tree_width), Constraint::Length(1), Constraint::Min(0)])
                .split(area);
        self.draw_tree(frame, columns[0]);
        frame.render_widget(
            Paragraph::new(vec![Line::raw("│"); usize::from(area.height)])
                .style(Style::default().fg(theme::rule())),
            columns[1],
        );
        self.draw_preview(frame, columns[2]);
    }

    fn draw_tree(&mut self, frame: &mut Frame, area: Rect) {
        if area.height == 0 {
            return;
        }
        let header = Line::from(Span::styled(
            format!(" {}", self.tree.root_name()),
            Style::default().fg(theme::chrome()).add_modifier(Modifier::BOLD),
        ));
        frame.render_widget(Paragraph::new(header), Rect { height: 1, ..area });

        let body = Rect { y: area.y + 1, height: area.height - 1, ..area };
        self.tree_body = body;
        let height = usize::from(body.height);
        if self.follow {
            // Keep the cursor on screen — but only when the cursor is what moved.
            if self.selected < self.offset {
                self.offset = self.selected;
            } else if height > 0 && self.selected >= self.offset + height {
                self.offset = self.selected + 1 - height;
            }
        }
        // Never scroll past the last row, however the offset got there.
        self.offset = self.offset.min(self.rows.len().saturating_sub(height.max(1)));

        let lines: Vec<Line> = self
            .rows
            .iter()
            .enumerate()
            .skip(self.offset)
            .take(height)
            .map(|(index, row)| tree_line(row, self.icons, index == self.selected, body.width))
            .collect();
        frame.render_widget(Paragraph::new(lines), body);
    }

    fn draw_preview(&mut self, frame: &mut Frame, area: Rect) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let mut header = vec![Span::styled(
            format!(" {}", self.doc.title),
            Style::default().fg(theme::chrome()).add_modifier(Modifier::BOLD),
        )];
        if !self.doc.info.is_empty() {
            header.push(Span::styled(
                format!("   {}", self.doc.info),
                Style::default().fg(theme::dim()),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(header)), Rect { height: 1, ..area });

        let body = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width.saturating_sub(1),
            height: area.height - 1,
        };
        self.body = body;
        if let Some(trouble) = &self.trouble {
            frame.render_widget(
                Paragraph::new(Line::styled(
                    format!("({trouble})"),
                    Style::default().fg(theme::dim()),
                )),
                body,
            );
            return;
        }
        // A picture's cells stay empty: herdr composites the image over them.
        if self.doc.media.is_some() {
            return;
        }
        let scroll = self.doc.scroll;
        let rows = self.doc.rows(body.width);
        let visible: Vec<Line> = rows.iter().skip(scroll).take(usize::from(body.height)).cloned().collect();
        frame.render_widget(Paragraph::new(visible), body);
    }

    /// A click in the tree selects the row under the pointer; on a folder it
    /// toggles, which is what the chevron looks like it promises. Clicks in the
    /// preview do nothing — there is nothing there to activate.
    fn click(&mut self, column: u16, row: u16) {
        let Some(index) = row_at(self.tree_body, self.offset, self.rows.len(), column, row) else {
            return;
        };
        let hit = self.rows[index].clone();
        self.selected = index;
        self.stale = true;
        self.follow = true;
        if hit.is_dir {
            let path = hit.path.clone();
            self.tree.toggle(&path);
            self.rebuild();
        }
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(key.code, KeyCode::Char('c'))
                {
                    self.quit = true;
                    return;
                }
                match key.code {
                    KeyCode::Up => self.move_by(-1),
                    KeyCode::Down => self.move_by(1),
                    KeyCode::Right => self.expand(),
                    KeyCode::Left => self.collapse(),
                    KeyCode::Enter => self.enter(),
                    _ => {}
                }
            }
            Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) => {
                self.click(mouse.column, mouse.row);
            }
            Event::Mouse(mouse) => {
                let delta = match mouse.kind {
                    MouseEventKind::ScrollUp => -3,
                    MouseEventKind::ScrollDown => 3,
                    _ => return,
                };
                if mouse.column >= self.body.x {
                    self.doc.scroll_by(delta);
                } else {
                    // The tree scrolls under the cursor; the selection stays put.
                    self.offset = self.offset.saturating_add_signed(delta);
                    self.follow = false;
                }
            }
            _ => {}
        }
    }
}

/// What the graphics layer should do this frame.
#[derive(Debug, PartialEq, Eq)]
enum Picture {
    /// Leave the pane exactly as it is.
    Leave,
    /// Put the loaded picture on this rectangle.
    Show(Rect),
    /// Take whatever is on the pane off it.
    Hide,
}

/// Decide from the picture that was LOADED, never from the row the cursor is on.
///
/// Between an arrow press and the load that follows it, the two disagree: the
/// document, its thumbnail and its caption still belong to the previous row.
/// Publishing then would put the old picture on the pane under the new row's
/// name — and the next frame, believing itself up to date, would never correct
/// it. That is the "preview does not update until you move again" bug.
fn picture_action(
    stale: bool,
    thumb: Option<(&Path, (u16, u16))>,
    body: Rect,
    placed: Option<(&Path, Rect)>,
) -> Picture {
    if stale {
        return Picture::Leave;
    }
    match thumb {
        None if placed.is_some() => Picture::Hide,
        None => Picture::Leave,
        Some((path, cells)) => {
            let rect = centre(body, cells);
            match placed {
                Some((shown, at)) if shown == path && at == rect => Picture::Leave,
                _ => Picture::Show(rect),
            }
        }
    }
}

/// The row under a click, or `None` when the pointer is outside the tree body or
/// past its last row.
fn row_at(body: Rect, offset: usize, count: usize, column: u16, row: u16) -> Option<usize> {
    let inside = column >= body.x
        && column < body.x.saturating_add(body.width)
        && row >= body.y
        && row < body.y.saturating_add(body.height);
    if !inside {
        return None;
    }
    let index = offset + usize::from(row - body.y);
    (index < count).then_some(index)
}

/// Centre a `cells`-sized picture inside `body`. The rectangle has the canvas's
/// own shape, so the compositor scales it one to one rather than stretching it.
fn centre(body: Rect, cells: (u16, u16)) -> Rect {
    Rect {
        x: body.x + body.width.saturating_sub(cells.0) / 2,
        y: body.y + body.height.saturating_sub(cells.1) / 2,
        width: cells.0.min(body.width),
        height: cells.1.min(body.height),
    }
}

/// One row: indent, chevron, icon, name — VS Code's shape, nothing else.
fn tree_line(row: &Row, icons: IconTheme, selected: bool, width: u16) -> Line<'static> {
    let icon = icons::icon(icons, &row.name, row.is_dir, row.expanded);
    let chevron = if row.is_dir {
        if row.expanded { "▾ " } else { "▸ " }
    } else {
        "  "
    };
    let mut spans = vec![
        Span::raw(" ".repeat(row.depth * 2 + 1)),
        Span::styled(chevron, Style::default().fg(theme::dim())),
    ];
    let glyph = Style::default();
    spans.push(Span::styled(
        format!("{} ", icon.glyph),
        match icon.rgb {
            Some((r, g, b)) => glyph.fg(ratatui::style::Color::Rgb(r, g, b)),
            None => glyph,
        },
    ));
    spans.push(Span::styled(
        row.name.clone(),
        Style::default().fg(theme::chrome()),
    ));
    let mut line = Line::from(spans);
    if selected {
        // Pad to the full width so the highlight reads as a row, not a label.
        let used = line.width();
        if used < usize::from(width) {
            line.spans.push(Span::raw(" ".repeat(usize::from(width) - used)));
        }
        line.style = Style::default().bg(theme::selection_bg());
    }
    line
}

fn main() -> io::Result<()> {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let root = root.canonicalize().unwrap_or(root);
    let mut app = App::new(root);

    let mut terminal = ratatui::init();
    execute!(io::stdout(), EnableMouseCapture)?;

    let result = run(&mut terminal, &mut app);

    if let Some(herdr) = &app.herdr {
        herdr.clear();
    }
    execute!(io::stdout(), DisableMouseCapture)?;
    ratatui::restore();
    result
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> io::Result<()> {
    while !app.quit {
        terminal.draw(|frame| app.draw(frame))?;
        app.sync_picture();
        // Load only once the arrow keys settle: holding Down through a folder of
        // videos must not run ffmpeg for every row it passes over.
        if app.stale && !event::poll(Duration::ZERO)? {
            app.load();
            continue;
        }
        if event::poll(Duration::from_millis(200))? {
            let event = event::read()?;
            app.handle(event);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_map_to_the_row_under_the_pointer() {
        assert_eq!(row_at(BODY, 0, 10, 3, 1), Some(0));
        assert_eq!(row_at(BODY, 0, 10, 3, 5), Some(4));
        // Scrolled: the same cell is a different row.
        assert_eq!(row_at(BODY, 7, 20, 3, 1), Some(7));
    }

    #[test]
    fn clicks_outside_the_tree_or_past_the_last_row_select_nothing() {
        assert_eq!(row_at(BODY, 0, 10, 25, 2), None, "in the preview half");
        assert_eq!(row_at(BODY, 0, 10, 3, 0), None, "on the header");
        assert_eq!(row_at(BODY, 0, 10, 3, 9), None, "below the body");
        assert_eq!(row_at(BODY, 0, 2, 3, 4), None, "past the last row");
    }

    const BODY: Rect = Rect { x: 0, y: 1, width: 20, height: 5 };
    const PANE: Rect = Rect { x: 20, y: 1, width: 40, height: 20 };

    /// The regression: arrowing from one image straight to another used to
    /// publish the FIRST image under the second one's name, and then sit there.
    #[test]
    fn a_stale_preview_never_publishes_the_previous_picture() {
        let old = Path::new("/pics/badge.png");
        let new = Path::new("/pics/gradient.png");
        // Cursor has moved to `new`; the loaded thumbnail is still `old`.
        assert_eq!(
            picture_action(true, Some((old, (13, 6))), PANE, Some((old, centre(PANE, (13, 6))))),
            Picture::Leave,
        );
        // Once the load catches up, the new picture goes up on its own rectangle.
        let action = picture_action(false, Some((new, (40, 12))), PANE, Some((old, centre(PANE, (13, 6)))));
        assert_eq!(action, Picture::Show(centre(PANE, (40, 12))));
    }

    #[test]
    fn an_unchanged_picture_is_not_resent() {
        let path = Path::new("/pics/badge.png");
        let rect = centre(PANE, (13, 6));
        assert_eq!(picture_action(false, Some((path, (13, 6))), PANE, Some((path, rect))), Picture::Leave);
    }

    #[test]
    fn a_moved_rectangle_republishes_the_same_picture() {
        let path = Path::new("/pics/badge.png");
        let stale_rect = Rect { x: 99, y: 99, width: 1, height: 1 };
        assert_eq!(
            picture_action(false, Some((path, (13, 6))), PANE, Some((path, stale_rect))),
            Picture::Show(centre(PANE, (13, 6))),
        );
    }

    #[test]
    fn moving_onto_a_text_file_takes_the_picture_down() {
        let path = Path::new("/pics/badge.png");
        assert_eq!(picture_action(false, None, PANE, Some((path, centre(PANE, (13, 6))))), Picture::Hide);
        assert_eq!(picture_action(false, None, PANE, None), Picture::Leave);
    }

    #[test]
    fn a_picture_is_centred_without_ever_leaving_the_body() {
        let body = Rect { x: 10, y: 2, width: 40, height: 20 };
        let placed = centre(body, (40, 12));
        assert_eq!(placed, Rect { x: 10, y: 6, width: 40, height: 12 });
        // A picture that would overflow is clipped to the body, not pushed out.
        let big = centre(body, (60, 30));
        assert!(big.x >= body.x && big.y >= body.y);
        assert!(big.width <= body.width && big.height <= body.height);
    }
}
