use super::{DEFAULT_TIMEOUT, Transport};
use crate::error::{Error, Result};
use nusb::MaybeFuture;
use nusb::descriptors::TransferType;
use nusb::transfer::{Buffer, Bulk, Direction, In, Out, TransferError};
use std::collections::VecDeque;
use std::time::Duration;

pub const VID_TI: u16 = 0x0451;
/// TI-84 Plus (classic, USB).
pub const PID_TI84P: u16 = 0xE003;
/// TI-84 Plus SE / C SE / CE / 83 Premium CE / 84 Plus T all share this one.
pub const PID_TI84P_SE: u16 = 0xE008;

#[derive(Debug, Clone)]
pub struct UsbDeviceInfo {
    pub bus: String,
    pub address: u8,
    pub product_id: u16,
    pub product: Option<String>,
    pub serial: Option<String>,
}

impl std::fmt::Display for UsbDeviceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "bus {} addr {} [{:04x}:{:04x}] {}",
            self.bus,
            self.address,
            VID_TI,
            self.product_id,
            self.product.as_deref().unwrap_or("TI calculator")
        )?;
        if let Some(s) = &self.serial {
            write!(f, " (serial {s})")?;
        }
        Ok(())
    }
}

fn usb_err(e: impl std::fmt::Display) -> Error {
    Error::Usb(e.to_string())
}

fn is_calc(d: &nusb::DeviceInfo) -> bool {
    d.vendor_id() == VID_TI && matches!(d.product_id(), PID_TI84P | PID_TI84P_SE)
}

pub fn list_calculators() -> Result<Vec<UsbDeviceInfo>> {
    Ok(nusb::list_devices()
        .wait()
        .map_err(usb_err)?
        .filter(is_calc)
        .map(|d| UsbDeviceInfo {
            bus: d.bus_id().to_string(),
            address: d.device_address(),
            product_id: d.product_id(),
            product: d.product_string().map(str::to_owned),
            serial: d.serial_number().map(str::to_owned),
        })
        .collect())
}

/// After an OS install: wait for the calculator at `old` to drop off USB
/// (it restarts once the OS is validated), then for a calculator to show up
/// on the same bus that wasn't there before. Returns the new device.
pub fn wait_for_restart(
    old: &UsbDeviceInfo,
    gone_within: Duration,
    back_within: Duration,
) -> Result<UsbDeviceInfo> {
    use std::time::Instant;
    let same = |d: &UsbDeviceInfo, bus: &str, addr: u8| d.bus == bus && d.address == addr;
    let others: Vec<(String, u8)> = list_calculators()?
        .into_iter()
        .filter(|d| !same(d, &old.bus, old.address))
        .map(|d| (d.bus, d.address))
        .collect();
    let poll = Duration::from_millis(250);

    let start = Instant::now();
    while list_calculators()?
        .iter()
        .any(|d| same(d, &old.bus, old.address))
    {
        if start.elapsed() > gone_within {
            return Err(Error::Usb(
                "the calculator didn't restart after the OS transfer; check its screen".into(),
            ));
        }
        std::thread::sleep(poll);
    }

    let start = Instant::now();
    loop {
        let found = list_calculators()?
            .into_iter()
            .find(|d| d.bus == old.bus && !others.iter().any(|(b, a)| same(d, b, *a)));
        if let Some(d) = found {
            return Ok(d);
        }
        if start.elapsed() > back_within {
            return Err(Error::Usb(
                "the calculator didn't come back on USB after restarting".into(),
            ));
        }
        std::thread::sleep(poll);
    }
}

pub struct UsbTransport {
    out_ep: nusb::Endpoint<Bulk, Out>,
    in_ep: nusb::Endpoint<Bulk, In>,
    in_max_packet: usize,
    pending: VecDeque<u8>,
    timeout: Duration,
    pub info: UsbDeviceInfo,
}

