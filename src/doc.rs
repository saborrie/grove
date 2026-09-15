//! What the right-hand half shows. Text is highlighted and wrapped to the pane;
//! media becomes a caption under a picture the graphics layer draws; directories
//! get a listing so arrowing over a folder still tells you something.
//!
//! Wrapping happens here rather than in `Paragraph` so that scrolling counts
//! RENDERED rows: a continuation row is reachable like any other.

use std::path::Path;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::media::{self, Kind};
use crate::theme;

/// Read at most this much of a file, and highlight at most this many lines —
/// a preview that has to chew through a 200MB log is a hang, not a preview.
const MAX_BYTES: usize = 512 * 1024;
const MAX_LINES: usize = 5000;

/// A piece of a document, lifted out to be pasted somewhere else.
pub struct Snippet {
    /// The source lines it covers, 1-based and inclusive — `None` for a
    /// document that has no line numbers to speak of, like a directory listing.
    pub lines: Option<(usize, usize)>,
    /// The original text, unwrapped and with no line-number gutter. What you
    /// want in a paste is the file's own bytes, not grove's rendering of them.
    pub text: String,
}

pub struct Doc {
    pub title: String,
    pub info: String,
    /// Set when the picture for this document belongs on the graphics layer.
    pub media: Option<Kind>,
    lines: Vec<Line<'static>>,
    numbered: bool,
    pub scroll: usize,
    rows: Vec<Line<'static>>,
    /// Which source line each rendered row came from. A wrapped line occupies
    /// several rows but is one line, and a reference has to name the line.
    row_lines: Vec<usize>,
    rows_width: Option<u16>,
}

impl Doc {
    fn new(title: String, info: String, lines: Vec<Line<'static>>, numbered: bool) -> Self {
        Self {
            title,
            info,
            media: None,
            lines,
            numbered,
            scroll: 0,
            rows: Vec::new(),
            row_lines: Vec::new(),
            rows_width: None,
        }
    }

    pub fn empty() -> Self {
        Self::new(String::new(), String::new(), Vec::new(), false)
    }

    pub fn load(path: &Path, is_dir: bool) -> Self {
        if is_dir {
            return Self::directory(path);
        }
        match media::classify(path) {
            Kind::Other => Self::text(path),
            kind => Self::media(path, kind),
        }
    }

