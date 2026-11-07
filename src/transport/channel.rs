use super::{DEFAULT_TIMEOUT, Transport};
use crate::error::{Error, Result};
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

/// In-process transport, used to wire the host side to the simulator.
pub struct ChannelTransport {
    tx: Sender<Vec<u8>>,
    rx: Receiver<Vec<u8>>,
    pending: VecDeque<u8>,
    timeout: Duration,
}

pub fn channel_pair() -> (ChannelTransport, ChannelTransport) {
    let (a_tx, b_rx) = channel();
    let (b_tx, a_rx) = channel();
    let make = |tx, rx| ChannelTransport {
        tx,
        rx,
        pending: VecDeque::new(),
        timeout: DEFAULT_TIMEOUT,
    };
    (make(a_tx, a_rx), make(b_tx, b_rx))
}

impl Transport for ChannelTransport {
    fn send(&mut self, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            // A ZLP carries no bytes on a stream.
            return Ok(());
        }
        self.tx.send(data.to_vec()).map_err(|_| Error::Disconnected)
    }

    fn recv_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        while self.pending.len() < buf.len() {
            match self.rx.recv_timeout(self.timeout) {
                Ok(chunk) => self.pending.extend(chunk),
                Err(RecvTimeoutError::Timeout) => return Err(Error::Timeout),
                Err(RecvTimeoutError::Disconnected) => return Err(Error::Disconnected),
            }
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
