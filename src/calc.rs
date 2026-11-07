//! High-level calculator operations, ported from libticalcs' calc_84p.c
//! (the `calc_84pce_usb` function table) so the packets match TiLP's.

use crate::dusb::cmd::{self, Attr, MODE_NORMAL, Value};
use crate::dusb::*;
use crate::error::{Error, Result};
use crate::names::{self, types};
use crate::tifiles::{FlashFile, VarEntry, VarFile};
use crate::transport::Transport;
use std::time::Duration;

pub const LCD_WIDTH: usize = 320;
pub const LCD_HEIGHT: usize = 240;

/// One entry of the calculator's directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalcVar {
    /// Name as the calculator reports it (TI-UTF-8).
    pub name: String,
    pub type_id: u8,
    pub size: u32,
    pub archived: bool,
}

impl CalcVar {
    pub fn is_app(&self) -> bool {
        self.type_id == types::APP
    }
}

#[derive(Debug, Clone, Default)]
pub struct CalcInfo {
    pub product_name: Option<String>,
    pub main_calc_id: Option<String>,
    pub product_id: Option<u8>,
    pub hw_version: Option<u16>,
    pub os_version: Option<String>,
    pub boot_version: Option<String>,
    pub run_level: Option<u8>,
    pub device_type: Option<u8>,
    pub language_id: Option<u8>,
    pub sub_lang_id: Option<u8>,
    pub ram_phys: Option<u64>,
    pub ram_user: Option<u64>,
    pub ram_free: Option<u64>,
    pub flash_phys: Option<u64>,
    pub flash_user: Option<u64>,
    pub flash_free: Option<u64>,
    pub lcd_width: Option<u16>,
    pub lcd_height: Option<u16>,
    pub bits_per_pixel: Option<u8>,
    pub color_screen: Option<u8>,
    pub battery: Option<u8>,
    pub exact_math: Option<u8>,
}

impl CalcInfo {
    /// Model name derived the way libticalcs does it.
    pub fn model(&self) -> &'static str {
        match self.product_id {
            Some(0x0A) => "TI-84 Plus",
            Some(0x0B) => "TI-82 Advanced",
            Some(0x0F) => "TI-84 Plus C Silver Edition",
            Some(0x13) if self.exact_math.unwrap_or(0) != 0 => "TI-83 Premium CE",
            Some(0x13) => "TI-84 Plus CE",
            _ => "unknown TI-84 Plus family calculator",
        }
    }

    pub fn lines(&self) -> Vec<(String, String)> {
        let mut v = Vec::new();
        let mut put = |k: &str, val: Option<String>| {
            if let Some(val) = val {
                v.push((k.to_string(), val));
            }
        };
        put("Model", Some(self.model().into()));
        put("Product name", self.product_name.clone());
        put("Calculator ID", self.main_calc_id.clone());
        put("Hardware version", self.hw_version.map(|h| h.to_string()));
        put("OS version", self.os_version.clone());
        put("Boot version", self.boot_version.clone());
        put("Run level", self.run_level.map(|r| r.to_string()));
        put(
            "Language",
            self.language_id
                .map(|l| format!("{l} / {}", self.sub_lang_id.unwrap_or(0))),
        );
        put("RAM free", self.ram_free.map(|f| fmt_mem(f, self.ram_user)));
        put(
            "Flash free",
            self.flash_free.map(|f| fmt_mem(f, self.flash_user)),
        );
        put("Battery", self.battery.map(|b| format!("{b}")));
        put(
            "Screen",
            self.lcd_width.map(|w| {
                format!(
                    "{w}x{} {}bpp{}",
                    self.lcd_height.unwrap_or(0),
                    self.bits_per_pixel.unwrap_or(0),
                    if self.color_screen.unwrap_or(0) != 0 {
                        " color"
                    } else {
                        ""
                    }
                )
            }),
        );
        v
    }
}