    fn title_of(path: &Path) -> String {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string())
    }

    /// The caption for something the graphics layer is drawing. The body stays
    /// empty: those cells must not be painted over the picture.
    fn media(path: &Path, kind: Kind) -> Self {
        let mut doc = Self::new(Self::title_of(path), String::new(), Vec::new(), false);
        doc.media = Some(kind);
        doc.info = media::details(path, kind).join("   ");
        doc
    }

    fn directory(path: &Path) -> Self {
        let mut entries: Vec<crate::tree::Entry> = std::fs::read_dir(path)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| crate::tree::Entry {
                        is_dir: e.file_type().map(|t| t.is_dir()).unwrap_or(false),
                        name: e.file_name().to_string_lossy().into_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        crate::tree::sort_entries(&mut entries);
        let (dirs, files) =
            entries.iter().fold(
                (0, 0),
                |(d, f), e| {
                    if e.is_dir { (d + 1, f) } else { (d, f + 1) }
                },
            );
        let lines: Vec<Line<'static>> = entries
            .iter()
            .map(|e| {
                let style = if e.is_dir {
                    Style::default()
                        .fg(theme::chrome())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme::dim())
                };
                Line::from(Span::styled(
                    if e.is_dir {
                        format!("{}/", e.name)
                    } else {
                        e.name.clone()
                    },
                    style,
                ))
            })
            .collect();
        let mut doc = Self::new(Self::title_of(path), String::new(), lines, false);
        doc.info = format!("{dirs} folders   {files} files");
        doc
    }

    fn text(path: &Path) -> Self {
        let name = Self::title_of(path);
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let (lines, numbered) = match std::fs::read(path) {
            Err(e) => (vec![Line::raw(format!("(unreadable: {e})"))], false),
            Ok(bytes) if bytes.contains(&0) => (
                vec![Line::raw(format!("(binary — {})", media::human_size(size)))],
                false,
            ),
            Ok(bytes) => {
                let truncated = bytes.len() > MAX_BYTES;
                let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_BYTES)]);
                let mut lines =
                    crate::syntax::highlight(&name, &text, MAX_LINES).unwrap_or_else(|| {
                        text.lines()
                            .take(MAX_LINES)
                            .map(|l| Line::raw(l.to_string()))
                            .collect()
                    });
                if truncated || text.lines().count() > MAX_LINES {
                    lines.push(Line::styled(
                        "… truncated",
                        Style::default().fg(theme::dim()),
                    ));
                }
                if lines.is_empty() {
                    lines.push(Line::styled(
                        "(empty file)",
                        Style::default().fg(theme::dim()),
                    ));
                }
                (lines, true)
            }
        };
        let mut doc = Self::new(name, String::new(), lines, numbered);
        doc.info = media::human_size(size);
        doc
    }

    /// Rendered rows for a `width`-wide body, rebuilt only when the width moves.
    pub fn rows(&mut self, width: u16) -> &[Line<'static>] {
        if self.rows_width != Some(width) {
            (self.rows, self.row_lines) = build_rows(&self.lines, self.numbered, width);
            self.rows_width = Some(width);
            self.clamp();
        }
        &self.rows
    }

    /// How many rendered rows there are, for turning a pointer into a row.
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// The snippet covered by a range of RENDERED rows — what the mouse lands
    /// on — resolved back to whole source lines, which is what a reference can
    /// name. Selecting half of a wrapped line takes the whole line: a fragment
    /// of a line is not something you can paste and act on.
    pub fn snippet(&self, from_row: usize, to_row: usize) -> Option<Snippet> {
        let (from_row, to_row) = (from_row.min(to_row), from_row.max(to_row));
        let first = *self.row_lines.get(from_row)?;
        let last = *self.row_lines.get(to_row.min(self.row_lines.len() - 1))?;
        let text = self.lines[first..=last]
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        Some(Snippet {
            // Line numbers only mean something where grove is showing them.
            lines: self.numbered.then_some((first + 1, last + 1)),
            text,
        })
    }

    pub fn scroll_by(&mut self, delta: isize) {
        self.scroll = self.scroll.saturating_add_signed(delta);
        self.clamp();
    }

    fn clamp(&mut self) {
        self.scroll = self.scroll.min(self.rows.len().saturating_sub(1));
    }
}

