//! Packet dissector producing the same text as libticalcs' `dusb_dissect()`
//! (dusb_rpkt.c + `dusb_dissect_cmd_data` in dusb_cmd.c), quirks included,
//! so logs can be diffed against libticalcs output.

use super::cmd::param_name;
use super::*;
use crate::error::CalcErrorCode;
use std::fmt::Write;

const UNEXPECTED_SIZE: &str = "(unexpected size for a packet of that type)\n";

fn ep_way(dir: Dir) -> &'static str {
    match dir {
        Dir::TiToPc => "TI>PC",
        Dir::PcToTi => "PC>TI",
    }
}

fn be32(d: &[u8]) -> u32 {
    u32::from_be_bytes([d[0], d[1], d[2], d[3]])
}

fn be16(d: &[u8]) -> u16 {
    u16::from_be_bytes([d[0], d[1]])
}

/// Stateful: like libticalcs it remembers whether the next data packet starts
/// a new virtual packet.
pub struct Dissector {
    first: bool,
}

impl Default for Dissector {
    fn default() -> Self {
        Self { first: true }
    }
}

impl Dissector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Dissect one raw packet. Returns `None` for zero-length packets.
    pub fn dissect(&mut self, dir: Dir, data: &[u8]) -> Option<String> {
        if data.is_empty() {
            return None;
        }
        // Like libticalcs, problems are reported out of band (it uses
        // ticalcs_critical); the dissection itself just stops.
        if data.len() < 5 || data.len() > RPKT_MAX_DATA {
            return None;
        }
        let mut f = String::new();
        let raw_size = be32(data);
        let raw_type = data[4];
        let _ = writeln!(
            f,
            "{:08X} ({:02X})\t\t\t\t\t\t\t| {}: {}",
            raw_size,
            raw_type,
            ep_way(dir),
            rpkt_name(raw_type)
        );
        if raw_size as usize > RPKT_MAX_DATA - 5 {
            return Some(f);
        }
        let min_data = match raw_type {
            1 | 2 => 4,
            3 | 4 => 6,
            5 => 2,
            _ => return Some(f),
        };
        if data.len() < 5 + min_data {
            return Some(f);
        }

        match raw_type {
            RPKT_BUF_SIZE_REQ | RPKT_BUF_SIZE_ALLOC => {
                let _ = writeln!(f, "\t[{:08X}]", be32(&data[5..]));
                if data.len() != 9 {
                    f.push_str(UNEXPECTED_SIZE);
                }
            }
            RPKT_VIRT_DATA | RPKT_VIRT_DATA_LAST => {
                if self.first {
                    let vtl_size = be32(&data[5..]);
                    let vtl_type = be16(&data[9..]);
                    let _ = writeln!(
                        f,
                        "\t{:08X} {{{:04X}}}\t\t\t\t\t\t| CMD: {}",
                        vtl_size,
                        vtl_type,
                        vpkt_name(vtl_type)
                    );
                    if vtl_size != raw_size.wrapping_sub(6) {
                        f.push_str(UNEXPECTED_SIZE);
                    }
                    if vtl_size == 0 {
                        f.push_str("(no data to dissect)\n");
                    } else {
                        let payload = &data[11..];
                        let len = payload.len();
                        if len < vtl_size as usize {
                            f.push_str("(packet truncated: insufficient given length)\n");
                        } else if len > vtl_size as usize {
                            f.push_str("(given length larger than length in packet)\n");
                        }
                        f.push_str("\t\t");
                        hex_wrapped(&mut f, payload, "\n\t\t");
                        f.push('\n');
                        // libticalcs then does `data -= vtl_size` after
                        // advancing by `len`, so when the virtual packet
                        // continues in later raw packets it decodes from
                        // `len - vtl_size` bytes before the payload (for CE
                        // OS Data that garbles the address). Copied so logs
                        // diff cleanly; bytes before the packet read as 0.
                        let start = 11 + len as isize - vtl_size as isize;
                        let shifted: Vec<u8> = (start..start + len as isize)
                            .map(|i| usize::try_from(i).ok().and_then(|i| data.get(i)))
                            .map(|b| b.copied().unwrap_or(0))
                            .collect();
                        if !dissect_cmd_data(&mut f, &shifted, vtl_type) {
                            f.push_str("(inner data dissection had a problem with the packet)\n");
                        }
                    }
                } else {
                    let _ = write!(f, "\t{:02X} {:02X} {:02X} ", data[5], data[6], data[7]);
                    let rest = &data[8..];
                    let expected = raw_size.saturating_sub(3) as usize;
                    if rest.len() < expected {
                        f.push_str("(packet truncated: insufficient given length)\n");
                    } else if rest.len() > expected {
                        f.push_str("(given length larger than length in packet)\n");
                    }
                    hex_wrapped(&mut f, rest, "\n\t\t");
                    f.push('\n');
                }
                self.first = raw_type != RPKT_VIRT_DATA;
            }
            RPKT_VIRT_DATA_ACK => {
                let _ = writeln!(f, "\t[{:04X}]", be16(&data[5..]));
                if data.len() != 7 {
                    f.push_str(UNEXPECTED_SIZE);
                }
            }
            _ => unreachable!(),
        }
        Some(f)
    }
}

