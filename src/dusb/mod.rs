//! DUSB ("DirectLink USB"), the protocol the TI-84 Plus CE speaks over USB.
//!
//! Three layers, mirroring libticalcs:
//!  * raw packets: `[size: u32 BE][type: u8][data]`, at most 1023 data bytes
//!  * virtual packets: `[size: u32 BE][type: u16 BE][data]`, fragmented over
//!    raw packets of type 3 (continued) / 4 (final), each acked with type 5
//!  * commands: the payloads of virtual packets (see [`cmd`])

pub mod cmd;
pub mod dissect;
pub mod link;

pub use link::{Dir, Link, LinkObserver, Role};

// Raw packet types.
pub const RPKT_BUF_SIZE_REQ: u8 = 1;
pub const RPKT_BUF_SIZE_ALLOC: u8 = 2;
pub const RPKT_VIRT_DATA: u8 = 3;
pub const RPKT_VIRT_DATA_LAST: u8 = 4;
pub const RPKT_VIRT_DATA_ACK: u8 = 5;

/// Size of `DUSBRawPacket::data` in libticalcs.
pub const RPKT_MAX_DATA: usize = 1023;
/// What libticalcs asks for in every buffer size request.
pub const DEFAULT_BUF_SIZE: u32 = 1024;
/// The CE allocates more than it can actually handle; libticalcs clamps to this.
pub const CE_MAX_RAW: u32 = 1018;
/// Size + type header of a virtual packet.
pub const VPKT_HEADER: usize = 6;

// Virtual packet types.
pub const VPKT_PING: u16 = 0x0001;
pub const VPKT_OS_BEGIN: u16 = 0x0002;
pub const VPKT_OS_ACK: u16 = 0x0003;
pub const VPKT_OS_HEADER: u16 = 0x0004;
pub const VPKT_OS_DATA: u16 = 0x0005;
pub const VPKT_EOT_ACK: u16 = 0x0006;
pub const VPKT_PARM_REQ: u16 = 0x0007;
pub const VPKT_PARM_DATA: u16 = 0x0008;
pub const VPKT_DIR_REQ: u16 = 0x0009;
pub const VPKT_VAR_HDR: u16 = 0x000A;
pub const VPKT_RTS: u16 = 0x000B;
pub const VPKT_VAR_REQ: u16 = 0x000C;
pub const VPKT_VAR_CNTS: u16 = 0x000D;
pub const VPKT_PARM_SET: u16 = 0x000E;
pub const VPKT_MODIF_VAR: u16 = 0x0010;
pub const VPKT_EXECUTE: u16 = 0x0011;
pub const VPKT_MODE_SET: u16 = 0x0012;
pub const VPKT_DATA_ACK: u16 = 0xAA00;
pub const VPKT_DELAY_ACK: u16 = 0xBB00;
pub const VPKT_EOT: u16 = 0xDD00;
pub const VPKT_ERROR: u16 = 0xEE00;

pub fn rpkt_name(kind: u8) -> &'static str {
    match kind {
        RPKT_BUF_SIZE_REQ => "Buffer Size Request",
        RPKT_BUF_SIZE_ALLOC => "Buffer Size Allocation",
        RPKT_VIRT_DATA => "Virtual Packet Data with Continuation",
        RPKT_VIRT_DATA_LAST => "Virtual Packet Data Final",
        RPKT_VIRT_DATA_ACK => "Virtual Packet Data Acknowledgement",
        _ => "",
    }
}

/// Names exactly as libticalcs' `dusb_vpkt_type2name` spells them.
pub fn vpkt_name(kind: u16) -> &'static str {
    match kind {
        0x0000 => "",
        VPKT_PING => "Ping / Set Mode",
        VPKT_OS_BEGIN => "Begin OS Transfer",
        VPKT_OS_ACK => "Acknowledgement of OS Transfer",
        VPKT_OS_HEADER => "OS Header",
        VPKT_OS_DATA => "OS Data",
        VPKT_EOT_ACK => "Acknowledgement of EOT",
        VPKT_PARM_REQ => "Parameter Request",
        VPKT_PARM_DATA => "Parameter Data",
        VPKT_DIR_REQ => "Request Directory Listing",
        VPKT_VAR_HDR => "Variable Header",
        VPKT_RTS => "Request to Send",
        VPKT_VAR_REQ => "Request Variable",
        VPKT_VAR_CNTS => "Variable Contents",
        VPKT_PARM_SET => "Parameter Set",
        0x000F => "",
        VPKT_MODIF_VAR => "Modify Variable",
        VPKT_EXECUTE => "Remote Control",
        VPKT_MODE_SET => "Acknowledgement of Mode Setting",
        VPKT_DATA_ACK => "Acknowledgement of Data",
        VPKT_DELAY_ACK => "Delay Acknowledgment",
        VPKT_EOT => "End of Transmission",
        VPKT_ERROR => "Error",
        _ => "unknown: not listed",
    }
}
