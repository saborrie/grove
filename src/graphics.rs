//! herdr's pane graphics API: hand the server a PNG and a cell rectangle and it
//! deals with the kitty protocol, the outer terminal and the SSH bridge. That is
//! the entire reason this tool can show images without linking a graphics stack.
//!
//! Requests are one JSON object per line on the session socket; every request
//! needs an `id` or the server rejects it as malformed.

use std::cell::Cell;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

use base64::Engine as _;
use ratatui::layout::Rect;

const LAYER: &str = "grove-preview";

pub struct Herdr {
    socket: String,
    pane: String,
    seq: Cell<u64>,
    /// Pixel size of one terminal cell, so a thumbnail can be built to fit the
    /// preview rectangle exactly instead of being rescaled by the compositor.
    pub cell: (u32, u32),
}

impl Herdr {
    /// `None` outside a herdr pane — the caller then falls back to a text card.
    pub fn from_env() -> Option<Self> {
        let socket = std::env::var("HERDR_SOCKET_PATH").ok()?;
        let pane = std::env::var("HERDR_PANE_ID").ok()?;
        let herdr = Self { socket, pane, seq: Cell::new(0), cell: (10, 20) };
        let (w, h) = herdr.cell_size()?;
        Some(Self { cell: (w, h), ..herdr })
    }

    fn call(&self, method: &str, params: serde_json::Value) -> Option<serde_json::Value> {
        self.seq.set(self.seq.get() + 1);
        let request = serde_json::json!({
            "id": format!("grove-{}", self.seq.get()),
            "method": method,
            "params": params,
        });
        let mut stream = UnixStream::connect(&self.socket).ok()?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok()?;
        stream.write_all(format!("{request}\n").as_bytes()).ok()?;
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).ok()?;
        let response: serde_json::Value = serde_json::from_str(&line).ok()?;
        response.get("result").cloned()
    }

    fn cell_size(&self) -> Option<(u32, u32)> {
        let info = self.call("pane.graphics.info", serde_json::json!({ "pane_id": self.pane }))?;
        let w = info.get("cell_width_px")?.as_u64()? as u32;
        let h = info.get("cell_height_px")?.as_u64()? as u32;
        (w > 0 && h > 0).then_some((w, h))
    }

    /// Place `png` over `rect`, replacing whatever the layer held before.
    pub fn set(&self, png: &[u8], size: (u32, u32), rect: Rect) {
        let data = base64::engine::general_purpose::STANDARD.encode(png);
        self.call(
            "pane.graphics.set",
            serde_json::json!({
                "pane_id": self.pane,
                "format": "png",
                "image_width": size.0,
                "image_height": size.1,
                "data_base64": data,
                "layer_id": LAYER,
                "z_index": 1,
                "placement": {
                    "grid_cols": rect.width,
                    "grid_rows": rect.height,
                    "viewport_col": rect.x,
                    "viewport_row": rect.y,
                },
            }),
        );
    }

    pub fn clear(&self) {
        self.call(
            "pane.graphics.clear",
            serde_json::json!({ "pane_id": self.pane, "layer_id": LAYER }),
        );
    }
}
