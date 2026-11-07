//! Virtual packet payloads (`dusb_cmd.c` in libticalcs).
//!
//! Builders produce exactly the bytes libticalcs would put in the packet;
//! parsers cover both what the calculator sends us and (for the simulator)
//! what a host sends the calculator.

use crate::error::{Error, Result};

// Parameter IDs.
pub const PID_PRODUCT_NUMBER: u16 = 0x0001;
pub const PID_PRODUCT_NAME: u16 = 0x0002;
pub const PID_MAIN_PART_ID: u16 = 0x0003;
pub const PID_HW_VERSION: u16 = 0x0004;
pub const PID_FULL_ID: u16 = 0x0005;
pub const PID_LANGUAGE_ID: u16 = 0x0006;
pub const PID_SUBLANG_ID: u16 = 0x0007;
pub const PID_DEVICE_TYPE: u16 = 0x0008;
pub const PID_BOOT_VERSION: u16 = 0x0009;
pub const PID_OS_MODE: u16 = 0x000A;
pub const PID_OS_VERSION: u16 = 0x000B;
pub const PID_PHYS_RAM: u16 = 0x000C;
pub const PID_USER_RAM: u16 = 0x000D;
pub const PID_FREE_RAM: u16 = 0x000E;
pub const PID_PHYS_FLASH: u16 = 0x000F;
pub const PID_USER_FLASH: u16 = 0x0010;
pub const PID_FREE_FLASH: u16 = 0x0011;
pub const PID_USER_PAGES: u16 = 0x0012;
pub const PID_FREE_PAGES: u16 = 0x0013;
pub const PID_HAS_SCREEN: u16 = 0x0019;
pub const PID_COLOR_AVAILABLE: u16 = 0x001B;
pub const PID_BITS_PER_PIXEL: u16 = 0x001D;
pub const PID_LCD_WIDTH: u16 = 0x001E;
pub const PID_LCD_HEIGHT: u16 = 0x001F;
pub const PID_SCREENSHOT: u16 = 0x0022;
pub const PID_CLASSIC_CLK_SUPPORT: u16 = 0x0023;
pub const PID_CLK_ON: u16 = 0x0024;
pub const PID_CLK_SEC_SINCE_1997: u16 = 0x0025;
pub const PID_CLK_DATE_FMT: u16 = 0x0027;
pub const PID_CLK_TIME_FMT: u16 = 0x0028;
pub const PID_BATTERY: u16 = 0x002D;
pub const PID_USER_DATA_1: u16 = 0x0030;
pub const PID_FLASHAPPS: u16 = 0x0031;
pub const PID_USER_DATA_2: u16 = 0x0035;
pub const PID_MAIN_PART_ID_STRING: u16 = 0x0036;
pub const PID_HOMESCREEN: u16 = 0x0037;
pub const PID_BUSY: u16 = 0x0038;
pub const PID_SCREEN_SPLIT: u16 = 0x0039;
pub const PID_NEW_CLK_SUPPORT: u16 = 0x003A;
pub const PID_CLK_SECONDS: u16 = 0x003B;
pub const PID_CLK_MINUTES: u16 = 0x003C;
pub const PID_CLK_HOURS: u16 = 0x003D;
pub const PID_CLK_DAY: u16 = 0x003E;
pub const PID_CLK_MONTH: u16 = 0x003F;
pub const PID_CLK_YEAR: u16 = 0x0040;
pub const PID_ANS: u16 = 0x0046;
pub const PID_OS_BUILD_NUMBER: u16 = 0x0048;
pub const PID_BOOT_BUILD_NUMBER: u16 = 0x0049;
pub const PID_EXACT_MATH: u16 = 0x004B;
pub const PID_BOOT_HASH: u16 = 0x004C;
pub const PID_OS_HASH: u16 = 0x004D;
pub const PID_PTT_MODE_SET: u16 = 0x004F;
pub const PID_OS_VERSION_STRING: u16 = 0x0052;
pub const PID_BOOT_VERSION_STRING: u16 = 0x0053;
pub const PID_PTT_MODE_STATE: u16 = 0x0054;
pub const PID_PTT_MODE_FEATURES: u16 = 0x0055;
pub const PID_STOPWATCH_START: u16 = 0x0059;
pub const PID_STOPWATCH_VALUE1: u16 = 0x005B;
pub const PID_STOPWATCH_VALUE2: u16 = 0x005C;

