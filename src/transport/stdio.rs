use super::{DEFAULT_TIMEOUT, Transport};
use crate::error::{Error, Result};
use std::io::{Read, Write};
use std::time::Duration;

/// Transport over stdin/stdout. Used by `haccerlink sim-serve` so external
/// programs (the libticalcs oracle) can talk to the simulator through a pipe.
/// Reads block; the timeout is not enforced.
pub struct StdioTransport {
    stdin: std::io::Stdin,
    stdout: std::io::Stdout,
    timeout: Duration,
}

impl StdioTransport {
    pub fn new() -> Self {
        Self {
            stdin: std::io::stdin(),
            stdout: std::io::stdout(),
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

impl Default for StdioTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for StdioTransport {
    fn send(&mut self, data: &[u8]) -> Result<()> {
        let mut out = self.stdout.lock();
        out.write_all(data)?;
        out.flush()?;
        Ok(())
    }

    fn recv_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        self.stdin.lock().read_exact(buf).map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                Error::Disconnected
            } else {
                Error::Io(e)
            }
        })
    }

    fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }
}