/// Wrap each source line to the body width and prefix a line-number gutter,
/// blank on continuation rows so code stays aligned under its own first row.
///
/// Returns the rows and, alongside them, the source line each row came from —
/// the mapping a selection needs to turn rows back into line numbers.
fn build_rows(
    lines: &[Line<'static>],
    numbered: bool,
    width: u16,
) -> (Vec<Line<'static>>, Vec<usize>) {
    let number_width = lines.len().to_string().len();
    let gutter = if numbered { number_width + 1 } else { 0 };
    let content = usize::from(width).saturating_sub(gutter);
    if content == 0 {
        return (Vec::new(), Vec::new());
    }
    let mut rows = Vec::with_capacity(lines.len());
    let mut row_lines = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        let has_tab = line.spans.iter().any(|s| s.content.contains('\t'));
        let pieces = if line.width() > content || has_tab {
            crate::wrap::wrap_line(line, content)
        } else {
            vec![line.clone()]
        };
        for (piece_index, piece) in pieces.into_iter().enumerate() {
            row_lines.push(index);
            if !numbered {
                rows.push(piece);
                continue;
            }
            let label = if piece_index == 0 {
                format!("{:>number_width$} ", index + 1)
            } else {
                " ".repeat(gutter)
            };
            let mut spans = vec![Span::styled(label, Style::default().fg(theme::dim()))];
            spans.extend(piece.spans);
            rows.push(Line::from(spans));
        }
    }
    (rows, row_lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_wrap_into_reachable_rows() {
        let lines = vec![Line::raw("aaaa bbbb cccc dddd")];
        let (rows, _) = build_rows(&lines, false, 10);
        assert!(rows.len() > 1);
        let joined: String = rows.iter().map(|r| r.to_string()).collect();
        assert_eq!(joined, "aaaa bbbb cccc dddd");
    }

    #[test]
    fn continuation_rows_indent_under_the_gutter() {
        let lines = vec![Line::raw("aaaa bbbb cccc")];
        let (rows, _) = build_rows(&lines, true, 8);
        assert!(rows[0].to_string().starts_with("1 "));
        assert!(
            rows[1].to_string().starts_with("  "),
            "{:?}",
            rows[1].to_string()
        );
    }

    /// Three source lines where the middle one wraps into three rows, so
    /// rendered rows and source lines deliberately disagree.
    fn wrapped_doc() -> Doc {
        let lines = vec![
            Line::raw("one"),
            Line::raw("aaaa bbbb cccc"),
            Line::raw("three"),
        ];
        let mut doc = Doc::new(String::new(), String::new(), lines, true);
        doc.rows(8);
        doc
    }

    #[test]
    fn every_rendered_row_maps_back_to_the_line_it_came_from() {
        let doc = wrapped_doc();
        assert_eq!(doc.row_count(), 5, "1 + 3 continuation rows + 1");
        assert_eq!(doc.row_lines, vec![0, 1, 1, 1, 2]);
    }

    #[test]
    fn selecting_part_of_a_wrapped_line_takes_the_whole_line() {
        let doc = wrapped_doc();
        // Rows 2 and 3 are both the middle of line 2. A reference to "half of
        // line 2" is not something anyone can act on.
        let snippet = doc.snippet(2, 3).unwrap();
        assert_eq!(snippet.lines, Some((2, 2)));
        assert_eq!(snippet.text, "aaaa bbbb cccc");
    }

    #[test]
    fn a_snippet_carries_the_source_text_not_the_rendering() {
        let doc = wrapped_doc();
        let snippet = doc.snippet(0, 4).unwrap();
        assert_eq!(snippet.lines, Some((1, 3)));
        // No gutter, and the wrapped line is whole again: what you paste is the
        // file, not grove's picture of it.
        assert_eq!(snippet.text, "one\naaaa bbbb cccc\nthree");
    }

    #[test]
    fn a_backwards_drag_selects_the_same_lines() {
        let doc = wrapped_doc();
        assert_eq!(
            doc.snippet(4, 0).unwrap().text,
            doc.snippet(0, 4).unwrap().text
        );
    }

    #[test]
    fn a_document_without_line_numbers_cites_none() {
        // A directory listing renders lines but they are not a file's lines,
        // so there is nothing honest to put after the colon.
        let lines = vec![Line::raw("src/"), Line::raw("main.rs")];
        let mut doc = Doc::new(String::new(), String::new(), lines, false);
        doc.rows(40);
        let snippet = doc.snippet(0, 1).unwrap();
        assert_eq!(snippet.lines, None);
        assert_eq!(snippet.text, "src/\nmain.rs");
    }

    #[test]
    fn selecting_past_the_last_row_stops_at_the_end() {
        let doc = wrapped_doc();
        assert_eq!(doc.snippet(0, 99).unwrap().lines, Some((1, 3)));
        assert!(
            doc.snippet(99, 99).is_none(),
            "starting past the end selects nothing"
        );
        let empty = Doc::empty();
        assert!(
            empty.snippet(0, 0).is_none(),
            "and an empty document has nothing"
        );
    }

    #[test]
    fn media_documents_leave_the_body_empty_for_the_picture() {
        let doc = Doc::media(Path::new("/tmp/x.png"), Kind::Image);
        assert_eq!(doc.media, Some(Kind::Image));
        assert!(doc.lines.is_empty());
    }
}