// Attribute IDs.
pub const AID_VAR_SIZE: u16 = 0x01;
pub const AID_VAR_TYPE: u16 = 0x02;
pub const AID_ARCHIVED: u16 = 0x03;
pub const AID_UNKNOWN_04: u16 = 0x04;
pub const AID_4APPVAR: u16 = 0x05;
pub const AID_VAR_VERSION: u16 = 0x08;
pub const AID_VAR_TYPE2: u16 = 0x11;
pub const AID_ARCHIVED2: u16 = 0x13;
pub const AID_LOCKED: u16 = 0x41;
pub const AID_UNKNOWN_42: u16 = 0x42;
pub const AID_BACKUP_HEADER: u16 = 0xFFFE;

// Remote control actions.
pub const EID_PRGM: u8 = 0x00;
pub const EID_ASM: u8 = 0x01;
pub const EID_APP: u8 = 0x02;
pub const EID_KEY: u8 = 0x03;

// RTS mode flags.
pub const RTS_SILENT: u8 = 0x01;
pub const RTS_NON_SILENT: u8 = 0x02;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeSet(pub u16, pub u16, pub u16, pub u16, pub u16);

pub const MODE_STARTUP: ModeSet = ModeSet(1, 1, 0, 0, 0x07d0);
pub const MODE_BASIC: ModeSet = ModeSet(2, 1, 0, 0, 0x07d0);
pub const MODE_NORMAL: ModeSet = ModeSet(3, 1, 0, 0, 0x07d0);

/// An attribute as sent by the host (id + raw bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attr {
    pub id: u16,
    pub data: Vec<u8>,
}

impl Attr {
    pub fn new(id: u16, data: impl Into<Vec<u8>>) -> Self {
        Self {
            id,
            data: data.into(),
        }
    }
}

/// A parameter or attribute value as returned by the calculator. `data` is
/// `None` when the calculator refused it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Value {
    pub id: u16,
    pub data: Option<Vec<u8>>,
}

impl Value {
    pub fn ok(&self) -> bool {
        self.data.is_some()
    }
    pub fn bytes(&self) -> &[u8] {
        self.data.as_deref().unwrap_or(&[])
    }
    pub fn be_uint(&self) -> Option<u64> {
        let d = self.data.as_ref()?;
        if d.is_empty() || d.len() > 8 {
            return None;
        }
        Some(d.iter().fold(0u64, |acc, &b| (acc << 8) | b as u64))
    }
}

// ---------------------------------------------------------------- builders

pub fn mode_set(m: ModeSet) -> Vec<u8> {
    [m.0, m.1, m.2, m.3, m.4]
        .iter()
        .flat_map(|v| v.to_be_bytes())
        .collect()
}

pub fn os_begin(size: u32) -> Vec<u8> {
    let mut v = vec![0u8; 7];
    v.extend_from_slice(&size.to_be_bytes());
    v
}

pub fn os_data_834pce(addr: u32, data: &[u8]) -> Vec<u8> {
    let mut v = addr.to_le_bytes().to_vec();
    v.extend_from_slice(data);
    v
}

pub fn param_request(pids: &[u16]) -> Vec<u8> {
    let mut v = (pids.len() as u16).to_be_bytes().to_vec();
    for p in pids {
        v.extend_from_slice(&p.to_be_bytes());
    }
    v
}

pub fn dirlist_request(aids: &[u16]) -> Vec<u8> {
    let mut v = (aids.len() as u32).to_be_bytes().to_vec();
    for a in aids {
        v.extend_from_slice(&a.to_be_bytes());
    }
    v.extend_from_slice(&[0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x01]);
    v
}

fn push_folder(v: &mut Vec<u8>, folder: &[u8]) {
    if folder.is_empty() {
        v.push(0);
    } else {
        push_name(v, folder);
    }
}

