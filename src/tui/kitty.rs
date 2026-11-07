//! Screenshots through the kitty graphics protocol
//! (https://sw.kovidgoyal.net/kitty/graphics-protocol/).
//!
//! ratatui only knows about text cells, so the image is drawn out of band:
//! the UI clears the popup area and reports where it is, and after each frame
//! [`KittyImage::sync`] places (or removes) the image there. It only talks to
//! the terminal when something changed.

use haccerlink::calc::Screenshot;
use ratatui::layout::Rect;
use std::io::Write;

/// Image id we use for the screenshot; any id works as long as it's ours.
const IMAGE_ID: u32 = 31;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum ImagePref {
    /// Kitty graphics when the terminal looks like it supports them.
    Auto,
    Kitty,
    /// Unicode half blocks (works everywhere with truecolor).
    Blocks,
}

/// Whether to use the kitty protocol. `auto` goes by what the terminal
/// advertises: kitty, WezTerm and Ghostty implement the protocol. Inside tmux
/// the escapes are swallowed unless passthrough is set up, so `auto` says no.
pub fn use_kitty(pref: ImagePref) -> bool {
    match pref {
        ImagePref::Kitty => true,
        ImagePref::Blocks => false,
        ImagePref::Auto => {
            let var = |k: &str| std::env::var(k).unwrap_or_default();
            if !var("TMUX").is_empty() || var("TERM").starts_with("screen") {
                return false;
            }
            var("TERM") == "xterm-kitty"
                || !var("KITTY_WINDOW_ID").is_empty()
                || !var("GHOSTTY_RESOURCES_DIR").is_empty()
                || matches!(var("TERM_PROGRAM").as_str(), "WezTerm" | "ghostty")
        }
    }
}

/// Where and how big to draw, decided from the popup area and cell size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    /// Top-left cell.
    pub x: u16,
    pub y: u16,
    /// Integer upscale applied before sending (pixel-exact when >= 1).
    pub scale: u32,
    /// Explicit size in cells when the image must be shrunk by the terminal.
    pub cells: Option<(u16, u16)>,
}

/// Fit a `w`x`h` image into `area`, given the cell size in pixels if known.
pub fn place(area: Rect, w: u32, h: u32, cell_px: Option<(u32, u32)>) -> Option<Placement> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let (cw, ch) = cell_px.unwrap_or((0, 0));
    if cw > 0 && ch > 0 {
        let (aw, ah) = (area.width as u32 * cw, area.height as u32 * ch);
        let k = (aw / w).min(ah / h);
        if k >= 1 {
            let cols = (w * k).div_ceil(cw) as u16;
            let rows = (h * k).div_ceil(ch) as u16;
            return Some(Placement {
                x: area.x + (area.width - cols.min(area.width)) / 2,
                y: area.y + (area.height - rows.min(area.height)) / 2,
                scale: k,
                cells: None,
            });
        }
        // Smaller than 1:1: let the terminal shrink it, keeping the aspect.
        let s = (aw as f64 / w as f64).min(ah as f64 / h as f64);
        let cols = ((w as f64 * s / cw as f64) as u16).clamp(1, area.width);
        let rows = ((h as f64 * s / ch as f64) as u16).clamp(1, area.height);
        return Some(Placement {
            x: area.x + (area.width - cols) / 2,
            y: area.y + (area.height - rows) / 2,
            scale: 1,
            cells: Some((cols, rows)),
        });
    }
    // Unknown cell size: assume cells are about twice as tall as wide.
    let s = (area.width as f64 / w as f64).min(area.height as f64 * 2.0 / h as f64);
    let cols = ((w as f64 * s) as u16).clamp(1, area.width);
    let rows = ((h as f64 * s / 2.0) as u16).clamp(1, area.height);
    Some(Placement {
        x: area.x + (area.width - cols) / 2,
        y: area.y + (area.height - rows) / 2,
        scale: 1,
        cells: Some((cols, rows)),
    })
}

