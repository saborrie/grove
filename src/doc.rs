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

pub struct Doc {
    pub title: String,
    pub info: String,
    /// Set when the picture for this document belongs on the graphics layer.
    pub media: Option<Kind>,
    lines: Vec<Line<'static>>,
    numbered: bool,
    pub scroll: usize,
    rows: Vec<Line<'static>>,
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
            self.rows = build_rows(&self.lines, self.numbered, width);
            self.rows_width = Some(width);
            self.clamp();
        }
        &self.rows
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
fn build_rows(lines: &[Line<'static>], numbered: bool, width: u16) -> Vec<Line<'static>> {
    let number_width = lines.len().to_string().len();
    let gutter = if numbered { number_width + 1 } else { 0 };
    let content = usize::from(width).saturating_sub(gutter);
    if content == 0 {
        return Vec::new();
    }
    let mut rows = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        let has_tab = line.spans.iter().any(|s| s.content.contains('\t'));
        let pieces = if line.width() > content || has_tab {
            crate::wrap::wrap_line(line, content)
        } else {
            vec![line.clone()]
        };
        for (piece_index, piece) in pieces.into_iter().enumerate() {
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
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_wrap_into_reachable_rows() {
        let lines = vec![Line::raw("aaaa bbbb cccc dddd")];
        let rows = build_rows(&lines, false, 10);
        assert!(rows.len() > 1);
        let joined: String = rows.iter().map(|r| r.to_string()).collect();
        assert_eq!(joined, "aaaa bbbb cccc dddd");
    }

    #[test]
    fn continuation_rows_indent_under_the_gutter() {
        let lines = vec![Line::raw("aaaa bbbb cccc")];
        let rows = build_rows(&lines, true, 8);
        assert!(rows[0].to_string().starts_with("1 "));
        assert!(
            rows[1].to_string().starts_with("  "),
            "{:?}",
            rows[1].to_string()
        );
    }

    #[test]
    fn media_documents_leave_the_body_empty_for_the_picture() {
        let doc = Doc::media(Path::new("/tmp/x.png"), Kind::Image);
        assert_eq!(doc.media, Some(Kind::Image));
        assert!(doc.lines.is_empty());
    }
}
