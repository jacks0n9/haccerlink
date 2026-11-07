//! Byte transports a DUSB link can run over.
//!
//! The calculator side of USB is a pair of bulk endpoints, so a transport is
//! just "send this buffer as one bulk transfer" and "give me exactly N bytes".
//! An empty `send` is a zero-length packet.

mod channel;
mod stdio;
mod usb;

pub use channel::{ChannelTransport, channel_pair};
pub use stdio::StdioTransport;
pub use usb::{UsbDeviceInfo, UsbTransport, list_calculators};

use crate::error::Result;
use std::time::Duration;

pub trait Transport: Send {
    /// Send `data` as a single bulk OUT transfer. Empty data is a ZLP.
    fn send(&mut self, data: &[u8]) -> Result<()>;
    /// Fill `buf` completely from the IN direction.
    fn recv_exact(&mut self, buf: &mut [u8]) -> Result<()>;
    fn set_timeout(&mut self, timeout: Duration);
    fn timeout(&self) -> Duration;
}

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn send(&mut self, data: &[u8]) -> Result<()> {
        (**self).send(data)
    }
    fn recv_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        (**self).recv_exact(buf)
    }
    fn set_timeout(&mut self, timeout: Duration) {
        (**self).set_timeout(timeout)
    }
    fn timeout(&self) -> Duration {
        (**self).timeout()
    }
}

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
