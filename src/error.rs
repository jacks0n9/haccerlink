use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("USB error: {0}")]
    Usb(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("timed out waiting for the calculator")]
    Timeout,
    #[error("link closed by the other side")]
    Disconnected,
    #[error("invalid packet: {0}")]
    InvalidPacket(String),
    #[error("calculator returned error code {0}")]
    Calc(CalcErrorCode),
    #[error("unexpected end of transmission")]
    Eot,
    #[error("invalid file: {0}")]
    InvalidFile(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Error code carried by a DUSB `0xEE00` virtual packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalcErrorCode(pub u16);

/// The codes libticalcs knows about; it reports them as ERR_CALC_ERROR2 + index + 1.
const KNOWN_CODES: [u16; 19] = [
    0x0004, 0x0006, 0x0008, 0x0009, 0x000c, 0x000d, 0x000e, 0x0011, 0x0012, 0x001c, 0x001d, 0x0021,
    0x0022, 0x0023, 0x0027, 0x0029, 0x002b, 0x002e, 0x0034,
];

impl CalcErrorCode {
    /// Index as computed by libticalcs' `err_code()` (0 when the code is unknown).
    pub fn libticalcs_index(self) -> u16 {
        KNOWN_CODES
            .iter()
            .position(|&c| c == self.0)
            .map_or(0, |i| i as u16 + 1)
    }
}

impl fmt::Display for CalcErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:04X}", self.0)?;
        if self.0 == 0x0009 || self.0 == 0x000e {
            // These two show up constantly in practice.
            write!(f, " (variable exists/locked or not enough memory?)")?;
        }
        Ok(())
    }
}
