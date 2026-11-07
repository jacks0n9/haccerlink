//! A simulated TI-84 Plus CE: the calculator side of DUSB.
//!
//! It runs on the same [`Link`] code with [`Role::Device`] and answers the
//! way the CE OS does as far as libticalcs and CEmu document it. The
//! identification values (OS version, calc ID, memory) are made up but
//! plausible. Use it to try haccerlink without hardware (`--sim`), in
//! tests, and as the far end of the libticalcs oracle (`sim-serve`).

use crate::calc::{LCD_HEIGHT, LCD_WIDTH};
use crate::dusb::cmd::{self, Value};
use crate::dusb::*;
use crate::error::{Error, Result};
use crate::names::types;
use crate::transport::{ChannelTransport, Transport, channel_pair};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimVar {
    pub name: String,
    pub type_id: u8,
    pub archived: bool,
    pub version: u8,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct SimState {
    pub vars: Vec<SimVar>,
    pub clock: [u8; 7], // sec, min, hour, day, month, year hi, year lo
    pub date_fmt: u8,
    pub time_fmt: u8,
    pub clock_on: u8,
    pub keys: Vec<u16>,
    pub executed: Vec<String>,
    /// Delay the simulated OS asks for after a key press, in microseconds.
    pub key_delay_us: u32,
}

const RAM_USER: u64 = 154_000;
const FLASH_USER: u64 = 3_014_656;

impl Default for SimState {
    fn default() -> Self {
        let var = |name: &str, type_id, archived, data: &[u8]| SimVar {
            name: name.into(),
            type_id,
            archived,
            version: 0,
            data: data.to_vec(),
        };
        // :Disp "HELLO WORLD"
        let mut prog = vec![0x00, 0x00, 0xDE, 0x2A];
        prog.extend_from_slice(b"HELLO WORLD");
        prog.push(0x2A);
        let len = (prog.len() - 2) as u16;
        prog[..2].copy_from_slice(&len.to_le_bytes());

        let mut appvar = vec![0, 0];
        appvar.extend_from_slice(b"saved game data\0");
        let alen = (appvar.len() - 2) as u16;
        appvar[..2].copy_from_slice(&alen.to_le_bytes());

        // A tiny fake app: 81 0F header, then the CE app name field (0x8140).
        let mut app = vec![
            0x81, 0x0F, 0x00, 0x00, 0x00, 0x20, 0x81, 0x44, b'D', b'E', b'M', b'O',
        ];
        app.resize(0x26, 0xFF);

        Self {
            vars: vec![
                var("HELLO", types::PROGRAM, false, &prog),
                var("SAVEDAT", types::APPVAR, true, &appvar),
                // A = 42
                var(
                    "A",
                    types::REAL,
                    false,
                    &[0x00, 0x81, 0x42, 0, 0, 0, 0, 0, 0],
                ),
                // L₁ = {1}
                var(
                    "L₁",
                    types::LIST,
                    false,
                    &[0x01, 0x00, 0x00, 0x80, 0x10, 0, 0, 0, 0, 0, 0],
                ),
                // Str1 = "HI"
                var("Str1", types::STRING, false, &[0x02, 0x00, b'H', b'I']),
                // [A] = [[7]]
                var(
                    "[A]",
                    types::MATRIX,
                    false,
                    &[0x01, 0x01, 0x00, 0x80, 0x70, 0, 0, 0, 0, 0, 0],
                ),
                var("DEMO", types::APP, true, &app),
            ],
            clock: [
                0,
                30,
                12,
                6,
                11,
                (2026u16 >> 8) as u8,
                (2026u16 & 0xFF) as u8,
            ],
            date_fmt: 1,
            time_fmt: 0x80,
            clock_on: 1,
            keys: Vec::new(),
            executed: Vec::new(),
            key_delay_us: 10_000,
        }
    }
}

impl SimState {
    fn ram_free(&self) -> u64 {
        let used: u64 = self
            .vars
            .iter()
            .filter(|v| !v.archived)
            .map(|v| v.data.len() as u64 + 9)
            .sum();
        RAM_USER.saturating_sub(used)
    }

    fn flash_free(&self) -> u64 {
        let used: u64 = self
            .vars
            .iter()
            .filter(|v| v.archived)
            .map(|v| v.data.len() as u64 + 16)
            .sum();
        FLASH_USER.saturating_sub(used)
    }

    fn param(&self, id: u16) -> Option<Vec<u8>> {
        use cmd::*;
        let be64 = |v: u64| v.to_be_bytes().to_vec();
        Some(match id {
            PID_PRODUCT_NUMBER => vec![0x00, 0x00, 0x00, 0x13],
            PID_PRODUCT_NAME => b"TI-84 Plus CE\0".to_vec(),
            PID_MAIN_PART_ID => vec![0x13, 0x1E, 0xAC, 0xC0, 0xDE],
            PID_HW_VERSION => vec![0x00, 0x07],
            PID_LANGUAGE_ID => vec![0x09],
            PID_SUBLANG_ID => vec![0x01],
            PID_DEVICE_TYPE => vec![0x00, 0x73],
            PID_BOOT_VERSION => vec![0x00, 0x05, 0x03, 0x06],
            PID_OS_MODE => vec![0x01],
            PID_OS_VERSION => vec![0x00, 0x05, 0x08, 0x02],
            PID_PHYS_RAM => be64(0x40000),
            PID_USER_RAM => be64(RAM_USER),
            PID_FREE_RAM => be64(self.ram_free()),
            PID_PHYS_FLASH => be64(0x400000),
            PID_USER_FLASH => be64(FLASH_USER),
            PID_FREE_FLASH => be64(self.flash_free()),
            PID_HAS_SCREEN => vec![0x01],
            PID_COLOR_AVAILABLE => vec![0x01],
            PID_BITS_PER_PIXEL => vec![0x10],
            PID_LCD_WIDTH => (LCD_WIDTH as u16).to_be_bytes().to_vec(),
            PID_LCD_HEIGHT => (LCD_HEIGHT as u16).to_be_bytes().to_vec(),
            PID_SCREENSHOT => sim_screen(self),
            PID_CLK_ON => vec![self.clock_on],
            PID_CLK_DATE_FMT => vec![self.date_fmt],
            PID_CLK_TIME_FMT => vec![self.time_fmt],
            PID_BATTERY => vec![0x04],
            PID_NEW_CLK_SUPPORT => vec![0x01],
            PID_CLK_SECONDS => vec![self.clock[0]],
            PID_CLK_MINUTES => vec![self.clock[1]],
            PID_CLK_HOURS => vec![self.clock[2]],
            PID_CLK_DAY => vec![self.clock[3]],
            PID_CLK_MONTH => vec![self.clock[4]],
            PID_CLK_YEAR => vec![self.clock[5], self.clock[6]],
            PID_OS_BUILD_NUMBER => vec![0x00, 0x14],
            PID_BOOT_BUILD_NUMBER => vec![0x00, 0x11],
            PID_EXACT_MATH => vec![0x00],
            // The CE refuses the classic clock parameters.
            _ => return None,
        })
    }

    fn set_param(&mut self, id: u16, d: &[u8]) -> bool {
        use cmd::*;
        let Some(&b) = d.first() else { return false };
        match id {
            PID_CLK_SECONDS => self.clock[0] = b,
            PID_CLK_MINUTES => self.clock[1] = b,
            PID_CLK_HOURS => self.clock[2] = b,
            PID_CLK_DAY => self.clock[3] = b,
            PID_CLK_MONTH => self.clock[4] = b,
            PID_CLK_YEAR if d.len() == 2 => {
                self.clock[5] = d[0];
                self.clock[6] = d[1];
            }
            PID_CLK_DATE_FMT => self.date_fmt = b,
            PID_CLK_TIME_FMT => self.time_fmt = b,
            PID_CLK_ON => self.clock_on = b,
            _ => return false,
        }
        true
    }

    fn find(&self, name: &[u8], type_id: Option<u8>) -> Option<usize> {
        self.vars
            .iter()
            .position(|v| v.name.as_bytes() == name && type_id.is_none_or(|t| t == v.type_id))
    }
}

/// A recognizable screen: a gradient with a banner and one row per variable.
fn sim_screen(state: &SimState) -> Vec<u8> {
    let mut px = vec![0u8; 7 + LCD_WIDTH * LCD_HEIGHT * 2];
    // 1 parameter, id 0x0022, ok, then a u16 size that can't hold 153600;
    // libticalcs ignores it and takes everything after these 7 bytes.
    px[..7].copy_from_slice(&[0x00, 0x01, 0x00, 0x22, 0x00, 0x00, 0x00]);
    let n = (LCD_WIDTH * LCD_HEIGHT * 2) as u32 as u16;
    px[5..7].copy_from_slice(&n.to_be_bytes());
    for y in 0..LCD_HEIGHT {
        for x in 0..LCD_WIDTH {
            let (r, g, b) = if y < 20 {
                (4, 20, 28) // status bar
            } else if (30..30 + 16 * state.vars.len().min(12)).contains(&y)
                && x >= 10
                && x < 10
                    + 8 * state.vars[((y - 30) / 16).min(state.vars.len() - 1)]
                        .name
                        .len()
                && (y - 30) % 16 < 12
            {
                (31, 63, 31)
            } else {
                (
                    (x * 31 / LCD_WIDTH) as u16,
                    (y * 63 / LCD_HEIGHT) as u16,
                    16,
                )
            };
            let v: u16 = (r << 11) | (g << 5) | b;
            let i = 7 + (y * LCD_WIDTH + x) * 2;
            px[i..i + 2].copy_from_slice(&v.to_le_bytes());
        }
    }
    px
}

fn type_value(id: u16, t: u8) -> Value {
    Value {
        id,
        data: Some(vec![0xF0, 0x0F, 0x00, t]),
    }
}

fn attr_values(v: &SimVar, aids: &[u16]) -> Vec<Value> {
    aids.iter()
        .map(|&id| Value {
            id,
            data: match id {
                cmd::AID_VAR_SIZE => Some((v.data.len() as u32).to_be_bytes().to_vec()),
                cmd::AID_VAR_TYPE | cmd::AID_VAR_TYPE2 => type_value(id, v.type_id).data,
                cmd::AID_ARCHIVED => Some(vec![v.archived as u8]),
                cmd::AID_VAR_VERSION => Some(vec![0, 0, 0, v.version]),
                cmd::AID_LOCKED => Some(vec![(v.type_id == types::PROT_PROGRAM) as u8]),
                _ => None,
            },
        })
        .collect()
}

fn attr_type(attrs: &[cmd::Attr]) -> Option<u8> {
    attrs
        .iter()
        .find(|a| matches!(a.id, cmd::AID_VAR_TYPE | cmd::AID_VAR_TYPE2))
        .and_then(|a| a.data.get(3).copied())
}

/// Error code the simulator returns for unknown variables.
pub const ERR_NOT_FOUND: u16 = 0x0004;

pub struct Simulator<T: Transport> {
    link: Link<T>,
    pub state: Arc<Mutex<SimState>>,
    pending_rts: Option<cmd::Rts>,
}

impl<T: Transport> Simulator<T> {
    pub fn new(transport: T, state: Arc<Mutex<SimState>>) -> Self {
        Self {
            link: Link::new(transport, Role::Device),
            state,
            pending_rts: None,
        }
    }

    pub fn link(&mut self) -> &mut Link<T> {
        &mut self.link
    }

    /// Serve requests until the host goes away.
    pub fn run(mut self) -> Result<()> {
        loop {
            match self.step() {
                Ok(()) => {}
                Err(Error::Disconnected) => return Ok(()),
                Err(e) => return Err(e),
            }
        }
    }

    fn reply_error(&mut self, code: u16) -> Result<()> {
        self.link.send_vpkt(VPKT_ERROR, &cmd::error(code))
    }

    fn data_ack(&mut self) -> Result<()> {
        self.link.send_vpkt(VPKT_DATA_ACK, &[0x01])
    }

    pub fn step(&mut self) -> Result<()> {
        let pkt = self.link.recv_vpkt()?;
        match pkt.kind {
            VPKT_PING => self
                .link
                .send_vpkt(VPKT_MODE_SET, &[0x00, 0x00, 0x07, 0xD0]),
            VPKT_PARM_REQ => {
                let pids = cmd::parse_param_request(&pkt.data)?;
                let st = self.state.lock().unwrap();
                let vals: Vec<Value> = pids
                    .iter()
                    .map(|&id| Value {
                        id,
                        data: st.param(id),
                    })
                    .collect();
                drop(st);
                if pids == [cmd::PID_SCREENSHOT] {
                    // build_param_data would truncate the u16 size; send raw.
                    let st = self.state.lock().unwrap();
                    let screen = sim_screen(&st);
                    drop(st);
                    return self.link.send_vpkt(VPKT_PARM_DATA, &screen);
                }
                self.link
                    .send_vpkt(VPKT_PARM_DATA, &cmd::build_param_data(&vals))
            }
            VPKT_PARM_SET => {
                let id = u16::from_be_bytes([pkt.data[0], pkt.data[1]]);
                let ok = self.state.lock().unwrap().set_param(id, &pkt.data[4..]);
                if ok {
                    self.data_ack()
                } else {
                    self.reply_error(0x0012)
                }
            }
            VPKT_DIR_REQ => {
                let aids = cmd::parse_dirlist_request(&pkt.data)?;
                let vars = self.state.lock().unwrap().vars.clone();
                for v in &vars {
                    let hdr = cmd::build_var_header(b"", v.name.as_bytes(), &attr_values(v, &aids));
                    self.link.send_vpkt(VPKT_VAR_HDR, &hdr)?;
                }
                self.link.send_vpkt(VPKT_EOT, &[])
            }
            VPKT_RTS => {
                let rts = cmd::parse_rts(&pkt.data)?;
                let t = attr_type(&rts.attrs);
                let st = self.state.lock().unwrap();
                let exists_archived = st
                    .find(&rts.name, t)
                    .is_some_and(|i| st.vars[i].archived && st.vars[i].type_id != types::APP);
                drop(st);
                if exists_archived {
                    // Can't overwrite an archived variable in place.
                    return self.reply_error(0x0009);
                }
                self.pending_rts = Some(rts);
                self.data_ack()
            }
            VPKT_VAR_CNTS => {
                let Some(rts) = self.pending_rts.take() else {
                    return self.reply_error(0x000C);
                };
                let type_id = attr_type(&rts.attrs).unwrap_or(types::PROGRAM);
                let archived = rts
                    .attrs
                    .iter()
                    .find(|a| a.id == cmd::AID_ARCHIVED)
                    .is_some_and(|a| a.data.first().is_some_and(|&b| b != 0));
                let version = rts
                    .attrs
                    .iter()
                    .find(|a| a.id == cmd::AID_VAR_VERSION)
                    .and_then(|a| a.data.get(3).copied())
                    .unwrap_or(0);
                let var = SimVar {
                    name: String::from_utf8_lossy(&rts.name).into_owned(),
                    type_id,
                    archived,
                    version,
                    data: pkt.data,
                };
                let mut st = self.state.lock().unwrap();
                match st.find(var.name.as_bytes(), Some(type_id)) {
                    Some(i) => st.vars[i] = var,
                    None => st.vars.push(var),
                }
                drop(st);
                self.data_ack()
            }
            VPKT_EOT => Ok(()),
            VPKT_VAR_REQ => {
                let req = cmd::parse_var_request(&pkt.data)?;
                let t = attr_type(&req.attrs);
                let st = self.state.lock().unwrap();
                let found = st.find(&req.name, t).map(|i| st.vars[i].clone());
                drop(st);
                let Some(v) = found else {
                    return self.reply_error(ERR_NOT_FOUND);
                };
                let hdr =
                    cmd::build_var_header(b"", v.name.as_bytes(), &attr_values(&v, &req.aids));
                self.link.send_vpkt(VPKT_VAR_HDR, &hdr)?;
                self.link.send_vpkt(VPKT_VAR_CNTS, &v.data)
            }
            VPKT_MODIF_VAR => {
                let m = cmd::parse_var_modify(&pkt.data)?;
                let t = attr_type(&m.src_attrs);
                let mut st = self.state.lock().unwrap();
                let Some(i) = st.find(&m.src_name, t) else {
                    drop(st);
                    return self.reply_error(ERR_NOT_FOUND);
                };
                if m.dst_name.is_empty() && m.dst_attrs.is_empty() {
                    st.vars.remove(i);
                } else {
                    if !m.dst_name.is_empty() && m.dst_name != m.src_name {
                        st.vars[i].name = String::from_utf8_lossy(&m.dst_name).into_owned();
                    }
                    if let Some(a) = m.dst_attrs.iter().find(|a| a.id == cmd::AID_ARCHIVED) {
                        st.vars[i].archived = a.data.first().is_some_and(|&b| b != 0);
                    }
                }
                drop(st);
                self.data_ack()
            }
            VPKT_EXECUTE => match cmd::parse_execute(&pkt.data)? {
                cmd::Execute::Key(code) => {
                    let delay = {
                        let mut st = self.state.lock().unwrap();
                        st.keys.push(code);
                        st.key_delay_us
                    };
                    self.link.send_vpkt(VPKT_DELAY_ACK, &delay.to_be_bytes())?;
                    self.data_ack()
                }
                cmd::Execute::Run { name, .. } => {
                    self.state
                        .lock()
                        .unwrap()
                        .executed
                        .push(String::from_utf8_lossy(&name).into_owned());
                    self.data_ack()
                }
            },
            _ => self.reply_error(0x000C),
        }
    }
}

/// Start a simulator on a background thread; returns the host-side transport.
pub fn spawn(state: Arc<Mutex<SimState>>) -> ChannelTransport {
    let (host, dev) = channel_pair();
    std::thread::spawn(move || {
        let mut dev = dev;
        dev.set_timeout(std::time::Duration::from_secs(3600 * 24));
        let sim = Simulator::new(dev, state);
        if let Err(e) = sim.run() {
            eprintln!("simulator stopped: {e}");
        }
    });
    host
}
