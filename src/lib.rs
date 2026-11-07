//! haccerlink: talk to a TI-84 Plus CE over USB using the DUSB protocol.

pub mod calc;
pub mod dusb;
pub mod error;
pub mod keys;
pub mod logging;
pub mod names;
pub mod sim;
pub mod tifiles;
pub mod transport;

pub use calc::{Calc, CalcVar};
pub use error::{Error, Result};
