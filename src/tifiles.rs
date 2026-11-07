//! TI file formats: `.8x?` variable files (single or group) and `.8ek`
//! flash apps, written the same way libtifiles writes them.

use crate::error::{Error, Result};
use crate::names;

pub const VAR_SIGNATURE: &[u8; 8] = b"**TI83F*";
pub const FLASH_SIGNATURE: &[u8; 8] = b"**TIFL**";
const VAR_SIG_TAIL: [u8; 3] = [0x1A, 0x0A, 0x00];
pub const DEVICE_TYPE_83P: u8 = 0x73;
/// Product ID of the TI-84 Plus CE (hw_id in flash files).
pub const PRODUCT_ID_TI84PCE: u8 = 0x13;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarEntry {
    pub type_id: u8,
    /// Tokenized name as stored in the file (no padding).
    pub raw_name: Vec<u8>,
    pub version: u8,
    pub archived: bool,
    pub data: Vec<u8>,
}

impl VarEntry {
    pub fn wire_name(&self) -> String {
        names::raw_to_wire(&self.raw_name, self.type_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarFile {
    pub comment: [u8; 42],
    pub entries: Vec<VarEntry>,
    /// False when the stored checksum didn't match (we still load the file).
    pub checksum_ok: bool,
}

/// libtifiles' `tifiles_comment_set_single`: "Single file dated <asctime>"
/// cut to 40 characters.
pub fn single_file_comment(now: chrono::DateTime<chrono::Local>) -> [u8; 42] {
    let s = format!("Single file dated {}", now.format("%a %b %e %H:%M:%S %Y\n"));
    let mut c = [0u8; 42];
    let b = s.as_bytes();
    let n = b.len().min(40);
    c[..n].copy_from_slice(&b[..n]);
    c
}

fn le16(d: &[u8], at: usize) -> Result<u16> {
    d.get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| Error::InvalidFile("file is truncated".into()))
}

/// Strip the NUL padding from a file's 8-byte name field. Tokenized names
/// keep their second byte even when it's 00: `5D 00` is L₁, `AA 00` is Str1.
fn trim_raw_name(field: &[u8]) -> Vec<u8> {
    let mut n = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    if matches!(
        field.first(),
        Some(0x3C | 0x5C | 0x5D | 0x5E | 0x60 | 0x61 | 0xAA)
    ) {
        n = n.max(2);
    }
    field[..n.min(field.len())].to_vec()
}

impl VarFile {
    pub fn single(entry: VarEntry, comment: [u8; 42]) -> Self {
        Self {
            comment,
            entries: vec![entry],
            checksum_ok: true,
        }
    }

    pub fn parse(d: &[u8]) -> Result<Self> {
        if d.len() < 57 || &d[..8] != VAR_SIGNATURE {
            return Err(Error::InvalidFile(
                "not a TI-83+/84+ variable file (missing **TI83F* signature)".into(),
            ));
        }
        let mut comment = [0u8; 42];
        comment.copy_from_slice(&d[11..53]);
        let data_len = le16(d, 53)? as usize;
        let section = d
            .get(55..55 + data_len)
            .ok_or_else(|| Error::InvalidFile("data section is truncated".into()))?;
        let stored_sum = le16(d, 55 + data_len).unwrap_or(0);
        let sum = section.iter().fold(0u16, |a, &b| a.wrapping_add(b as u16));

        let mut entries = Vec::new();
        let mut p = 0usize;
        while p < section.len() {
            let hdr_len = le16(section, p)? as usize;
            let size = le16(section, p + 2)? as usize;
            let type_id = *section
                .get(p + 4)
                .ok_or_else(|| Error::InvalidFile("entry header truncated".into()))?;
            let name = section
                .get(p + 5..p + 13)
                .ok_or_else(|| Error::InvalidFile("entry name truncated".into()))?;
            let raw_name = trim_raw_name(name);
            let (version, flag) = if hdr_len >= 13 {
                (section[p + 13], section[p + 14])
            } else {
                (0, 0)
            };
            let data_start = p + 2 + hdr_len + 2;
            let size2 = le16(section, p + 2 + hdr_len)? as usize;
            if size2 != size {
                return Err(Error::InvalidFile(format!(
                    "entry sizes disagree ({size} vs {size2})"
                )));
            }
            let data = section
                .get(data_start..data_start + size)
                .ok_or_else(|| Error::InvalidFile("entry data truncated".into()))?
                .to_vec();
            entries.push(VarEntry {
                type_id,
                raw_name,
                version,
                archived: flag & 0x80 != 0,
                data,
            });
            p = data_start + size;
        }
        if entries.is_empty() {
            return Err(Error::InvalidFile("file contains no variables".into()));
        }
        Ok(Self {
            comment,
            entries,
            checksum_ok: sum == stored_sum,
        })
    }

    /// Serialize like `ti8x_file_write_regular`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut section = Vec::new();
        for e in &self.entries {
            section.extend_from_slice(&0x000Du16.to_le_bytes());
            section.extend_from_slice(&(e.data.len() as u16).to_le_bytes());
            section.push(e.type_id);
            let mut name = [0u8; 8];
            let n = e.raw_name.len().min(8);
            name[..n].copy_from_slice(&e.raw_name[..n]);
            section.extend_from_slice(&name);
            section.push(e.version);
            section.push(if e.archived { 0x80 } else { 0x00 });
            section.extend_from_slice(&(e.data.len() as u16).to_le_bytes());
            section.extend_from_slice(&e.data);
        }
        let sum = section.iter().fold(0u16, |a, &b| a.wrapping_add(b as u16));
        let mut out = Vec::with_capacity(57 + section.len());
        out.extend_from_slice(VAR_SIGNATURE);
        out.extend_from_slice(&VAR_SIG_TAIL);
        out.extend_from_slice(&self.comment);
        out.extend_from_slice(&(section.len() as u16).to_le_bytes());
        out.extend_from_slice(&section);
        out.extend_from_slice(&sum.to_le_bytes());
        out
    }
}

/// A CE flash file (app or OS), flat data as stored after the 78-byte header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlashFile {
    pub revision_major: u8,
    pub revision_minor: u8,
    pub flags: u8,
    pub object_type: u8,
    pub revision_day: u8,
    pub revision_month: u8,
    pub revision_year: u16,
    pub name: String,
    pub device_type: u8,
    pub data_type: u8,
    pub hw_id: u8,
    pub data: Vec<u8>,
}

