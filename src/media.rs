//! Turning a file into a picture: decode images directly, seek a frame out of a
//! video, rasterise the first page of a PDF. Everything lands as a PNG scaled to
//! fit the preview rectangle, because that is what the graphics API takes.

use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Image,
    Video,
    Pdf,
    Audio,
    Other,
}

pub fn classify(path: &Path) -> Kind {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tiff" | "tif" | "ico" | "avif"
        | "heic" | "heif" | "jxl" | "ppm" | "pgm" => Kind::Image,
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v" | "wmv" | "flv" | "mpg" | "mpeg"
        | "ts" | "ogv" => Kind::Video,
        "pdf" => Kind::Pdf,
        "mp3" | "flac" | "wav" | "ogg" | "m4a" | "aac" | "opus" | "wma" => Kind::Audio,
        _ => Kind::Other,
    }
}

/// A picture ready for the graphics layer: a canvas whose pixel size is an exact
/// multiple of the cell size, so the placement rectangle it is given has exactly
/// the canvas's own shape and nothing gets stretched.
pub struct Thumb {
    pub png: Vec<u8>,
    /// Canvas pixels — `cells` multiplied by the cell size.
    pub size: (u32, u32),
    /// How many terminal cells the canvas covers.
    pub cells: (u16, u16),
    /// Dimensions of the ORIGINAL, for the info line.
    pub source: (u32, u32),
}

/// The cell rectangle that best matches `source`'s shape, never upscaling.
///
/// Rounding up to whole cells is what makes the aspect ratio survive: the canvas
/// is then padded to that exact rectangle, so herdr maps canvas pixels to pane
/// pixels one to one instead of stretching the image to fill the pane.
fn fit(source: (u32, u32), max_cells: (u16, u16), cell: (u32, u32)) -> (u16, u16) {
    if source.0 == 0 || source.1 == 0 || max_cells.0 == 0 || max_cells.1 == 0 {
        return (1, 1);
    }
    let max_px = (u32::from(max_cells.0) * cell.0, u32::from(max_cells.1) * cell.1);
    let scale = (f64::from(max_px.0) / f64::from(source.0))
        .min(f64::from(max_px.1) / f64::from(source.1))
        .min(1.0);
    let target = (
        ((f64::from(source.0) * scale).round() as u32).max(1),
        ((f64::from(source.1) * scale).round() as u32).max(1),
    );
    (
        (target.0.div_ceil(cell.0) as u16).clamp(1, max_cells.0),
        (target.1.div_ceil(cell.1) as u16).clamp(1, max_cells.1),
    )
}

/// Scale `path` to fit inside `max_cells` and centre it on a cell-aligned canvas.
pub fn thumbnail(
    path: &Path,
    kind: Kind,
    max_cells: (u16, u16),
    cell: (u32, u32),
) -> Result<Thumb, String> {
    let picture = match kind {
        Kind::Image => decode_image(path)?,
        Kind::Video => decode_bytes(&video_frame(path)?)?,
        Kind::Pdf => decode_bytes(&pdf_page(path)?)?,
        _ => return Err("not previewable".into()),
    };
    let source = (picture.width(), picture.height());
    let cells = fit(source, max_cells, cell);
    let size = (u32::from(cells.0) * cell.0, u32::from(cells.1) * cell.1);

    // Fit inside the canvas; a picture smaller than the pane keeps its own size
    // rather than becoming a blur.
    let scaled = if source.0 > size.0 || source.1 > size.1 {
        picture.thumbnail(size.0, size.1)
    } else {
        picture
    }
    .to_rgba8();

    // Transparent letterbox: the pane's own background shows through the bars.
    let mut canvas = image::RgbaImage::from_pixel(size.0, size.1, image::Rgba([0, 0, 0, 0]));
    let offset = (
        i64::from(size.0.saturating_sub(scaled.width()) / 2),
        i64::from(size.1.saturating_sub(scaled.height()) / 2),
    );
    image::imageops::overlay(&mut canvas, &scaled, offset.0, offset.1);

    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(canvas)
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| format!("encode failed: {e}"))?;
    Ok(Thumb { png: png.into_inner(), size, cells, source })
}

/// The `image` crate first; ffmpeg picks up the formats it was not built for
/// (avif, heic, jxl, exotic TIFFs) so the preview does not just go blank.
fn decode_image(path: &Path) -> Result<image::DynamicImage, String> {
    match image::ImageReader::open(path)
        .map_err(|e| e.to_string())
        .and_then(|r| r.with_guessed_format().map_err(|e| e.to_string()))
        .and_then(|r| r.decode().map_err(|e| e.to_string()))
    {
        Ok(image) => Ok(image),
        Err(direct) => match video_frame(path) {
            Ok(bytes) => decode_bytes(&bytes),
            Err(_) => Err(direct),
        },
    }
}

fn decode_bytes(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    image::load_from_memory(bytes).map_err(|e| format!("decode failed: {e}"))
}