fn push_name(v: &mut Vec<u8>, name: &[u8]) {
    v.push(name.len() as u8);
    v.extend_from_slice(name);
    v.push(0);
}

fn push_attrs(v: &mut Vec<u8>, attrs: &[Attr]) {
    v.extend_from_slice(&(attrs.len() as u16).to_be_bytes());
    for a in attrs {
        v.extend_from_slice(&a.id.to_be_bytes());
        v.extend_from_slice(&(a.data.len() as u16).to_be_bytes());
        v.extend_from_slice(&a.data);
    }
}

pub fn rts(folder: &[u8], name: &[u8], size: u32, attrs: &[Attr], mode: u8) -> Vec<u8> {
    let mut v = Vec::new();
    push_folder(&mut v, folder);
    push_name(&mut v, name);
    v.extend_from_slice(&size.to_be_bytes());
    v.push(mode);
    push_attrs(&mut v, attrs);
    v
}

pub fn var_request(folder: &[u8], name: &[u8], aids: &[u16], attrs: &[Attr]) -> Vec<u8> {
    let mut v = Vec::new();
    push_folder(&mut v, folder);
    push_name(&mut v, name);
    v.extend_from_slice(&[0x01, 0xFF, 0xFF, 0xFF, 0xFF]);
    v.extend_from_slice(&(aids.len() as u16).to_be_bytes());
    for a in aids {
        v.extend_from_slice(&a.to_be_bytes());
    }
    push_attrs(&mut v, attrs);
    v.extend_from_slice(&[0x00, 0x00]);
    v
}

pub fn param_set(id: u16, data: &[u8]) -> Vec<u8> {
    let mut v = id.to_be_bytes().to_vec();
    v.extend_from_slice(&(data.len() as u16).to_be_bytes());
    v.extend_from_slice(data);
    v
}

/// `dusb_cmd_s_var_modify`. An empty `dst_name` with no attributes deletes.
pub fn var_modify(
    src_folder: &[u8],
    src_name: &[u8],
    src_attrs: &[Attr],
    dst_folder: &[u8],
    dst_name: &[u8],
    dst_attrs: &[Attr],
) -> Vec<u8> {
    let mut v = Vec::new();
    push_folder(&mut v, src_folder);
    push_name(&mut v, src_name);
    push_attrs(&mut v, src_attrs);
    v.push(0x01);
    push_folder(&mut v, dst_folder);
    if dst_name.is_empty() {
        v.push(0);
    } else {
        push_name(&mut v, dst_name);
    }
    push_attrs(&mut v, dst_attrs);
    v
}

/// `dusb_cmd_s_execute` for TI-Z80/eZ80 models (the folder is never sent).
pub fn execute(name: &[u8], action: u8, args: &[u8], keycode: u16) -> Vec<u8> {
    let mut v = vec![0u8]; // strlen(folder), always "" here
    if name.is_empty() {
        v.push(0);
    } else {
        push_name(&mut v, name);
    }
    v.push(action);
    v.extend_from_slice(args);
    if action == EID_KEY {
        // eZ80 wants single-byte codes big-endian; two-byte codes go
        // little-endian (keyExtend first, kbdKey second).
        if keycode >> 8 == 0 {
            v.extend_from_slice(&keycode.to_be_bytes());
        } else {
            v.extend_from_slice(&keycode.to_le_bytes());
        }
    }
    v
}

pub fn error(code: u16) -> Vec<u8> {
    code.to_be_bytes().to_vec()
}

// ---------------------------------------------------------------- parsers