/// Find a field in TI's certificate-style TLV encoding (`tifiles_cert_field_find`).
pub fn cert_field_find(mut d: &[u8], id: u16) -> Option<&[u8]> {
    while d.len() >= 2 {
        let fid = u16::from_be_bytes([d[0], d[1]]);
        let (len, hdr) = match fid & 0x000F {
            0x0D => (*d.get(2)? as usize, 3),
            0x0E => (u16::from_be_bytes([*d.get(2)?, *d.get(3)?]) as usize, 4),
            0x0F => (
                u32::from_be_bytes(d.get(2..6)?.try_into().ok()?) as usize,
                6,
            ),
            n => (n as usize, 2),
        };
        if (fid & 0xFFF0) == id {
            return d.get(hdr..hdr + len);
        }
        d = d.get(hdr + len..)?;
    }
    None
}

impl FlashFile {
    pub fn parse(d: &[u8]) -> Result<Self> {
        if d.len() < 78 || &d[..8] != FLASH_SIGNATURE {
            return Err(Error::InvalidFile(
                "not a flash file (missing **TIFL** signature)".into(),
            ));
        }
        let name_len = (d[16] as usize).min(8);
        let header_name: String = d[17..17 + name_len]
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as char)
            .collect();
        let device_type = d[48];
        let data_type = d[49];
        let hw_id = d[73];
        let len = u32::from_le_bytes(d[74..78].try_into().unwrap()) as usize;
        let data = d
            .get(78..78 + len)
            .ok_or_else(|| Error::InvalidFile("flash data is truncated".into()))?
            .to_vec();
        if device_type != DEVICE_TYPE_83P {
            return Err(Error::InvalidFile(format!(
                "flash file is for device type {device_type:02X}, not a TI-83+/84+ family calculator"
            )));
        }
        if hw_id == 0 {
            return Err(Error::InvalidFile(
                "this is an Intel-hex TI-84+ (non-CE) flash file; the CE needs .8ek/.8eu files"
                    .into(),
            ));
        }
        if data_type == names::types::APP && data.first() != Some(&0x81) {
            return Err(Error::InvalidFile(
                "app data does not start with 0x81".into(),
            ));
        }
        // Like libtifiles, prefer the name from the app's own header.
        let name = if data_type == names::types::APP && data.len() >= 6 && data[1] == 0x0F {
            let app_type = (data[0] as u16) << 8;
            cert_field_find(&data[6..], app_type + 0x40)
                .map(|n| {
                    n.iter()
                        .take(8)
                        .take_while(|&&b| b != 0)
                        .map(|&b| b as char)
                        .collect()
                })
                .unwrap_or(header_name)
        } else {
            header_name
        };
        Ok(Self {
            revision_major: d[8],
            revision_minor: d[9],
            flags: d[10],
            object_type: d[11],
            revision_day: d[12],
            revision_month: d[13],
            revision_year: u16::from_le_bytes([d[14], d[15]]),
            name,
            device_type,
            data_type,
            hw_id,
            data,
        })
    }

    /// What `recv_flash_834pce` + `ti8x_file_write_flash` produce.
    pub fn from_received(name: &str, data_type: u8, data: Vec<u8>) -> Self {
        Self {
            revision_major: 0,
            revision_minor: 0,
            flags: 0,
            object_type: 0,
            revision_day: 0,
            revision_month: 0,
            revision_year: 0,
            name: name.into(),
            device_type: DEVICE_TYPE_83P,
            data_type,
            hw_id: PRODUCT_ID_TI84PCE,
            data,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(78 + self.data.len());
        out.extend_from_slice(FLASH_SIGNATURE);
        out.extend_from_slice(&[
            self.revision_major,
            self.revision_minor,
            self.flags,
            self.object_type,
            self.revision_day,
            self.revision_month,
        ]);
        out.extend_from_slice(&self.revision_year.to_le_bytes());
        let mut name = [0u8; 8];
        let nb = self.name.as_bytes();
        let n = nb.len().min(8);
        name[..n].copy_from_slice(&nb[..n]);
        out.push(n as u8);
        out.extend_from_slice(&name);
        out.extend_from_slice(&[0u8; 23]);
        out.push(self.device_type);
        out.push(self.data_type);
        out.extend_from_slice(&[0u8; 23]);
        out.push(self.hw_id);
        out.extend_from_slice(&(self.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.data);
        out
    }
}

/// Anything haccerlink knows how to send.
#[derive(Debug, Clone)]
pub enum TiFile {
    Vars(VarFile),
    Flash(FlashFile),
    Bundle(Bundle),
}

/// A zip of TI files: a TI Connect CE bundle (`.b84`/`.b83`, with METADATA
/// and _CHECKSUM) or a TiLP group (`.tig`, just the files).
#[derive(Debug, Clone)]
pub struct Bundle {
    /// TI's METADATA file, when present.
    pub metadata: Option<String>,
    /// `None` when there is no _CHECKSUM (e.g. .tig files).
    pub checksum_ok: Option<bool>,
    /// Members in archive order, with their names inside the zip.
    pub files: Vec<(String, TiFile)>,
}

/// One thing to transfer, after opening bundles and group files.
#[derive(Debug, Clone)]
pub enum Piece {
    Var(VarEntry),
    App(FlashFile),
    /// OS upgrades aren't supported; listed so callers can say so.
    Os(FlashFile),
}

const ZIP_MAGIC: &[u8] = b"PK\x03\x04";
/// Biggest CE bundles (OS + all apps) are ~2.5 MB; refuse anything absurd.
const MAX_BUNDLE_BYTES: u64 = 64 * 1024 * 1024;

fn zip_err(e: impl std::fmt::Display) -> Error {
    Error::InvalidFile(format!("bad bundle: {e}"))
}

impl Bundle {
    pub fn parse(d: &[u8]) -> Result<Self> {
        use std::io::Read;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(d)).map_err(zip_err)?;
        let mut metadata = None;
        let mut stated_sum = None;
        let mut crc_sum = 0u32;
        let mut total = 0u64;
        let mut files = Vec::new();
        for i in 0..zip.len() {
            let mut f = zip.by_index(i).map_err(zip_err)?;
            if f.is_dir() {
                continue;
            }
            let name = f.name().map_err(zip_err)?.into_owned();
            total += f.size();
            if total > MAX_BUNDLE_BYTES {
                return Err(Error::InvalidFile("bundle is unreasonably large".into()));
            }
            let crc = f.crc32();
            let mut data = Vec::with_capacity(f.size() as usize);
            // Reading to the end makes the zip crate verify the CRC.
            (&mut f)
                .take(MAX_BUNDLE_BYTES)
                .read_to_end(&mut data)
                .map_err(zip_err)?;
            match name.as_str() {
                "_CHECKSUM" => {
                    let text = String::from_utf8_lossy(&data);
                    stated_sum = Some(u32::from_str_radix(text.trim(), 16).map_err(|_| {
                        Error::InvalidFile(format!("bad bundle checksum {:?}", text.trim()))
                    })?);
                    continue;
                }
                "METADATA" => metadata = Some(String::from_utf8_lossy(&data).replace("\r\n", "\n")),
                _ => {
                    if data.starts_with(ZIP_MAGIC) {
                        return Err(Error::InvalidFile(format!(
                            "{name}: bundles can't contain bundles"
                        )));
                    }
                    let file = TiFile::parse(&data)
                        .map_err(|e| Error::InvalidFile(format!("{name} in bundle: {e}")))?;
                    files.push((name, file));
                }
            }
            // TI's _CHECKSUM: sum of every other member's CRC-32, METADATA included.
            crc_sum = crc_sum.wrapping_add(crc);
        }
        if files.is_empty() {
            return Err(Error::InvalidFile(
                "bundle contains no calculator files".into(),
            ));
        }
        Ok(Self {
            metadata,
            checksum_ok: stated_sum.map(|s| s == crc_sum),
            files,
        })
    }

    /// `bundle_target_device` from METADATA (e.g. "84CE").
    pub fn target_device(&self) -> Option<&str> {
        self.metadata
            .as_deref()?
            .lines()
            .find_map(|l| l.strip_prefix("bundle_target_device:").map(str::trim))
    }
}

