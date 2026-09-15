//! Putting text on the system clipboard from inside a herdr pane, and wrapping
//! a snippet so it still says where it came from once it has been pasted.
//!
//! OSC 52 rather than xclip, wl-copy or pbcopy: the escape sequence goes to the
//! terminal, herdr captures it and forwards it to the outer terminal, which owns
//! the real clipboard. It is the same division of labour as the pictures —
//! herdr is the one with the plumbing — so this needs no DISPLAY, no helper
//! binary and no temp file, and it survives `herdr --remote` and SSH, where a
//! clipboard helper would be talking to the wrong machine's clipboard.

use std::io::{self, Write};

use base64::Engine as _;

/// Terminals cap how much they will accept in one OSC 52, and the failure mode
/// is silence: an oversized sequence is dropped rather than truncated. Better
/// to refuse out loud than to leave someone pasting whatever was there before.
const MAX_BYTES: usize = 64 * 1024;

/// Put `text` on the system clipboard.
pub fn copy(text: &str) -> Result<(), String> {
    if text.len() > MAX_BYTES {
        return Err(format!(
            "too big to copy — {} KB, limit {} KB",
            text.len() / 1024,
            MAX_BYTES / 1024
        ));
    }
    let data = base64::engine::general_purpose::STANDARD.encode(text);
    let mut out = io::stdout().lock();
    // `c` is the clipboard proper, as against `p`, the primary selection.
    write!(out, "\x1b]52;c;{data}\x07").map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())
}

/// A snippet wrapped the way a chat expects it: the reference on its own line
/// above a fenced block, so that pasting it into Claude carries the file and
/// the lines along with the code.
///
/// The path is absolute on purpose. grove's root is often not the directory the
/// agent you are pasting into was started in, and a relative path that resolves
/// against the wrong root is worse than a long one — it points at a file that
/// either does not exist or, worse, is a different file of the same name.
pub fn block(path: &str, lines: Option<(usize, usize)>, language: &str, text: &str) -> String {
    let reference = match lines {
        // `path:line` is the form Claude Code prints and treats as clickable.
        Some((first, last)) if first == last => format!("{path}:{first}"),
        Some((first, last)) => format!("{path}:{first}-{last}"),
        None => path.to_string(),
    };
    let fence = "`".repeat(fence_width(text));
    format!("{reference}\n{fence}{language}\n{text}\n{fence}\n")
}

/// Long enough to survive whatever backticks are inside. A markdown file full
/// of its own code blocks would otherwise close the fence early and the paste
/// would arrive in pieces.
fn fence_width(text: &str) -> usize {
    let mut longest = 0;
    let mut run = 0;
    for ch in text.chars() {
        run = if ch == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    longest.max(2) + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_of_lines_becomes_a_clickable_reference() {
        let out = block("/w/src/tree.rs", Some((142, 158)), "rs", "fn main() {}");
        assert!(out.starts_with("/w/src/tree.rs:142-158\n```rs\n"), "{out}");
        assert!(out.ends_with("fn main() {}\n```\n"), "{out}");
    }

    #[test]
    fn one_line_does_not_pretend_to_be_a_range() {
        let out = block("/w/a.rs", Some((7, 7)), "rs", "let x = 1;");
        assert!(out.starts_with("/w/a.rs:7\n"), "{out}");
    }

    #[test]
    fn a_document_without_line_numbers_still_names_its_path() {
        // A directory listing has no lines to cite, but which directory it was
        // is still the useful half.
        let out = block("/w/src", None, "", "main.rs\ntree.rs");
        assert!(out.starts_with("/w/src\n```\n"), "{out}");
    }

    #[test]
    fn the_fence_outgrows_backticks_in_the_snippet() {
        // Copying out of a markdown file: the snippet's own fence must not end
        // the block early and split the paste in half.
        let markdown = "Text:\n```rust\nfn main() {}\n```\ndone";
        let out = block("/w/README.md", Some((1, 5)), "md", markdown);
        assert!(out.contains("\n````md\n"), "fence must be longer: {out}");
        assert!(out.ends_with("done\n````\n"), "{out}");
        // And a plain snippet keeps the ordinary three.
        assert!(block("/w/a.rs", None, "rs", "x").contains("\n```rs\n"));
    }

    #[test]
    fn fence_width_counts_the_longest_run_not_the_total() {
        assert_eq!(fence_width("no backticks"), 3);
        assert_eq!(
            fence_width("a ` b ` c"),
            3,
            "singles still fit inside three"
        );
        assert_eq!(fence_width("```"), 4);
        assert_eq!(fence_width("````"), 5);
        assert_eq!(fence_width("`` and ```` and ``"), 5);
    }

    #[test]
    fn an_oversized_selection_is_refused_rather_than_silently_dropped() {
        let huge = "x".repeat(MAX_BYTES + 1);
        assert!(copy(&huge).is_err());
    }
}
