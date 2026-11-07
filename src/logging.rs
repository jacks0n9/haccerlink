//! Packet logs.
//!
//! * [`HexLog`] writes the same format as libticables' `ticables-log.hex`
//!   (what TiLP produces with logging enabled), so the two can be diffed.
//! * [`Transcript`] records every raw packet with its direction; it is what
//!   the oracle tests compare against libticalcs.

use crate::dusb::{Dir, LinkObserver};
use std::fmt::Write as _;
use std::io::Write;
use std::sync::{Arc, Mutex};

/// Version string we claim in the hex log header (the libticables we mirror).
pub const TICABLES_VERSION: &str = "1.3.5";

pub struct HexLog<W: Write> {
    out: W,
    line: Vec<u8>,
}

impl<W: Write> HexLog<W> {
    pub fn new(mut out: W) -> std::io::Result<Self> {
        write!(
            out,
            "TiCables-2 data logger\nVersion {TICABLES_VERSION}\n\n"
        )?;
        Ok(Self {
            out,
            line: Vec::with_capacity(16),
        })
    }

    /// `log_hex_N`: direction is not recorded.
    pub fn log(&mut self, data: &[u8]) -> std::io::Result<()> {
        for &b in data {
            self.line.push(b);
            write!(self.out, "{b:02X} ")?;
            if self.line.len() == 16 {
                write!(self.out, "| ")?;
                for &c in &self.line {
                    let ch = if !(32..=127).contains(&c) {
                        ' '
                    } else {
                        c as char
                    };
                    write!(self.out, "{ch}")?;
                }
                writeln!(self.out)?;
                self.line.clear();
            }
        }
        self.out.flush()
    }
}

/// Observer that appends to a TiLP-format hex log file.
pub fn hex_log_observer(path: &std::path::Path) -> std::io::Result<LinkObserver> {
    let mut log = HexLog::new(std::fs::File::create(path)?)?;
    Ok(Box::new(move |_dir, bytes| {
        let _ = log.log(bytes);
    }))
}

/// Every packet seen so far, with its direction.
pub type Packets = Vec<(Dir, Vec<u8>)>;

#[derive(Clone, Default)]
pub struct Transcript(Arc<Mutex<Packets>>);

impl Transcript {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn observer(&self) -> LinkObserver {
        let inner = self.0.clone();
        Box::new(move |dir, bytes| inner.lock().unwrap().push((dir, bytes.to_vec())))
    }

    pub fn packets(&self) -> Vec<(Dir, Vec<u8>)> {
        self.0.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
    }

    /// One line per packet: `PC>TI 00 00 00 04 01 ...` (or `ZLP`).
    pub fn render(&self) -> String {
        render_packets(&self.packets())
    }
}

pub fn render_packets(pkts: &[(Dir, Vec<u8>)]) -> String {
    let mut s = String::new();
    for (dir, bytes) in pkts {
        s.push_str(match dir {
            Dir::PcToTi => "PC>TI",
            Dir::TiToPc => "TI>PC",
        });
        if bytes.is_empty() {
            s.push_str(" ZLP");
        }
        for b in bytes {
            let _ = write!(s, " {b:02X}");
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_log_format() {
        let mut buf = Vec::new();
        {
            let mut log = HexLog::new(&mut buf).unwrap();
            log.log(&[0, 0, 0, 4, 1, 0, 0, 4, 0]).unwrap();
            log.log(&[0, 0, 0, 4, 2, 0, 0, 3, 0xFF]).unwrap();
        }
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(
            s,
            "TiCables-2 data logger\nVersion 1.3.5\n\n\
             00 00 00 04 01 00 00 04 00 00 00 00 04 02 00 00 |                 \n\
             03 FF "
        );
    }
}