impl TiFile {
    pub fn parse(d: &[u8]) -> Result<Self> {
        if d.starts_with(FLASH_SIGNATURE) {
            Ok(TiFile::Flash(FlashFile::parse(d)?))
        } else if d.starts_with(ZIP_MAGIC) {
            Ok(TiFile::Bundle(Bundle::parse(d)?))
        } else {
            Ok(TiFile::Vars(VarFile::parse(d)?))
        }
    }

    pub fn load(path: &std::path::Path) -> Result<Self> {
        Self::parse(&std::fs::read(path)?)
    }

    /// Everything to send, in order, plus warnings worth showing the user.
    pub fn pieces(&self) -> (Vec<Piece>, Vec<String>) {
        let mut pieces = Vec::new();
        let mut warnings = Vec::new();
        self.collect(&mut pieces, &mut warnings, None);
        (pieces, warnings)
    }

    fn collect(&self, pieces: &mut Vec<Piece>, warnings: &mut Vec<String>, member: Option<&str>) {
        let label = |what: &str| match member {
            Some(m) => format!("{m}: {what}"),
            None => what.to_string(),
        };
        match self {
            TiFile::Vars(v) => {
                if !v.checksum_ok {
                    warnings.push(label("bad checksum (sending anyway)"));
                }
                pieces.extend(v.entries.iter().cloned().map(Piece::Var));
            }
            TiFile::Flash(f) if f.data_type == names::types::OS => {
                pieces.push(Piece::Os(f.clone()))
            }
            TiFile::Flash(f) => pieces.push(Piece::App(f.clone())),
            TiFile::Bundle(b) => {
                if b.checksum_ok == Some(false) {
                    warnings.push(label("bundle checksum doesn't match its contents"));
                }
                if let Some(dev) = b.target_device().filter(|d| !d.contains("CE")) {
                    warnings.push(label(&format!("bundle is for a {dev}, not a CE")));
                }
                for (name, f) in &b.files {
                    f.collect(pieces, warnings, Some(name));
                }
            }
        }
    }
}