fn hex_wrapped(f: &mut String, bytes: &[u8], sep: &str) {
    for (i, b) in bytes.iter().enumerate() {
        let _ = write!(f, "{b:02X} ");
        if (i + 1) % 16 == 0 {
            f.push_str(sep);
        }
    }
}

/// `dusb_check_cmd_data`: false when the payload is malformed for its type.
fn check_cmd_data(d: &[u8], kind: u16) -> bool {
    let len = d.len();
    match kind {
        VPKT_PING => len == 10,
        VPKT_PARM_REQ => len >= 2 && len == 2 + be16(d) as usize * 2,
        VPKT_PARM_DATA => {
            if len < 2 {
                return false;
            }
            let n = be16(d) as usize;
            if len < 2 + 3 * n {
                return false;
            }
            let mut extra = 0usize;
            let mut p = 2usize;
            for _ in 0..n {
                let ok = d.get(p + 2) == Some(&0);
                p += 3;
                if ok {
                    let Some(sz) = d.get(p..p + 2).map(be16) else {
                        return false;
                    };
                    p += sz as usize + 2;
                    extra += sz as usize + 2;
                }
                if len < 2 + 3 * n + extra {
                    return false;
                }
            }
            len == 2 + 3 * n + extra
        }
        VPKT_PARM_SET => len > 4 && len == 4 + be16(&d[2..]) as usize,
        VPKT_OS_BEGIN => len == 11,
        VPKT_OS_ACK => len >= 4,
        VPKT_OS_HEADER | VPKT_OS_DATA => len > 4,
        VPKT_DELAY_ACK => len == 4,
        VPKT_ERROR => len == 2,
        VPKT_VAR_CNTS | VPKT_MODE_SET | VPKT_EOT_ACK | VPKT_DATA_ACK | VPKT_EOT => true,
        VPKT_DIR_REQ | VPKT_VAR_HDR | VPKT_RTS | VPKT_VAR_REQ | VPKT_MODIF_VAR | VPKT_EXECUTE => {
            true
        }
        _ => false,
    }
}

fn dissect_cmd_data(f: &mut String, d: &[u8], kind: u16) -> bool {
    if !check_cmd_data(d, kind) {
        return false;
    }
    match kind {
        VPKT_PING => {
            let a: Vec<u16> = (0..5).map(|i| be16(&d[i * 2..])).collect();
            let _ = writeln!(
                f,
                "Set mode: {{ {}, {}, {}, {}, 0x{:04X} }}",
                a[0], a[1], a[2], a[3], a[4]
            );
        }
        VPKT_PARM_REQ => {
            let n = be16(d) as usize;
            let _ = writeln!(f, "Requested {n} ({n:X}) parameter IDs:");
            for i in 0..n {
                let pid = be16(&d[2 + 2 * i..]);
                let _ = writeln!(f, "\t{:04X} ({})", pid, param_name(pid));
            }
            f.push('\n');
        }
        VPKT_PARM_DATA => {
            let n = be16(d) as usize;
            let _ = writeln!(f, "Received {n} ({n:X}) parameter values:");
            let mut p = 2;
            let mut extra = 0usize;
            for _ in 0..n {
                let pid = be16(&d[p..]);
                p += 2;
                let ok = d[p] == 0;
                p += 1;
                let _ = write!(f, "\t{:04X} ({}): ", pid, param_name(pid));
                if ok {
                    let size = be16(&d[p..]) as usize;
                    p += 2;
                    extra += size + 2;
                    if d.len() < 2 + 3 * n + extra {
                        break;
                    }
                    let _ = write!(f, "OK, size {size:04X}\n\t\t");
                    hex_wrapped(f, &d[p..p + size], "\n\t\t");
                    p += size;
                    f.push('\n');
                } else {
                    f.push_str("NOK !\n");
                }
            }
        }
        VPKT_PARM_SET => {
            let id = be16(d);
            let size = be16(&d[2..]) as usize;
            let _ = write!(
                f,
                "Sending value of size {size:04X} for parameter {id:04X}\n\t"
            );
            // libticalcs increments its counter twice per byte here; keep that.
            let mut i = 0;
            let mut p = 4;
            while i < size {
                let _ = write!(f, "{:02X} ", d[p]);
                p += 1;
                i += 1;
                if i & 15 == 0 {
                    f.push_str("\n\t");
                }
                i += 1;
            }
        }
        VPKT_OS_BEGIN => {
            let size = be32(&d[7..]);
            let _ = writeln!(f, "Size: {size} / {size:08X}");
        }
        VPKT_OS_ACK => {
            let size = be32(d);
            let _ = writeln!(f, "Chunk size: {size} / {size:08X}");
        }
        VPKT_OS_HEADER | VPKT_OS_DATA => {
            let addr = u32::from_le_bytes([d[0], d[1], d[2], d[3]]);
            let _ = writeln!(f, "Address: {addr:08X}");
        }
        VPKT_DELAY_ACK => {
            let _ = writeln!(f, "Delay: {}", be32(d));
        }
        VPKT_ERROR => {
            let err = CalcErrorCode(be16(d)).libticalcs_index();
            let _ = writeln!(f, "Error code: {err} ({err:04X})");
        }
        VPKT_VAR_CNTS | VPKT_MODE_SET | VPKT_EOT_ACK | VPKT_DATA_ACK | VPKT_EOT => {}
        VPKT_DIR_REQ | VPKT_VAR_HDR | VPKT_RTS | VPKT_VAR_REQ | VPKT_MODIF_VAR | VPKT_EXECUTE => {
            f.push_str("(no extra dissection performed for now)\n");
        }
        _ => {
            f.push_str("(not performing extra dissection on unknown vpkt type)\n");
        }
    }
    true
}