/// Nearest-neighbor upscale to RGB8, then PNG.
pub fn encode_png(s: &Screenshot, k: u32) -> Vec<u8> {
    let k = k.max(1) as usize;
    let (w, h) = (s.width * k, s.height * k);
    let src = s.to_rgb8();
    let mut rgb = Vec::with_capacity(w * h * 3);
    for y in 0..s.height {
        let mut row = Vec::with_capacity(w * 3);
        for x in 0..s.width {
            let p = &src[(y * s.width + x) * 3..][..3];
            for _ in 0..k {
                row.extend_from_slice(p);
            }
        }
        for _ in 0..k {
            rgb.extend_from_slice(&row);
        }
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().expect("png header");
        wr.write_image_data(&rgb).expect("png data");
    }
    out
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

/// The escape sequences that put `png` at `p`.
pub fn display_commands(png: &[u8], p: &Placement) -> Vec<u8> {
    let mut out = Vec::new();
    // Drop the previous image (and its placements), then save the cursor and
    // move to the target cell.
    let _ = write!(out, "\x1b_Ga=d,d=I,i={IMAGE_ID},q=2\x1b\\");
    let _ = write!(out, "\x1b7\x1b[{};{}H", p.y + 1, p.x + 1);
    let b64 = base64(png);
    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(4096).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = (i + 1 < chunks.len()) as u8;
        if i == 0 {
            // q=2: no replies (they'd land in our key input), C=1: leave the
            // cursor alone.
            let _ = write!(out, "\x1b_Ga=T,f=100,i={IMAGE_ID},q=2,C=1");
            if let Some((c, r)) = p.cells {
                let _ = write!(out, ",c={c},r={r}");
            }
            let _ = write!(out, ",m={more};");
        } else {
            let _ = write!(out, "\x1b_Gm={more};");
        }
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
    out.extend_from_slice(b"\x1b8");
    out
}

pub fn delete_command() -> Vec<u8> {
    format!("\x1b_Ga=d,d=I,i={IMAGE_ID},q=2\x1b\\").into_bytes()
}

/// What the terminal currently shows: area, image generation, cell size in px.
type Shown = (Rect, u64, Option<(u32, u32)>);

/// Keeps the terminal's image in step with what the UI wants shown.
#[derive(Default)]
pub struct KittyImage {
    /// (area, screenshot generation, cell size) currently on screen.
    shown: Option<Shown>,
}

impl KittyImage {
    /// `want`: popup area and the screenshot with its generation number.
    pub fn sync(
        &mut self,
        out: &mut impl Write,
        want: Option<(Rect, &Screenshot, u64)>,
    ) -> std::io::Result<()> {
        let cell = cell_size();
        match want {
            None => {
                if self.shown.take().is_some() {
                    out.write_all(&delete_command())?;
                    out.flush()?;
                }
            }
            Some((area, shot, generation)) => {
                if self.shown == Some((area, generation, cell)) {
                    return Ok(());
                }
                if let Some(p) = place(area, shot.width as u32, shot.height as u32, cell) {
                    out.write_all(&display_commands(&encode_png(shot, p.scale), &p))?;
                    out.flush()?;
                }
                self.shown = Some((area, generation, cell));
            }
        }
        Ok(())
    }
}

/// Cell size in pixels, when the terminal reports its pixel size.
fn cell_size() -> Option<(u32, u32)> {
    let ws = ratatui::crossterm::terminal::window_size().ok()?;
    if ws.width == 0 || ws.height == 0 || ws.columns == 0 || ws.rows == 0 {
        return None;
    }
    Some((
        ws.width as u32 / ws.columns as u32,
        ws.height as u32 / ws.rows as u32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn integer_scale_when_it_fits() {
        // 100x40 cells of 10x20 px = 1000x800 px -> 3x (960x720), 96x36 cells.
        let p = place(Rect::new(0, 0, 100, 40), 320, 240, Some((10, 20))).unwrap();
        assert_eq!(
            p,
            Placement {
                x: 2,
                y: 2,
                scale: 3,
                cells: None
            }
        );
    }

    #[test]
    fn shrinks_when_too_small() {
        let p = place(Rect::new(0, 0, 20, 10), 320, 240, Some((10, 20))).unwrap();
        assert_eq!(p.scale, 1);
        let (c, r) = p.cells.unwrap();
        assert!(c <= 20 && r <= 10);
    }

    #[test]
    fn png_payload_decodes_at_scaled_size() {
        let mut rgb565 = vec![0u8; 320 * 240 * 2];
        rgb565[0] = 0x1F; // top-left pixel blue
        let shot = Screenshot {
            width: 320,
            height: 240,
            rgb565,
        };
        let png_bytes = encode_png(&shot, 3);
        let dec = png::Decoder::new(std::io::Cursor::new(png_bytes));
        let mut reader = dec.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (960, 720));
        // The blue pixel became a 3x3 block, its right neighbour is black.
        assert_eq!(&buf[..3], &[0, 0, 255]);
        assert_eq!(&buf[3 * 2..3 * 3], &[0, 0, 255]);
        assert_eq!(&buf[3 * 3..3 * 4], &[0, 0, 0]);
        assert_eq!(&buf[960 * 3 * 2..960 * 3 * 2 + 3], &[0, 0, 255]);
    }

    #[test]
    fn chunked_transmission() {
        let p = Placement {
            x: 4,
            y: 2,
            scale: 1,
            cells: None,
        };
        let cmd = String::from_utf8(display_commands(&vec![0u8; 9000], &p)).unwrap();
        assert!(cmd.starts_with(
            "\x1b_Ga=d,d=I,i=31,q=2\x1b\\\x1b7\x1b[3;5H\x1b_Ga=T,f=100,i=31,q=2,C=1,m=1;"
        ));
        // 9000 bytes -> 12000 base64 chars -> 3 chunks, the last with m=0.
        assert_eq!(cmd.matches("\x1b_Gm=1;").count(), 1);
        assert_eq!(cmd.matches("\x1b_Gm=0;").count(), 1);
        assert!(cmd.ends_with("\x1b\\\x1b8"));
    }
}