/// Short description of a piece for logs.
pub fn describe(p: &Piece) -> String {
    match p {
        Piece::Var(e) => format!(
            "{} ({}, {} bytes)",
            e.wire_name(),
            names::type_name(e.type_id),
            e.data.len()
        ),
        Piece::App(f) => format!("app {} ({} bytes)", f.name, f.data.len()),
        Piece::Os(f) => format!("OS {} ({} bytes)", f.name, f.data.len()),
    }
}

/// File name TiLP would choose for a received variable.
pub fn file_name_for(wire_name: &str, type_id: u8) -> String {
    let safe: String = wire_name
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect();
    format!("{safe}.{}", names::file_extension(type_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prog() -> VarEntry {
        VarEntry {
            type_id: 5,
            raw_name: b"HELLO".to_vec(),
            version: 0,
            archived: false,
            data: vec![0x03, 0x00, 0xDE, 0x2A, 0x48],
        }
    }

    #[test]
    fn var_file_roundtrip_and_checksum() {
        let f = VarFile::single(prog(), [0; 42]);
        let bytes = f.to_bytes();
        assert_eq!(&bytes[..11], b"**TI83F*\x1a\x0a\x00");
        assert_eq!(bytes.len(), 55 + 17 + 5 + 2);
        let g = VarFile::parse(&bytes).unwrap();
        assert!(g.checksum_ok);
        assert_eq!(g.entries, f.entries);
    }

    #[test]
    fn tokenized_names_keep_index_zero() {
        let mut e = prog();
        e.type_id = 0x01;
        e.raw_name = vec![0x5D, 0x00];
        let g = VarFile::parse(&VarFile::single(e, [0; 42]).to_bytes()).unwrap();
        assert_eq!(g.entries[0].raw_name, [0x5D, 0x00]);
        assert_eq!(g.entries[0].wire_name(), "L₁");
    }

    #[test]
    fn comment_format() {
        use chrono::TimeZone;
        let t = chrono::Local
            .with_ymd_and_hms(2026, 11, 6, 9, 5, 3)
            .unwrap();
        let c = single_file_comment(t);
        assert_eq!(&c[..40], b"Single file dated Fri Nov  6 09:05:03 20");
        assert_eq!(c[40], 0);
    }

    #[test]
    fn group_file() {
        let mut b = prog();
        b.raw_name = b"B".to_vec();
        b.archived = true;
        let f = VarFile {
            comment: [0; 42],
            entries: vec![prog(), b],
            checksum_ok: true,
        };
        let g = VarFile::parse(&f.to_bytes()).unwrap();
        assert_eq!(g.entries.len(), 2);
        assert!(g.entries[1].archived);
    }

    #[test]
    fn flash_roundtrip() {
        // app header: 81 0F <len32>, then the CE name field 8140, length 4
        let mut data = vec![
            0x81, 0x0F, 0, 0, 0, 0x10, 0x81, 0x44, b'D', b'E', b'M', b'O',
        ];
        data.resize(32, 0xFF);
        let f = FlashFile::from_received("DEMO", 0x24, data);
        let g = FlashFile::parse(&f.to_bytes()).unwrap();
        assert_eq!(g, f);
        assert_eq!(g.name, "DEMO");
    }

    fn make_bundle(checksum: Option<u32>) -> Vec<u8> {
        use std::io::Write;
        let mut members: Vec<(&str, Vec<u8>)> = vec![
            (
                "METADATA",
                b"bundle_identifier:TI Bundle\r\nbundle_target_device:84CE\r\n".to_vec(),
            ),
            ("HELLO.8xp", VarFile::single(prog(), [0; 42]).to_bytes()),
        ];
        let mut os = FlashFile::from_received("basecode", names::types::OS, vec![0x80, 0x0F, 0, 0]);
        os.hw_id = PRODUCT_ID_TI84PCE;
        members.push(("os.8eu", os.to_bytes()));
        let sum = members.iter().fold(0u32, |a, (_, d)| {
            let mut h = flate2::Crc::new();
            h.update(d);
            a.wrapping_add(h.sum())
        });
        let mut out = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut out);
            let opts = zip::write::SimpleFileOptions::default();
            for (name, data) in &members {
                z.start_file(*name, opts).unwrap();
                z.write_all(data).unwrap();
            }
            z.start_file("_CHECKSUM", opts).unwrap();
            write!(z, "{:08x}\r\n", checksum.unwrap_or(sum)).unwrap();
            z.finish().unwrap();
        }
        out.into_inner()
    }

    #[test]
    fn bundle_parses_and_verifies_checksum() {
        let TiFile::Bundle(b) = TiFile::parse(&make_bundle(None)).unwrap() else {
            panic!("not a bundle");
        };
        assert_eq!(b.checksum_ok, Some(true));
        assert_eq!(b.target_device(), Some("84CE"));
        let (pieces, warnings) = TiFile::Bundle(b).pieces();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(matches!(&pieces[..], [Piece::Var(v), Piece::Os(_)] if v.raw_name == b"HELLO"));
    }

    #[test]
    fn bundle_with_wrong_checksum_warns() {
        let f = TiFile::parse(&make_bundle(Some(0x1234))).unwrap();
        let (_, warnings) = f.pieces();
        assert!(warnings.iter().any(|w| w.contains("checksum")));
    }
}