/// One-line summary of a raw packet for the TUI's packet log.
pub fn summarize(dir: Dir, data: &[u8], first: &mut bool) -> String {
    let arrow = match dir {
        Dir::PcToTi => "PC→TI",
        Dir::TiToPc => "TI→PC",
    };
    if data.is_empty() {
        return format!("{arrow} (zero-length packet)");
    }
    if data.len() < 5 {
        return format!("{arrow} short packet ({} bytes)", data.len());
    }
    let size = be32(data);
    let kind = data[4];
    let body = &data[5..];
    let s = match kind {
        RPKT_BUF_SIZE_REQ | RPKT_BUF_SIZE_ALLOC if body.len() >= 4 => {
            format!("{} ({} bytes)", rpkt_name(kind), be32(body))
        }
        RPKT_VIRT_DATA_ACK => "ack".to_string(),
        RPKT_VIRT_DATA | RPKT_VIRT_DATA_LAST => {
            let s = if *first && body.len() >= 6 {
                let vtype = be16(&body[4..]);
                let vsize = be32(body);
                let cont = if kind == RPKT_VIRT_DATA { " …" } else { "" };
                format!("{} [{vtype:04X}] {vsize} bytes{cont}", vpkt_name(vtype))
            } else {
                format!("  continuation, {size} bytes")
            };
            *first = kind == RPKT_VIRT_DATA_LAST;
            s
        }
        _ => format!("raw type {kind} ({size} bytes)"),
    };
    format!("{arrow} {s}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dissect_ping() {
        let pkt = [
            0, 0, 0, 0x10, 4, 0, 0, 0, 0x0A, 0, 1, 0, 3, 0, 1, 0, 0, 0, 0, 7, 0xD0,
        ];
        let out = Dissector::new().dissect(Dir::PcToTi, &pkt).unwrap();
        assert_eq!(
            out,
            "00000010 (04)\t\t\t\t\t\t\t| PC>TI: Virtual Packet Data Final\n\
             \t0000000A {0001}\t\t\t\t\t\t| CMD: Ping / Set Mode\n\
             \t\t00 03 00 01 00 00 00 00 07 D0 \n\
             Set mode: { 3, 1, 0, 0, 0x07D0 }\n"
        );
    }

    #[test]
    fn dissect_ack_and_bufsize() {
        let mut d = Dissector::new();
        assert_eq!(
            d.dissect(Dir::TiToPc, &[0, 0, 0, 2, 5, 0xE0, 0]).unwrap(),
            "00000002 (05)\t\t\t\t\t\t\t| TI>PC: Virtual Packet Data Acknowledgement\n\t[E000]\n"
        );
        assert_eq!(
            d.dissect(Dir::PcToTi, &[0, 0, 0, 4, 1, 0, 0, 4, 0])
                .unwrap(),
            "00000004 (01)\t\t\t\t\t\t\t| PC>TI: Buffer Size Request\n\t[00000400]\n"
        );
    }
}