fn fmt_mem(free: u64, total: Option<u64>) -> String {
    match total {
        Some(t) => format!("{free} of {t} bytes"),
        None => format!("{free} bytes"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    /// 1 = M/D/Y, 2 = D/M/Y, 3 = Y/M/D (libticalcs' encoding).
    pub date_format: u8,
    /// 12 or 24.
    pub time_format: u8,
    pub on: bool,
}

pub struct Screenshot {
    pub width: usize,
    pub height: usize,
    /// RGB565, little endian, row-major.
    pub rgb565: Vec<u8>,
}

impl Screenshot {
    pub fn pixel(&self, x: usize, y: usize) -> (u8, u8, u8) {
        let i = (y * self.width + x) * 2;
        let v = u16::from_le_bytes([self.rgb565[i], self.rgb565[i + 1]]);
        let r = ((v >> 11) & 0x1F) as u8;
        let g = ((v >> 5) & 0x3F) as u8;
        let b = (v & 0x1F) as u8;
        (
            (r << 3) | (r >> 2),
            (g << 2) | (g >> 4),
            (b << 3) | (b >> 2),
        )
    }

    pub fn to_rgb8(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.width * self.height * 3);
        for y in 0..self.height {
            for x in 0..self.width {
                let (r, g, b) = self.pixel(x, y);
                out.extend_from_slice(&[r, g, b]);
            }
        }
        out
    }

    pub fn write_png(&self, path: &std::path::Path) -> Result<()> {
        let file = std::fs::File::create(path)?;
        let mut enc = png::Encoder::new(
            std::io::BufWriter::new(file),
            self.width as u32,
            self.height as u32,
        );
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc
            .write_header()
            .map_err(|e| Error::Other(format!("png: {e}")))?;
        w.write_image_data(&self.to_rgb8())
            .map_err(|e| Error::Other(format!("png: {e}")))?;
        Ok(())
    }
}

fn type_attr(id: u16, class: u8, type_id: u8) -> Attr {
    Attr::new(id, [0xF0, class, 0x00, type_id])
}

pub struct Calc<T: Transport> {
    link: Link<T>,
    /// libticalcs pauses 50 ms after each variable ("needed").
    pub inter_var_delay: Duration,
}

impl<T: Transport> Calc<T> {
    pub fn new(transport: T) -> Self {
        Self {
            link: Link::new(transport, Role::Host),
            inter_var_delay: Duration::from_millis(50),
        }
    }

    pub fn link(&mut self) -> &mut Link<T> {
        &mut self.link
    }

    /// See [`Link::replace_transport`]. Call `is_ready` afterwards.
    pub fn replace_transport(&mut self, transport: T) -> T {
        self.link.replace_transport(transport)
    }

    /// `is_ready`: buffer size negotiation, then set mode NORMAL.
    pub fn is_ready(&mut self) -> Result<()> {
        self.link.negotiate_buffer(DEFAULT_BUF_SIZE)?;
        self.link
            .send_vpkt(VPKT_PING, &cmd::mode_set(MODE_NORMAL))?;
        self.link.expect(VPKT_MODE_SET)?;
        Ok(())
    }

    pub fn get_params(&mut self, pids: &[u16]) -> Result<Vec<Value>> {
        self.link
            .send_vpkt(VPKT_PARM_REQ, &cmd::param_request(pids))?;
        let pkt = self.link.expect(VPKT_PARM_DATA)?;
        let vals = cmd::parse_param_data(&pkt.data)?;
        if vals.len() != pids.len() {
            return Err(Error::InvalidPacket(format!(
                "asked for {} parameters, got {}",
                pids.len(),
                vals.len()
            )));
        }
        Ok(vals)
    }

    pub fn set_param(&mut self, id: u16, data: &[u8]) -> Result<()> {
        self.link
            .send_vpkt(VPKT_PARM_SET, &cmd::param_set(id, data))?;
        self.link.expect(VPKT_DATA_ACK)?;
        Ok(())
    }

    pub fn get_version(&mut self) -> Result<CalcInfo> {
        use cmd::*;
        const PIDS: [u16; 24] = [
            PID_OS_MODE,
            PID_DEVICE_TYPE,
            PID_PRODUCT_NAME,
            PID_MAIN_PART_ID,
            PID_HW_VERSION,
            PID_LANGUAGE_ID,
            PID_SUBLANG_ID,
            PID_BOOT_BUILD_NUMBER,
            PID_BOOT_VERSION,
            PID_OS_BUILD_NUMBER,
            PID_OS_VERSION,
            PID_PHYS_RAM,
            PID_USER_RAM,
            PID_FREE_RAM,
            PID_PHYS_FLASH,
            PID_USER_FLASH,
            PID_FREE_FLASH,
            PID_LCD_WIDTH,
            PID_LCD_HEIGHT,
            PID_BITS_PER_PIXEL,
            PID_COLOR_AVAILABLE,
            PID_BATTERY,
            PID_EXACT_MATH,
            PID_CLASSIC_CLK_SUPPORT,
        ];
        let p = self.get_params(&PIDS)?;
        let sized = |i: usize, n: usize| p[i].data.as_ref().filter(|d| d.len() == n);
        let u8_at = |i: usize| sized(i, 1).map(|d| d[0]);
        let u16_at = |i: usize| sized(i, 2).map(|d| u16::from_be_bytes([d[0], d[1]]));
        let u64_at = |i: usize| sized(i, 8).map(|d| u64::from_be_bytes(d[..8].try_into().unwrap()));
        let version = |build: usize, ver: usize| -> Option<String> {
            let d = p[ver].data.as_ref().filter(|d| d.len() >= 3)?;
            match u16_at(build) {
                None => Some(format!("{}.{:02}", d[1], d[2])),
                Some(b) if d.len() == 4 => Some(format!("{}.{}.{}.{:04}", d[1], d[2], d[3], b)),
                Some(_) => None,
            }
        };
        let id = sized(3, 5).map(|d| d.to_vec());
        Ok(CalcInfo {
            run_level: u8_at(0),
            device_type: sized(1, 2).map(|d| d[1]),
            product_name: p[2].data.as_ref().map(|d| {
                String::from_utf8_lossy(d.split(|&b| b == 0).next().unwrap_or(&[])).into_owned()
            }),
            product_id: id.as_ref().map(|d| d[0]),
            main_calc_id: id.map(|d| d.iter().map(|b| format!("{b:02X}")).collect()),
            hw_version: u16_at(4),
            language_id: u8_at(5),
            sub_lang_id: u8_at(6),
            boot_version: version(7, 8),
            os_version: version(9, 10),
            ram_phys: u64_at(11),
            ram_user: u64_at(12),
            ram_free: u64_at(13),
            flash_phys: u64_at(14),
            flash_user: u64_at(15),
            flash_free: u64_at(16),
            lcd_width: u16_at(17),
            lcd_height: u16_at(18),
            bits_per_pixel: u8_at(19),
            color_screen: u8_at(20),
            battery: u8_at(21),
            exact_math: u8_at(22),
        })
    }

    /// `get_memfree`: free RAM and free Flash in bytes.
    pub fn get_memfree(&mut self) -> Result<(u64, u64)> {
        let p = self.get_params(&[cmd::PID_FREE_RAM, cmd::PID_FREE_FLASH])?;
        let be = |v: &Value| v.be_uint().unwrap_or(0);
        Ok((be(&p[0]), be(&p[1])))
    }

    /// `get_dirlist`: everything the calculator reports, apps included.
    pub fn list(&mut self) -> Result<Vec<CalcVar>> {
        let aids = [cmd::AID_VAR_SIZE, cmd::AID_VAR_TYPE, cmd::AID_ARCHIVED];
        self.link
            .send_vpkt(VPKT_DIR_REQ, &cmd::dirlist_request(&aids))?;
        let mut out = Vec::new();
        loop {
            let pkt = self.link.recv_vpkt_checked()?;
            match pkt.kind {
                VPKT_EOT => break,
                VPKT_VAR_HDR => {}
                k => {
                    return Err(Error::InvalidPacket(format!(
                        "expected variable header, got {} ({k:04X})",
                        vpkt_name(k)
                    )));
                }
            }
            let h = cmd::parse_var_header(&pkt.data)?;
            let size = h
                .attr(cmd::AID_VAR_SIZE)
                .and_then(Value::be_uint)
                .unwrap_or(0) as u32;
            let type_id = h
                .attr(cmd::AID_VAR_TYPE)
                .and_then(|a| a.bytes().get(3).copied())
                .unwrap_or(0);
            let archived = h
                .attr(cmd::AID_ARCHIVED)
                .is_some_and(|a| a.bytes().first().is_some_and(|&b| b != 0));
            out.push(CalcVar {
                name: String::from_utf8_lossy(&h.name).into_owned(),
                type_id,
                size,
                archived,
            });
        }
        Ok(out)
    }

    /// `send_var` for one entry. `archive` overrides the file's flag.
    pub fn send_var(&mut self, entry: &VarEntry, archive: Option<bool>) -> Result<()> {
        let archived = archive.unwrap_or(entry.archived);
        let attrs = [
            type_attr(cmd::AID_VAR_TYPE, 0x07, entry.type_id),
            Attr::new(cmd::AID_ARCHIVED, [archived as u8]),
            Attr::new(cmd::AID_VAR_VERSION, [0, 0, 0, entry.version]),
        ];
        let name = entry.wire_name();
        let size = entry.data.len() as u32;
        self.link.send_vpkt(
            VPKT_RTS,
            &cmd::rts(b"", name.as_bytes(), size, &attrs, cmd::RTS_SILENT),
        )?;
        self.link.expect(VPKT_DATA_ACK)?;
        self.link.send_vpkt(VPKT_VAR_CNTS, &entry.data)?;
        self.link.expect(VPKT_DATA_ACK)?;
        self.link.send_vpkt(VPKT_EOT, &[])?;
        std::thread::sleep(self.inter_var_delay);
        Ok(())
    }

    pub fn send_file(&mut self, file: &VarFile, archive: Option<bool>) -> Result<()> {
        for e in &file.entries {
            self.send_var(e, archive)?;
        }
        Ok(())
    }

    /// `send_flash_834pce`: apps always go to the archive.
    pub fn send_app(&mut self, app: &FlashFile) -> Result<()> {
        if app.data_type != types::APP {
            return Err(Error::Other(
                "not an app (OS files go through send_os)".into(),
            ));
        }
        let attrs = [
            type_attr(cmd::AID_VAR_TYPE, 0x0F, app.data_type),
            Attr::new(cmd::AID_ARCHIVED, [1]),
        ];
        let size = app.data.len() as u32;
        self.link.send_vpkt(
            VPKT_RTS,
            &cmd::rts(b"", app.name.as_bytes(), size, &attrs, cmd::RTS_SILENT),
        )?;
        self.link.expect(VPKT_DATA_ACK)?;
        self.link.send_vpkt(VPKT_VAR_CNTS, &app.data)?;
        self.link.expect(VPKT_DATA_ACK)?;
        self.link.send_vpkt(VPKT_EOT, &[])?;
        Ok(())
    }

    /// `send_os_834pce`: install a new OS. The calculator erases its current
    /// OS first, so an interrupted transfer leaves it in the boot code's
    /// "waiting for OS" screen until an OS is sent again. After the final
    /// acknowledgement the calculator validates the OS and restarts, which
    /// drops the USB connection.
    ///
    /// Reports progress as (bytes sent, OS size) through the link's progress
    /// callback.
    pub fn send_os(&mut self, os: &FlashFile) -> Result<()> {
        if os.data_type != types::OS {
            return Err(Error::Other("not an OS file".into()));
        }
        if os.hw_id != crate::tifiles::PRODUCT_ID_TI84PCE {
            return Err(Error::Other(format!(
                "OS is for hardware id {:02X}, not the TI-84 Plus CE ({:02X})",
                os.hw_id,
                crate::tifiles::PRODUCT_ID_TI84PCE
            )));
        }
        let data = &os.data;
        // libticalcs scans without a bound; refuse instead.
        let hdr_len = os.os_header_len().ok_or_else(|| {
            Error::Other("OS image has no end-of-header marker (FF FF FF FF)".into())
        })?;

        // Flash erases and the final validation make the calculator slow to
        // answer; give it more than the usual timeout. Timing isn't part of
        // what libticalcs puts on the wire.
        let old_timeout = self.link.timeout();
        self.link
            .set_timeout(old_timeout.max(Duration::from_secs(60)));
        let mut progress = self.link.take_progress();
        let r = self.send_os_inner(data, hdr_len, &mut progress);
        self.link.set_progress(progress);
        self.link.set_timeout(old_timeout);
        r
    }

    fn send_os_inner(
        &mut self,
        data: &[u8],
        hdr_len: usize,
        progress: &mut Option<Box<dyn FnMut(usize, usize) + Send>>,
    ) -> Result<()> {
        let mut report = |done: usize| {
            if let Some(p) = progress {
                p(done, data.len());
            }
        };
        report(0);
        // dusb_cmd_s_mode_set: buffer size negotiation, then the mode.
        self.link.negotiate_buffer(DEFAULT_BUF_SIZE)?;
        self.link
            .send_vpkt(VPKT_PING, &cmd::mode_set(cmd::MODE_OS_TRANSFER))?;
        self.link.expect(VPKT_MODE_SET)?;

        self.link
            .send_vpkt(VPKT_OS_BEGIN, &cmd::os_begin(data.len() as u32))?;
        // libticalcs reads a chunk size from both acks; only the second is used.
        cmd::parse_os_ack(&self.link.expect(VPKT_OS_ACK)?.data)?;

        self.link.send_vpkt(VPKT_OS_HEADER, &data[..hdr_len])?;
        let pkt_size = cmd::parse_os_ack(&self.link.expect(VPKT_OS_ACK)?.data)?;
        let chunk = pkt_size.checked_sub(4).filter(|&c| c > 0).ok_or_else(|| {
            Error::InvalidPacket(format!(
                "calculator asked for OS chunks of {pkt_size} bytes"
            ))
        })? as usize;

        // Full chunks, then the remainder, which libticalcs sends even when
        // it's empty (an OS Data packet holding just the address).
        let q = data.len() / chunk;
        for i in 0..=q {
            let start = i * chunk;
            let end = (start + chunk).min(data.len());
            let addr = cmd::CE_OS_BASE_ADDR + start as u32;
            self.link
                .send_vpkt(VPKT_OS_DATA, &cmd::os_data_834pce(addr, &data[start..end]))?;
            self.link.expect(VPKT_DATA_ACK)?;
            report(end);
        }

        self.link.send_vpkt(VPKT_EOT, &[])?;
        std::thread::sleep(Duration::from_millis(500));
        self.link.expect(VPKT_EOT_ACK)?;
        Ok(())
    }

    /// `recv_var`.
    pub fn recv_var(&mut self, var: &CalcVar) -> Result<VarEntry> {
        let aids = [cmd::AID_ARCHIVED, cmd::AID_VAR_VERSION, cmd::AID_VAR_SIZE];
        let attrs = [type_attr(cmd::AID_VAR_TYPE2, 0x07, var.type_id)];
        self.link.send_vpkt(
            VPKT_VAR_REQ,
            &cmd::var_request(b"", var.name.as_bytes(), &aids, &attrs),
        )?;
        let hdr = cmd::parse_var_header(&self.link.expect(VPKT_VAR_HDR)?.data)?;
        let content = self.link.expect(VPKT_VAR_CNTS)?.data;

        let size = hdr
            .attr(cmd::AID_VAR_SIZE)
            .and_then(Value::be_uint)
            .map_or(content.len(), |s| (s as usize).min(content.len()));
        let archived = hdr
            .attr(cmd::AID_ARCHIVED)
            .filter(|a| a.bytes().len() == 1)
            .map_or(var.archived, |a| a.bytes()[0] != 0);
        let version = hdr
            .attr(cmd::AID_VAR_VERSION)
            .filter(|a| a.bytes().len() == 4)
            .map_or(0, |a| a.bytes()[3]);
        Ok(VarEntry {
            type_id: var.type_id,
            raw_name: names::wire_to_raw(&var.name, var.type_id),
            version,
            archived,
            data: content[..size].to_vec(),
        })
    }

    /// `recv_flash_834pce`.
    pub fn recv_app(&mut self, var: &CalcVar) -> Result<FlashFile> {
        let aids = [cmd::AID_ARCHIVED, cmd::AID_VAR_VERSION];
        let attrs = [type_attr(cmd::AID_VAR_TYPE2, 0x0F, var.type_id)];
        self.link.send_vpkt(
            VPKT_VAR_REQ,
            &cmd::var_request(b"", var.name.as_bytes(), &aids, &attrs),
        )?;
        self.link.expect(VPKT_VAR_HDR)?;
        let content = self.link.expect(VPKT_VAR_CNTS)?.data;
        Ok(FlashFile::from_received(&var.name, var.type_id, content))
    }

    /// `del_var`.
    pub fn delete(&mut self, var: &CalcVar) -> Result<()> {
        let attrs = [type_attr(cmd::AID_VAR_TYPE2, 0x0B, var.type_id)];
        self.link.send_vpkt(
            VPKT_MODIF_VAR,
            &cmd::var_modify(b"", var.name.as_bytes(), &attrs, b"", b"", &[]),
        )?;
        self.link.expect(VPKT_DATA_ACK)?;
        Ok(())
    }

    /// `rename_var`.
    pub fn rename(&mut self, var: &CalcVar, new_name: &str) -> Result<()> {
        let attrs = [type_attr(cmd::AID_VAR_TYPE2, 0x07, var.type_id)];
        self.link.send_vpkt(
            VPKT_MODIF_VAR,
            &cmd::var_modify(
                b"",
                var.name.as_bytes(),
                &attrs,
                b"",
                new_name.as_bytes(),
                &[],
            ),
        )?;
        self.link.expect(VPKT_DATA_ACK)?;
        Ok(())
    }

    /// `change_attr`: move between RAM and archive.
    pub fn set_archived(&mut self, var: &CalcVar, archived: bool) -> Result<()> {
        let src = [type_attr(cmd::AID_VAR_TYPE2, 0x07, var.type_id)];
        // libticalcs uses 0xFF rather than 0x01 "to work around an OS bug".
        let dst = [Attr::new(
            cmd::AID_ARCHIVED,
            [if archived { 0xFF } else { 0x00 }],
        )];
        let name = var.name.as_bytes();
        self.link.send_vpkt(
            VPKT_MODIF_VAR,
            &cmd::var_modify(b"", name, &src, b"", name, &dst),
        )?;
        self.link.expect(VPKT_DATA_ACK)?;
        Ok(())
    }

    /// `recv_screen`.
    pub fn screenshot(&mut self) -> Result<Screenshot> {
        self.link
            .send_vpkt(VPKT_PARM_REQ, &cmd::param_request(&[cmd::PID_SCREENSHOT]))?;
        let pkt = self.link.expect(VPKT_PARM_DATA)?;
        let d = &pkt.data;
        if d.len() < 7
            || u16::from_be_bytes([d[0], d[1]]) != 1
            || u16::from_be_bytes([d[2], d[3]]) != cmd::PID_SCREENSHOT
            || d[4] != 0
        {
            return Err(Error::InvalidPacket("malformed screenshot reply".into()));
        }
        let pixels = &d[7..];
        if pixels.len() != LCD_WIDTH * LCD_HEIGHT * 2 {
            return Err(Error::InvalidPacket(format!(
                "screenshot is {} bytes, expected {}",
                pixels.len(),
                LCD_WIDTH * LCD_HEIGHT * 2
            )));
        }
        Ok(Screenshot {
            width: LCD_WIDTH,
            height: LCD_HEIGHT,
            rgb565: pixels.to_vec(),
        })
    }

    /// `send_key`: remote control key press.
    pub fn send_key(&mut self, keycode: u16) -> Result<()> {
        self.link
            .send_vpkt(VPKT_EXECUTE, &cmd::execute(b"", cmd::EID_KEY, b"", keycode))?;
        // send_key_noack: dusb_cmd_r_delay_ack (no CATCH_DELAY), then 100 ms.
        let pkt = self.link.recv_vpkt()?;
        if pkt.kind == VPKT_ERROR && pkt.data.len() >= 2 {
            return Err(Error::Calc(crate::error::CalcErrorCode(
                u16::from_be_bytes([pkt.data[0], pkt.data[1]]),
            )));
        }
        if pkt.kind != VPKT_DELAY_ACK {
            return Err(Error::InvalidPacket(format!(
                "expected delay ack, got {:04X}",
                pkt.kind
            )));
        }
        std::thread::sleep(Duration::from_millis(100));
        self.link.expect(VPKT_DATA_ACK)?;
        Ok(())
    }

    /// `execute`: run a program (or assembly program / app).
    pub fn execute(&mut self, var: &CalcVar) -> Result<()> {
        let action = match var.type_id {
            types::APP => cmd::EID_APP,
            _ => cmd::EID_PRGM,
        };
        self.link.send_vpkt(
            VPKT_EXECUTE,
            &cmd::execute(var.name.as_bytes(), action, b"", 0),
        )?;
        self.link.expect(VPKT_DATA_ACK)?;
        Ok(())
    }

    /// `get_clock` (the CE has the "new" clock).
    pub fn get_clock(&mut self) -> Result<Clock> {
        use cmd::*;
        let p = self.get_params(&[
            PID_CLASSIC_CLK_SUPPORT,
            PID_NEW_CLK_SUPPORT,
            PID_CLK_ON,
            PID_CLK_SEC_SINCE_1997,
            PID_CLK_DATE_FMT,
            PID_CLK_TIME_FMT,
            PID_CLK_SECONDS,
            PID_CLK_MINUTES,
            PID_CLK_HOURS,
            PID_CLK_DAY,
            PID_CLK_MONTH,
            PID_CLK_YEAR,
        ])?;
        let b = |i: usize| p[i].bytes().first().copied().unwrap_or(0);
        let date_format = if b(4) == 0 { 3 } else { b(4) };
        if !p[0].ok() && p[1].ok() {
            if !p[6..12].iter().all(Value::ok) {
                return Err(Error::Other("calculator did not report the clock".into()));
            }
            let y = p[11].bytes();
            return Ok(Clock {
                year: u16::from_be_bytes([y[0], *y.get(1).unwrap_or(&0)]),
                month: b(10),
                day: b(9),
                hours: b(8),
                minutes: b(7),
                seconds: b(6),
                date_format,
                time_format: if b(5) == 0x80 { 12 } else { 24 },
                on: b(2) != 0,
            });
        }
        if p[0].ok() && !p[1].ok() && p[3].ok() {
            let secs = p[3].be_uint().unwrap_or(0) as i64;
            let base = chrono::NaiveDate::from_ymd_opt(1997, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap();
            let t = base + chrono::Duration::seconds(secs);
            use chrono::{Datelike, Timelike};
            return Ok(Clock {
                year: t.year() as u16,
                month: t.month() as u8,
                day: t.day() as u8,
                hours: t.hour() as u8,
                minutes: t.minute() as u8,
                seconds: t.second() as u8,
                date_format,
                time_format: if b(5) != 0 { 24 } else { 12 },
                on: b(2) != 0,
            });
        }
        Err(Error::Other(
            "could not determine the calculator's clock type".into(),
        ))
    }

    /// `set_clock` for the new-style clock the CE uses.
    pub fn set_clock(&mut self, c: &Clock) -> Result<()> {
        use cmd::*;
        let p = self.get_params(&[PID_CLASSIC_CLK_SUPPORT, PID_NEW_CLK_SUPPORT])?;
        if !(!p[0].ok() && p[1].ok()) {
            return Err(Error::Other(
                "calculator does not have the new-style clock; refusing to set it".into(),
            ));
        }
        self.set_param(
            PID_CLK_DATE_FMT,
            &[if c.date_format == 3 { 0 } else { c.date_format }],
        )?;
        self.set_param(
            PID_CLK_TIME_FMT,
            &[if c.time_format == 24 { 0x81 } else { 0x80 }],
        )?;
        self.set_param(PID_CLK_YEAR, &c.year.to_be_bytes())?;
        self.set_param(PID_CLK_MONTH, &[c.month])?;
        self.set_param(PID_CLK_DAY, &[c.day])?;
        self.set_param(PID_CLK_HOURS, &[c.hours])?;
        self.set_param(PID_CLK_MINUTES, &[c.minutes])?;
        self.set_param(PID_CLK_SECONDS, &[c.seconds])?;
        self.set_param(PID_CLK_ON, &[c.on as u8])?;
        Ok(())
    }
}