struct Reader<'a> {
    d: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(d: &'a [u8]) -> Self {
        Self { d, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.d.len() {
            return Err(Error::InvalidPacket(format!(
                "payload truncated: wanted {n} bytes at offset {} of {}",
                self.pos,
                self.d.len()
            )));
        }
        let s = &self.d[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes(b.try_into().unwrap()))
    }
    /// `[len][bytes][NUL]`, or just `[0]` for an empty string.
    fn name(&mut self) -> Result<Vec<u8>> {
        let len = self.u8()? as usize;
        if len == 0 {
            return Ok(Vec::new());
        }
        let s = self.take(len + 1)?;
        Ok(s[..len].to_vec())
    }
    fn values(&mut self, n: usize) -> Result<Vec<Value>> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let id = self.u16()?;
            let ok = self.u8()? == 0;
            let data = if ok {
                let size = self.u16()? as usize;
                Some(self.take(size)?.to_vec())
            } else {
                None
            };
            out.push(Value { id, data });
        }
        Ok(out)
    }
    fn attrs(&mut self) -> Result<Vec<Attr>> {
        let n = self.u16()? as usize;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let id = self.u16()?;
            let size = self.u16()? as usize;
            out.push(Attr::new(id, self.take(size)?));
        }
        Ok(out)
    }
    fn rest(&mut self) -> &'a [u8] {
        let s = &self.d[self.pos..];
        self.pos = self.d.len();
        s
    }
}