impl UsbTransport {
    /// Open the first calculator found (or the one at `bus`/`address`).
    pub fn open(select: Option<(&str, u8)>) -> Result<Self> {
        let dev_info = nusb::list_devices()
            .wait()
            .map_err(usb_err)?
            .filter(is_calc)
            .find(|d| select.is_none_or(|(b, a)| d.bus_id() == b && d.device_address() == a))
            .ok_or_else(|| {
                Error::Usb(
                    "no TI-84 Plus CE found (is it plugged in and turned on? \
                     do you have permission to open it? see README for the udev rule)"
                        .into(),
                )
            })?;
        let info = UsbDeviceInfo {
            bus: dev_info.bus_id().to_string(),
            address: dev_info.device_address(),
            product_id: dev_info.product_id(),
            product: dev_info.product_string().map(str::to_owned),
            serial: dev_info.serial_number().map(str::to_owned),
        };

        let device = dev_info.open().wait().map_err(|e| {
            Error::Usb(format!(
                "could not open {info}: {e} (permissions? see README for the udev rule)"
            ))
        })?;

        // libticables always selects configuration #1. Only do it when needed:
        // re-selecting the active configuration resets the device's endpoints.
        let active = device
            .active_configuration()
            .map(|c| c.configuration_value())
            .ok();
        if active != Some(1) {
            device.set_configuration(1).wait().map_err(usb_err)?;
        }

        let interface = device
            .detach_and_claim_interface(0)
            .wait()
            .map_err(|e| Error::Usb(format!("could not claim interface 0: {e}")))?;

        // Find the bulk endpoints like libticables does (normally 0x02 / 0x81).
        let (mut out_addr, mut in_addr) = (0x02u8, 0x81u8);
        if let Some(desc) = interface.descriptor() {
            for ep in desc.endpoints() {
                if ep.transfer_type() != TransferType::Bulk {
                    continue;
                }
                match ep.direction() {
                    Direction::In => in_addr = ep.address(),
                    Direction::Out => out_addr = ep.address(),
                }
            }
        }

        let mut out_ep = interface.endpoint::<Bulk, Out>(out_addr).map_err(usb_err)?;
        let mut in_ep = interface.endpoint::<Bulk, In>(in_addr).map_err(usb_err)?;
        // libticables' reset clears halts on both endpoints; a stale stall from
        // an aborted transfer otherwise wedges the first packet.
        let _ = out_ep.clear_halt().wait();
        let _ = in_ep.clear_halt().wait();
        let in_max_packet = in_ep.max_packet_size().max(1);

        Ok(Self {
            out_ep,
            in_ep,
            in_max_packet,
            pending: VecDeque::new(),
            timeout: DEFAULT_TIMEOUT,
            info,
        })
    }

    fn map_transfer_error(e: TransferError) -> Error {
        match e {
            TransferError::Cancelled => Error::Timeout,
            TransferError::Disconnected => Error::Disconnected,
            other => Error::Usb(other.to_string()),
        }
    }
}

impl Transport for UsbTransport {
    fn send(&mut self, data: &[u8]) -> Result<()> {
        let c = self
            .out_ep
            .transfer_blocking(Buffer::from(data.to_vec()), self.timeout);
        c.status.map_err(Self::map_transfer_error)?;
        if c.actual_len != data.len() {
            return Err(Error::Usb(format!(
                "short write: {} of {} bytes",
                c.actual_len,
                data.len()
            )));
        }
        Ok(())
    }

    fn recv_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        while self.pending.len() < buf.len() {
            // Ask for exactly what's still missing, rounded up to whole packets.
            // Asking for more could block when the calculator ends a raw packet
            // on a packet boundary without a ZLP and then waits for our ack.
            let need = buf.len() - self.pending.len();
            let req = need.div_ceil(self.in_max_packet) * self.in_max_packet;
            let c = self.in_ep.transfer_blocking(Buffer::new(req), self.timeout);
            c.status.map_err(Self::map_transfer_error)?;
            self.pending.extend(&c.buffer[..c.actual_len]);
        }
        for b in buf.iter_mut() {
            *b = self.pending.pop_front().unwrap();
        }
        Ok(())
    }

    fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }
}