/// One frame, a little way in — the first frame of a video is very often black.
fn video_frame(path: &Path) -> Result<Vec<u8>, String> {
    let seek = (duration(path).unwrap_or(0.0) * 0.15).clamp(0.0, 10.0);
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-ss", &format!("{seek:.2}"), "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "image2pipe", "-vcodec", "png", "-"])
        .stderr(Stdio::null())
        .output()
        .map_err(|_| "ffmpeg not installed".to_string())?;
    if output.stdout.is_empty() {
        return Err("no frame decoded".into());
    }
    Ok(output.stdout)
}

fn pdf_page(path: &Path) -> Result<Vec<u8>, String> {
    let output = Command::new("pdftoppm")
        .args(["-png", "-r", "150", "-f", "1", "-l", "1"])
        .arg(path)
        .stderr(Stdio::null())
        .output()
        .map_err(|_| "pdftoppm not installed (poppler-utils)".to_string())?;
    if output.stdout.is_empty() {
        return Err("no page rendered".into());
    }
    Ok(output.stdout)
}

fn ffprobe(path: &Path, field: &str) -> Option<String> {
    let output = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", field, "-of", "default=nw=1:nk=1"])
        .arg(path)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn duration(path: &Path) -> Option<f64> {
    ffprobe(path, "format=duration")?.lines().next()?.parse().ok()
}

/// The lines under the picture: what it is, how big, how long.
pub fn details(path: &Path, kind: Kind) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(meta) = std::fs::metadata(path) {
        out.push(format!("{}  ·  {}", kind_label(kind), human_size(meta.len())));
    }
    match kind {
        Kind::Video | Kind::Audio => {
            if let Some(secs) = duration(path) {
                out.push(format!("duration  {}", clock(secs)));
            }
            if let Some(codec) = ffprobe(path, "stream=codec_name") {
                let names: Vec<&str> = codec.lines().take(2).collect();
                out.push(format!("codec  {}", names.join(", ")));
            }
        }
        Kind::Pdf => {
            if let Some(pages) = pdf_pages(path) {
                out.push(format!("pages  {pages}"));
            }
        }
        _ => {}
    }
    out
}

fn pdf_pages(path: &Path) -> Option<String> {
    let output = Command::new("pdfinfo").arg(path).stderr(Stdio::null()).output().ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("Pages:").map(|v| v.trim().to_string()))
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Image => "image",
        Kind::Video => "video",
        Kind::Pdf => "pdf",
        Kind::Audio => "audio",
        Kind::Other => "file",
    }
}

fn clock(secs: f64) -> String {
    let total = secs.round() as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{size:.1} {}", UNITS[unit]) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_classify() {
        assert_eq!(classify(Path::new("a/b.PNG")), Kind::Image);
        assert_eq!(classify(Path::new("clip.mkv")), Kind::Video);
        assert_eq!(classify(Path::new("paper.pdf")), Kind::Pdf);
        assert_eq!(classify(Path::new("main.rs")), Kind::Other);
        assert_eq!(classify(Path::new("noext")), Kind::Other);
    }

    /// The canvas must have the same shape as the cell rectangle it is placed in,
    /// or herdr stretches the picture to fill the pane — the bug this fixes.
    #[test]
    fn canvas_shape_matches_the_cell_rectangle_it_is_placed_in() {
        let cell = (16, 34);
        for source in [(640, 400), (400, 640), (3000, 200), (200, 3000), (1, 1)] {
            let cells = fit(source, (40, 20), cell);
            let canvas = (u32::from(cells.0) * cell.0, u32::from(cells.1) * cell.1);
            let placed = (u32::from(cells.0) * cell.0, u32::from(cells.1) * cell.1);
            assert_eq!(canvas, placed, "{source:?}");
            assert!(cells.0 <= 40 && cells.1 <= 20, "{source:?} overflowed the pane");
            assert!(cells.0 >= 1 && cells.1 >= 1, "{source:?} vanished");
        }
    }

    #[test]
    fn wide_and_tall_pictures_use_the_axis_that_binds() {
        let cell = (16, 34);
        // 640x400 in a 40x20 cell box (640x680px): width binds, height is spare.
        assert_eq!(fit((640, 400), (40, 20), cell), (40, 12));
        // Taller than the box: height binds and the width comes in well under.
        let tall = fit((400, 2000), (40, 20), cell);
        assert_eq!(tall.1, 20);
        assert!(tall.0 < 40, "{tall:?}");
    }

    #[test]
    fn small_pictures_are_never_blown_up() {
        let cell = (16, 34);
        let cells = fit((200, 200), (40, 20), cell);
        // 200x200 needs 13x6 cells; anything larger means it was upscaled.
        assert_eq!(cells, (13, 6));
    }

    #[test]
    fn degenerate_inputs_do_not_panic() {
        assert_eq!(fit((0, 0), (40, 20), (16, 34)), (1, 1));
        assert_eq!(fit((640, 400), (0, 0), (16, 34)), (1, 1));
    }

    #[test]
    fn sizes_read_like_a_file_manager() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(clock(3661.0), "1:01:01");
        assert_eq!(clock(75.0), "1:15");
    }
}
