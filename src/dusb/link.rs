//! Raw and virtual packet handling (`dusb_rpkt.c` + `dusb_vpkt.c` in libticalcs).

use super::*;
use crate::error::{CalcErrorCode, Error, Result};
use crate::transport::Transport;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    PcToTi,
    TiToPc,
}

impl Dir {
    /// Endpoint number used by libticalcs' dissector to label the direction.
    pub fn ep(self) -> u8 {
        match self {
            Dir::PcToTi => 0x02,
            Dir::TiToPc => 0x01,
        }
    }
}

/// Which end of the cable we are. The simulator runs the device side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Host,
    Device,
}

/// Called with every raw packet (header included) as it crosses the wire,
/// and with an empty slice for zero-length packets.
pub type LinkObserver = Box<dyn FnMut(Dir, &[u8]) + Send>;

pub struct RawPacket {
    pub kind: u8,
    pub data: Vec<u8>,
}

impl RawPacket {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(5 + self.data.len());
        out.extend_from_slice(&(self.data.len() as u32).to_be_bytes());
        out.push(self.kind);
        out.extend_from_slice(&self.data);
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualPacket {
    pub kind: u16,
    pub data: Vec<u8>,
}

pub struct Link<T: Transport> {
    transport: T,
    role: Role,
    /// `dusb_rpkt_maxlen`: largest raw packet data size we may send.
    max_raw: usize,
    /// What a device answers to buffer size requests.
    device_alloc: u32,
    observers: Vec<LinkObserver>,
    /// Progress callback for large virtual packets: (done, total) in bytes.
    progress: Option<Box<dyn FnMut(usize, usize) + Send>>,
}

impl<T: Transport> Link<T> {
    pub fn new(transport: T, role: Role) -> Self {
        Self {
            transport,
            role,
            max_raw: 250,
            device_alloc: 1023,
            observers: Vec::new(),
            progress: None,
        }
    }

    pub fn add_observer(&mut self, obs: LinkObserver) {
        self.observers.push(obs);
    }

    pub fn set_progress(&mut self, f: Option<Box<dyn FnMut(usize, usize) + Send>>) {
        self.progress = f;
    }

    pub fn set_device_alloc(&mut self, size: u32) {
        self.device_alloc = size;
    }