/// Parameter Data (0x0008).
pub fn parse_param_data(data: &[u8]) -> Result<Vec<Value>> {
    let mut r = Reader::new(data);
    let n = r.u16()? as usize;
    r.values(n)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarHeader {
    pub folder: Vec<u8>,
    pub name: Vec<u8>,
    pub attrs: Vec<Value>,
}

impl VarHeader {
    pub fn attr(&self, id: u16) -> Option<&Value> {
        self.attrs.iter().find(|a| a.id == id && a.ok())
    }
}

/// Variable Header (0x000A).
pub fn parse_var_header(data: &[u8]) -> Result<VarHeader> {
    let mut r = Reader::new(data);
    let folder = r.name()?;
    let name = r.name()?;
    let n = r.u16()? as usize;
    let attrs = r.values(n)?;
    Ok(VarHeader {
        folder,
        name,
        attrs,
    })
}

pub fn build_var_header(folder: &[u8], name: &[u8], attrs: &[Value]) -> Vec<u8> {
    let mut v = Vec::new();
    push_folder(&mut v, folder);
    push_name(&mut v, name);
    push_values(&mut v, attrs);
    v
}

fn push_values(v: &mut Vec<u8>, vals: &[Value]) {
    v.extend_from_slice(&(vals.len() as u16).to_be_bytes());
    for a in vals {
        v.extend_from_slice(&a.id.to_be_bytes());
        match &a.data {
            Some(d) => {
                v.push(0);
                v.extend_from_slice(&(d.len() as u16).to_be_bytes());
                v.extend_from_slice(d);
            }
            None => v.push(1),
        }
    }
}

pub fn build_param_data(vals: &[Value]) -> Vec<u8> {
    let mut v = Vec::new();
    push_values(&mut v, vals);
    v
}

// ---- host -> calculator parsers (used by the simulator) ----

pub fn parse_param_request(data: &[u8]) -> Result<Vec<u16>> {
    let mut r = Reader::new(data);
    let n = r.u16()? as usize;
    (0..n).map(|_| r.u16()).collect()
}

pub fn parse_dirlist_request(data: &[u8]) -> Result<Vec<u16>> {
    let mut r = Reader::new(data);
    let n = r.u32()? as usize;
    (0..n).map(|_| r.u16()).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rts {
    pub folder: Vec<u8>,
    pub name: Vec<u8>,
    pub size: u32,
    pub mode: u8,
    pub attrs: Vec<Attr>,
}

pub fn parse_rts(data: &[u8]) -> Result<Rts> {
    let mut r = Reader::new(data);
    Ok(Rts {
        folder: r.name()?,
        name: r.name()?,
        size: r.u32()?,
        mode: r.u8()?,
        attrs: r.attrs()?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarRequest {
    pub folder: Vec<u8>,
    pub name: Vec<u8>,
    pub aids: Vec<u16>,
    pub attrs: Vec<Attr>,
}

pub fn parse_var_request(data: &[u8]) -> Result<VarRequest> {
    let mut r = Reader::new(data);
    let folder = r.name()?;
    let name = r.name()?;
    r.take(5)?;
    let n = r.u16()? as usize;
    let aids = (0..n).map(|_| r.u16()).collect::<Result<_>>()?;
    let attrs = r.attrs()?;
    Ok(VarRequest {
        folder,
        name,
        aids,
        attrs,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Modify {
    pub src_name: Vec<u8>,
    pub src_attrs: Vec<Attr>,
    pub dst_name: Vec<u8>,
    pub dst_attrs: Vec<Attr>,
}

pub fn parse_var_modify(data: &[u8]) -> Result<Modify> {
    let mut r = Reader::new(data);
    let _src_folder = r.name()?;
    let src_name = r.name()?;
    let src_attrs = r.attrs()?;
    r.u8()?;
    let _dst_folder = r.name()?;
    let dst_name = r.name()?;
    let dst_attrs = r.attrs()?;
    Ok(Modify {
        src_name,
        src_attrs,
        dst_name,
        dst_attrs,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Execute {
    Key(u16),
    Run {
        action: u8,
        name: Vec<u8>,
        args: Vec<u8>,
    },
}

pub fn parse_execute(data: &[u8]) -> Result<Execute> {
    let mut r = Reader::new(data);
    r.u8()?; // folder length, always 0 on the CE
    let name = r.name()?;
    let action = r.u8()?;
    let rest = r.rest();
    if action == EID_KEY {
        if rest.len() < 2 {
            return Err(Error::InvalidPacket("key press without keycode".into()));
        }
        // Mirror of the builder: [0, k] for one-byte codes, else little-endian.
        let code = if rest[0] == 0 {
            rest[1] as u16
        } else {
            u16::from_le_bytes([rest[0], rest[1]])
        };
        Ok(Execute::Key(code))
    } else {
        Ok(Execute::Run {
            action,
            name,
            args: rest.to_vec(),
        })
    }
}

/// Human readable parameter names (libticalcs' `dusb_cmd_param_type2name`).
pub fn param_name(id: u16) -> &'static str {
    match id {
        0x0000 => "",
        PID_PRODUCT_NUMBER => "Product number",
        PID_PRODUCT_NAME => "Product name",
        PID_MAIN_PART_ID => "Main part ID",
        PID_HW_VERSION => "Hardware version",
        PID_FULL_ID => "Full ID",
        PID_LANGUAGE_ID => "Language ID",
        PID_SUBLANG_ID => "Sub-language ID",
        PID_DEVICE_TYPE => "Device type",
        PID_BOOT_VERSION => "Boot version",
        PID_OS_MODE => "OS mode",
        PID_OS_VERSION => "OS version",
        PID_PHYS_RAM => "Physical RAM",
        PID_USER_RAM => "User RAM",
        PID_FREE_RAM => "Free RAM",
        PID_PHYS_FLASH => "Physical Flash",
        PID_USER_FLASH => "User Flash",
        PID_FREE_FLASH => "Free Flash",
        PID_USER_PAGES => "User pages",
        PID_FREE_PAGES => "Free pages",
        PID_HAS_SCREEN => "Has screen",
        PID_COLOR_AVAILABLE => "Color is available",
        PID_BITS_PER_PIXEL => "Bits per pixel",
        PID_LCD_WIDTH => "LCD width",
        PID_LCD_HEIGHT => "LCD height",
        PID_SCREENSHOT => "Screenshot",
        PID_CLASSIC_CLK_SUPPORT => "Classic clock supported",
        PID_CLK_ON => "Clock ON",
        PID_CLK_SEC_SINCE_1997 => "Clock sec since 1997",
        PID_CLK_DATE_FMT => "Clock date format",
        PID_CLK_TIME_FMT => "Clock time format",
        PID_BATTERY => "Battery level",
        PID_USER_DATA_1 => "User data area 1",
        PID_FLASHAPPS => "FlashApps",
        PID_USER_DATA_2 => "User data area 2",
        PID_MAIN_PART_ID_STRING => "Main part ID (as string)",
        PID_HOMESCREEN => "Home screen",
        PID_BUSY => "Busy",
        PID_SCREEN_SPLIT => "Screen split mode",
        PID_NEW_CLK_SUPPORT => "New clock supported",
        PID_CLK_SECONDS => "Clock seconds",
        PID_CLK_MINUTES => "Clock minutes",
        PID_CLK_HOURS => "Clock hours",
        PID_CLK_DAY => "Clock day",
        PID_CLK_MONTH => "Clock month",
        PID_CLK_YEAR => "Clock year",
        PID_ANS => "Ans contents",
        PID_OS_BUILD_NUMBER => "OS build number",
        PID_BOOT_BUILD_NUMBER => "Boot build number",
        PID_EXACT_MATH => "Exact math engine",
        PID_BOOT_HASH => "Boot hash",
        PID_OS_HASH => "OS hash",
        PID_PTT_MODE_SET => "PTT mode set",
        PID_OS_VERSION_STRING => "OS version (as string)",
        PID_BOOT_VERSION_STRING => "Boot version (as string)",
        PID_PTT_MODE_STATE => "PTT mode state",
        PID_PTT_MODE_FEATURES => "PTT mode features",
        PID_STOPWATCH_START => "Stopwatch start",
        PID_STOPWATCH_VALUE1 => "Stopwatch value 1",
        PID_STOPWATCH_VALUE2 => "Stopwatch value 2",
        _ => "unknown: not listed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_set_normal_matches_libticalcs() {
        assert_eq!(
            mode_set(MODE_NORMAL),
            [0x00, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x07, 0xD0]
        );
    }

    #[test]
    fn rts_layout() {
        let attrs = [
            Attr::new(AID_VAR_TYPE, [0xF0, 0x07, 0x00, 0x05]),
            Attr::new(AID_ARCHIVED, [0x00]),
            Attr::new(AID_VAR_VERSION, [0, 0, 0, 0]),
        ];
        let v = rts(b"", b"ABC", 0x10, &attrs, RTS_SILENT);
        #[rustfmt::skip]
        let expected = [
            0x00,                               // no folder
            0x03, b'A', b'B', b'C', 0x00,       // name
            0x00, 0x00, 0x00, 0x10,             // size
            0x01,                               // silent
            0x00, 0x03,                         // 3 attrs
            0x00, 0x02, 0x00, 0x04, 0xF0, 0x07, 0x00, 0x05,
            0x00, 0x03, 0x00, 0x01, 0x00,
            0x00, 0x08, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(v, expected);
        let p = parse_rts(&v).unwrap();
        assert_eq!(p.name, b"ABC");
        assert_eq!(p.size, 0x10);
        assert_eq!(p.attrs, attrs);
    }

    #[test]
    fn delete_layout() {
        let v = var_modify(
            b"",
            b"A",
            &[Attr::new(AID_VAR_TYPE2, [0xF0, 0x0B, 0x00, 0x05])],
            b"",
            b"",
            &[],
        );
        assert_eq!(
            v,
            [
                0x00, 0x01, b'A', 0x00, 0x00, 0x01, 0x00, 0x11, 0x00, 0x04, 0xF0, 0x0B, 0x00, 0x05,
                0x01, 0x00, 0x00, 0x00, 0x00
            ]
        );
        let m = parse_var_modify(&v).unwrap();
        assert_eq!(m.src_name, b"A");
        assert!(m.dst_name.is_empty());
    }

    #[test]
    fn execute_key_byte_order() {
        // single-byte key: big endian
        assert_eq!(execute(b"", EID_KEY, b"", 0x0009), [0, 0, 3, 0x00, 0x09]);
        // two-byte key: little endian
        assert_eq!(execute(b"", EID_KEY, b"", 0xFC05), [0, 0, 3, 0x05, 0xFC]);
        assert_eq!(
            parse_execute(&execute(b"", EID_KEY, b"", 0xFC05)).unwrap(),
            Execute::Key(0xFC05)
        );
    }

    #[test]
    fn var_header_roundtrip() {
        let vals = vec![
            Value {
                id: AID_VAR_SIZE,
                data: Some(vec![0, 0, 0, 5]),
            },
            Value {
                id: AID_LOCKED,
                data: None,
            },
        ];
        let b = build_var_header(b"", b"FOO", &vals);
        let h = parse_var_header(&b).unwrap();
        assert_eq!(h.name, b"FOO");
        assert_eq!(h.attrs, vals);
    }
}
