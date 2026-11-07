//! Key codes for remote control (`kbdKey`/`keyExtend` values from TI's
//! ti83plus.inc, as listed in libticalcs' keys83p.h).

pub const RIGHT: u16 = 0x01;
pub const LEFT: u16 = 0x02;
pub const UP: u16 = 0x03;
pub const DOWN: u16 = 0x04;
pub const ENTER: u16 = 0x05;
pub const CLEAR: u16 = 0x09;
pub const DEL: u16 = 0x0A;
pub const INS: u16 = 0x0B;
pub const PRGM: u16 = 0x2D;
pub const ZOOM: u16 = 0x2E;
pub const STAT: u16 = 0x31;
pub const MATH: u16 = 0x32;
pub const VARS: u16 = 0x35;
pub const MATRIX: u16 = 0x37;
pub const APPS: u16 = 0x2C;
pub const CATALOG: u16 = 0x3E;
pub const QUIT: u16 = 0x40;
pub const LINK_IO: u16 = 0x41;
pub const GRAPH: u16 = 0x44;
pub const MODE: u16 = 0x45;
pub const WINDOW: u16 = 0x48;
pub const YEQU: u16 = 0x49;
pub const TABLE: u16 = 0x4A;
pub const TRACE: u16 = 0x5A;
pub const ADD: u16 = 0x80;
pub const SUB: u16 = 0x81;
pub const MUL: u16 = 0x82;
pub const DIV: u16 = 0x83;
pub const EXPON: u16 = 0x84;
pub const LPAREN: u16 = 0x85;
pub const RPAREN: u16 = 0x86;
pub const LBRACK: u16 = 0x87;
pub const RBRACK: u16 = 0x88;
pub const STORE: u16 = 0x8A;
pub const COMMA: u16 = 0x8B;
pub const CHS: u16 = 0x8C;
pub const DEC_PNT: u16 = 0x8D;
pub const K0: u16 = 0x8E;
pub const SPACE: u16 = 0x99;
pub const CAP_A: u16 = 0x9A;
pub const VAR_X: u16 = 0xB4;
pub const PI: u16 = 0xB5;
pub const SIN: u16 = 0xB7;
pub const COS: u16 = 0xB9;
pub const TAN: u16 = 0xBB;
pub const SQUARE: u16 = 0xBD;
pub const ANS: u16 = 0xC5;
pub const COLON: u16 = 0xC6;
pub const QUEST: u16 = 0xCA;
pub const QUOTE: u16 = 0xCB;
pub const THETA: u16 = 0xCC;
pub const LBRACE: u16 = 0xEC;
pub const RBRACE: u16 = 0xED;
pub const EQUAL: u16 = 0xFE0A;

/// Key code for a typed character, if it has a direct equivalent.
pub fn for_char(c: char) -> Option<u16> {
    Some(match c {
        '0'..='9' => K0 + (c as u16 - '0' as u16),
        'A'..='Z' => CAP_A + (c as u16 - 'A' as u16),
        'a'..='z' => CAP_A + (c as u16 - 'a' as u16),
        '+' => ADD,
        '-' => SUB,
        '*' => MUL,
        '/' => DIV,
        '^' => EXPON,
        '(' => LPAREN,
        ')' => RPAREN,
        '[' => LBRACK,
        ']' => RBRACK,
        '{' => LBRACE,
        '}' => RBRACE,
        ',' => COMMA,
        '.' => DEC_PNT,
        ' ' => SPACE,
        ':' => COLON,
        '?' => QUEST,
        '"' => QUOTE,
        '=' => EQUAL,
        '~' => CHS,
        '>' => STORE,
        _ => return None,
    })
}

/// Named keys for `haccerlink key <name>` and the TUI help.
pub const NAMED: &[(&str, u16)] = &[
    ("right", RIGHT),
    ("left", LEFT),
    ("up", UP),
    ("down", DOWN),
    ("enter", ENTER),
    ("clear", CLEAR),
    ("del", DEL),
    ("ins", INS),
    ("quit", QUIT),
    ("mode", MODE),
    ("prgm", PRGM),
    ("apps", APPS),
    ("math", MATH),
    ("vars", VARS),
    ("stat", STAT),
    ("matrix", MATRIX),
    ("catalog", CATALOG),
    ("graph", GRAPH),
    ("yequ", YEQU),
    ("window", WINDOW),
    ("zoom", ZOOM),
    ("trace", TRACE),
    ("table", TABLE),
    ("link", LINK_IO),
    ("sto", STORE),
    ("ans", ANS),
    ("pi", PI),
    ("x", VAR_X),
    ("sin", SIN),
    ("cos", COS),
    ("tan", TAN),
    ("square", SQUARE),
    ("theta", THETA),
    ("neg", CHS),
];

pub fn by_name(name: &str) -> Option<u16> {
    let lower = name.to_ascii_lowercase();
    if let Some(&(_, k)) = NAMED.iter().find(|(n, _)| *n == lower) {
        return Some(k);
    }
    if let Some(hex) = lower.strip_prefix("0x") {
        return u16::from_str_radix(hex, 16).ok();
    }
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => for_char(c),
        _ => None,
    }
}