    pub fn max_raw(&self) -> usize {
        self.max_raw
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    pub fn set_timeout(&mut self, t: Duration) {
        self.transport.set_timeout(t);
    }

    pub fn timeout(&self) -> Duration {
        self.transport.timeout()
    }

    fn out_dir(&self) -> Dir {
        match self.role {
            Role::Host => Dir::PcToTi,
            Role::Device => Dir::TiToPc,
        }
    }

    fn notify(&mut self, dir: Dir, bytes: &[u8]) {
        for o in &mut self.observers {
            o(dir, bytes);
        }
    }

    // ---- raw packets ----

    pub fn send_raw(&mut self, kind: u8, data: &[u8]) -> Result<()> {
        let bytes = RawPacket {
            kind,
            data: data.to_vec(),
        }
        .encode();
        let dir = self.out_dir();
        self.notify(dir, &bytes);
        self.transport.send(&bytes)
    }

    fn send_zlp(&mut self) -> Result<()> {
        let dir = self.out_dir();
        self.notify(dir, &[]);
        self.transport.send(&[])
    }

    pub fn recv_raw(&mut self) -> Result<RawPacket> {
        let mut hdr = [0u8; 5];
        self.transport.recv_exact(&mut hdr)?;
        let size = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as usize;
        let kind = hdr[4];
        if size > RPKT_MAX_DATA {
            return Err(Error::InvalidPacket(format!(
                "raw packet is too large: {size} bytes"
            )));
        }
        let mut data = vec![0u8; size];
        self.transport.recv_exact(&mut data)?;
        let mut bytes = hdr.to_vec();
        bytes.extend_from_slice(&data);
        let dir = match self.role {
            Role::Host => Dir::TiToPc,
            Role::Device => Dir::PcToTi,
        };
        self.notify(dir, &bytes);
        Ok(RawPacket { kind, data })
    }

    /// Buffer size negotiation (`dusb_send_buf_size_request` + `dusb_recv_buf_size_alloc`).
    pub fn negotiate_buffer(&mut self, size: u32) -> Result<u32> {
        let size = size.min(RPKT_MAX_DATA as u32 + 1);
        self.send_raw(RPKT_BUF_SIZE_REQ, &size.to_be_bytes())?;
        let raw = self.recv_raw()?;
        if raw.kind != RPKT_BUF_SIZE_ALLOC || raw.data.len() != 4 {
            return Err(Error::InvalidPacket(format!(
                "expected buffer size allocation, got raw type {} ({} bytes)",
                raw.kind,
                raw.data.len()
            )));
        }
        let mut alloc = u32::from_be_bytes(raw.data[..4].try_into().unwrap());
        alloc = alloc.min(RPKT_MAX_DATA as u32).min(CE_MAX_RAW);
        self.max_raw = alloc as usize;
        Ok(alloc)
    }

    fn send_buf_size_alloc(&mut self, size: u32) -> Result<()> {
        let size = size.min(RPKT_MAX_DATA as u32 + 1);
        self.send_raw(RPKT_BUF_SIZE_ALLOC, &size.to_be_bytes())?;
        self.max_raw = size as usize;
        Ok(())
    }

    pub fn send_ack(&mut self) -> Result<()> {
        self.send_raw(RPKT_VIRT_DATA_ACK, &[0xE0, 0x00])
    }

    /// `dusb_recv_acknowledge`, including answering a buffer size request
    /// the calculator may slip in before its ack.
    pub fn recv_ack(&mut self) -> Result<()> {
        let mut raw = self.recv_raw()?;
        if raw.data.len() != 2 && raw.data.len() != 4 {
            return Err(Error::InvalidPacket(format!(
                "expected ack, got raw type {} ({} bytes)",
                raw.kind,
                raw.data.len()
            )));
        }
        if raw.kind == RPKT_BUF_SIZE_REQ {
            if raw.data.len() != 4 {
                return Err(Error::InvalidPacket("bad buffer size request".into()));
            }
            let size = u32::from_be_bytes(raw.data[..4].try_into().unwrap());
            self.send_buf_size_alloc(size)?;
            raw = self.recv_raw()?;
        }
        if raw.kind != RPKT_VIRT_DATA_ACK {
            return Err(Error::InvalidPacket(format!(
                "expected ack, got raw type {}",
                raw.kind
            )));
        }
        // Same (lenient) check as libticalcs.
        if raw.data.len() >= 2 && raw.data[0] != 0xE0 && raw.data[1] != 0x00 {
            return Err(Error::InvalidPacket(format!(
                "bad ack payload {:02X} {:02X}",
                raw.data[0], raw.data[1]
            )));
        }
        Ok(())
    }

    // ---- virtual packets ----

    /// libticalcs' `workaround_send` for the 83PCE/84+CE: TI's OS needs an
    /// extra zero-length bulk write after a final raw packet that fills a
    /// whole number of 64-byte USB packets.
    fn ce_zlp_workaround(&mut self, raw_kind: u8, raw_size: usize) -> Result<()> {
        if self.role == Role::Host
            && raw_kind == RPKT_VIRT_DATA_LAST
            && (raw_size + 5).is_multiple_of(64)
        {
            self.send_zlp()?;
        }
        Ok(())
    }

    /// `dusb_send_data`: fragment a virtual packet into raw packets.
    pub fn send_vpkt(&mut self, kind: u16, data: &[u8]) -> Result<()> {
        let max = self.max_raw;
        let total = data.len();
        let mut header = Vec::with_capacity(VPKT_HEADER);
        header.extend_from_slice(&(total as u32).to_be_bytes());
        header.extend_from_slice(&kind.to_be_bytes());

        if total <= max - VPKT_HEADER {
            // A single raw packet, which is also the final one.
            let mut raw = header;
            raw.extend_from_slice(data);
            let raw_size = raw.len();
            self.send_raw(RPKT_VIRT_DATA_LAST, &raw)?;
            self.ce_zlp_workaround(RPKT_VIRT_DATA_LAST, raw_size)?;
            return self.recv_ack();
        }

        // First raw packet carries the virtual packet header.
        let first = max - VPKT_HEADER;
        let mut raw = header;
        raw.extend_from_slice(&data[..first]);
        self.send_raw(RPKT_VIRT_DATA, &raw)?;
        self.recv_ack()?;
        let mut offset = first;

        // Then full chunks without header...
        let q = (total - offset) / max;
        let r = (total - offset) % max;
        for _ in 0..q {
            self.send_raw(RPKT_VIRT_DATA, &data[offset..offset + max])?;
            self.recv_ack()?;
            offset += max;
            if let Some(p) = &mut self.progress {
                p(offset, total);
            }
        }

        // ...and a final one. Like libticalcs this is sent even when empty.
        self.send_raw(RPKT_VIRT_DATA_LAST, &data[offset..offset + r])?;
        self.ce_zlp_workaround(RPKT_VIRT_DATA_LAST, r)?;
        self.recv_ack()?;
        if let Some(p) = &mut self.progress {
            p(total, total);
        }
        Ok(())
    }

    /// `dusb_recv_data`: reassemble a virtual packet, acking each raw packet.
    pub fn recv_vpkt(&mut self) -> Result<VirtualPacket> {
        let mut declared = 0usize;
        let mut kind = 0u16;
        let mut data = Vec::new();
        let mut first = true;
        loop {
            let raw = self.recv_raw()?;
            if first && self.role == Role::Device && raw.kind == RPKT_BUF_SIZE_REQ {
                // The host negotiates before its first virtual packet: it tells
                // us how much it can take, we tell it how much we can.
                if raw.data.len() != 4 {
                    return Err(Error::InvalidPacket("bad buffer size request".into()));
                }
                let req = u32::from_be_bytes(raw.data[..4].try_into().unwrap());
                let alloc = self.device_alloc;
                self.send_raw(RPKT_BUF_SIZE_ALLOC, &alloc.to_be_bytes())?;
                self.max_raw = (req as usize).min(RPKT_MAX_DATA);
                continue;
            }
            if raw.kind != RPKT_VIRT_DATA && raw.kind != RPKT_VIRT_DATA_LAST {
                return Err(Error::InvalidPacket(format!(
                    "unexpected raw packet type {}",
                    raw.kind
                )));
            }
            if first {
                if raw.data.len() < VPKT_HEADER {
                    return Err(Error::InvalidPacket("first raw packet is too small".into()));
                }
                declared = u32::from_be_bytes(raw.data[..4].try_into().unwrap()) as usize;
                kind = u16::from_be_bytes([raw.data[4], raw.data[5]]);
                data.reserve(declared.min(1 << 20));
                data.extend_from_slice(&raw.data[VPKT_HEADER..]);
                first = false;
            } else {
                data.extend_from_slice(&raw.data);
                if let Some(p) = &mut self.progress {
                    p(data.len(), declared.max(data.len()));
                }
            }
            self.send_ack()?;
            if raw.kind == RPKT_VIRT_DATA_LAST {
                break;
            }
        }
        if declared != data.len() {
            return Err(Error::InvalidPacket(format!(
                "declared size {declared} but received {} bytes",
                data.len()
            )));
        }
        Ok(VirtualPacket { kind, data })
    }

    /// Receive a virtual packet, sitting out delay acks and turning error
    /// packets into errors (the `CATCH_DELAY` + error check in dusb_cmd.c).
    pub fn recv_vpkt_checked(&mut self) -> Result<VirtualPacket> {
        loop {
            let pkt = self.recv_vpkt()?;
            match pkt.kind {
                VPKT_DELAY_ACK if pkt.data.len() >= 4 => {
                    let us = u32::from_be_bytes(pkt.data[..4].try_into().unwrap()).min(400_000);
                    std::thread::sleep(Duration::from_micros(us as u64));
                }
                VPKT_ERROR if pkt.data.len() >= 2 => {
                    return Err(Error::Calc(CalcErrorCode(u16::from_be_bytes([
                        pkt.data[0],
                        pkt.data[1],
                    ]))));
                }
                _ => return Ok(pkt),
            }
        }
    }

    /// Receive a virtual packet of the given type.
    pub fn expect(&mut self, kind: u16) -> Result<VirtualPacket> {
        let pkt = self.recv_vpkt_checked()?;
        if pkt.kind != kind {
            if pkt.kind == VPKT_EOT {
                return Err(Error::Eot);
            }
            return Err(Error::InvalidPacket(format!(
                "expected {} ({kind:04X}), got {} ({:04X})",
                vpkt_name(kind),
                vpkt_name(pkt.kind),
                pkt.kind
            )));
        }
        Ok(pkt)
    }
}
