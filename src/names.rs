//! Variable name conversion between the tokenized names stored in TI files
//! (e.g. `5D 00` for L₁) and the UTF-8 names the CE uses over DUSB ("L₁").
//!
//! Follows libticonv's `ticonv_varname_from_tifile` / `_to_tifile` for the
//! TI-84 Plus CE: detokenize with the TI-84+ rules, then map through the
//! TI-83+ character set.

/// libticonv's `ti83p_charset`, as UTF-8 strings.
#[rustfmt::skip]
const TI83P_CHARSET: [&str; 256] = [
    "\0", "η", "u", "v", "w", "▶", "↑", "↓",
    "∫", "×", "°", "¸", "·", "⊺", "³", "F",
    "√", "´", "²", "∠", "°", "ʳ", "⊺", "≤",
    "≠", "≥", "−", "𝖤", "→", "?", "↑", "↓",
    " ", "!", "\"", "#", "⁴", "%", "&", "'",
    "(", ")", "*", "+", ",", "-", ".", "/",
    "0", "1", "2", "3", "4", "5", "6", "7",
    "8", "9", ":", ";", "<", "=", ">", "?",
    "@", "A", "B", "C", "D", "E", "F", "G",
    "H", "I", "J", "K", "L", "M", "N", "O",
    "P", "Q", "R", "S", "T", "U", "V", "W",
    "X", "Y", "Z", "θ", "\\", "]", "^", "_",
    "`", "a", "b", "c", "d", "e", "f", "g",
    "h", "i", "j", "k", "l", "m", "n", "o",
    "p", "q", "r", "s", "t", "u", "v", "w",
    "x", "y", "z", "{", "|", "}", "~", "=",
    "₀", "₁", "₂", "₃", "₄", "₅", "₆", "₇",
    "₈", "₉", "Á", "À", "Â", "Ä", "á", "à",
    "â", "ä", "É", "È", "Ê", "Ë", "é", "è",
    "ê", "ë", "Í", "Ì", "Î", "Ï", "í", "ì",
    "î", "ï", "Ó", "Ò", "Ô", "Ö", "ó", "ò",
    "ô", "ö", "Ú", "Ù", "Û", "Ü", "ú", "ù",
    "û", "ü", "Ç", "ç", "Ñ", "ñ", "'", "`",
    "¨", "¿", "¡", "α", "β", "γ", "Δ", "δ",
    "ε", "[", "λ", "μ", "π", "ρ", "Σ", "σ",
    "τ", "ϕ", "Ω", "x̅", "y̅", "¤", "…", "◀",
    "◾", "?", "−", "²", "°", "³", "\n", "𝒊",
    "?", "χ", "F", "ℯ", "L", "N", "_", "→",
    "_", "_", "_", "_", "_", "_", "_", "_",
    "_", "_", "_", "_", "_", "_", "_", "_",
    "_", "_", "$", "_", "ß", "_", "_", "_",
    "_", "_", "_", "_", "_", "_", "_", "_",
];

const SUBSCRIPTS: [char; 10] = ['₀', '₁', '₂', '₃', '₄', '₅', '₆', '₇', '₈', '₉'];

pub mod types {
    pub const REAL: u8 = 0x00;
    pub const LIST: u8 = 0x01;
    pub const MATRIX: u8 = 0x02;
    pub const EQU: u8 = 0x03;
    pub const STRING: u8 = 0x04;
    pub const PROGRAM: u8 = 0x05;
    pub const PROT_PROGRAM: u8 = 0x06;
    pub const PICTURE: u8 = 0x07;
    pub const GDB: u8 = 0x08;
    pub const COMPLEX: u8 = 0x0C;
    pub const CLIST: u8 = 0x0D;
    pub const WINDOW: u8 = 0x0F;
    pub const RCL_WINDOW: u8 = 0x10;
    pub const TABLE_RANGE: u8 = 0x11;
    pub const BACKUP: u8 = 0x13;
    pub const APPVAR: u8 = 0x15;
    pub const TEMP_PROGRAM: u8 = 0x16;
    pub const GROUP: u8 = 0x17;
    pub const FRACTION: u8 = 0x18;
    pub const IMAGE: u8 = 0x1A;
    pub const OS: u8 = 0x23;
    pub const APP: u8 = 0x24;
}

/// Map raw TI-charset bytes (stopping at NUL) to UTF-8.
pub fn ti_to_utf8(raw: &[u8]) -> String {
    raw.iter()
        .take_while(|&&b| b != 0)
        .map(|&b| TI83P_CHARSET[b as usize])
        .collect()
}

/// Map UTF-8 back to TI-charset bytes, choosing the first matching code
/// (libticonv's `ticonv_utf16_to_ti83p`); unknown characters become '?'.
pub fn utf8_to_ti(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        let overlined = rest.starts_with("x̅") || rest.starts_with("y̅");
        // Plain ASCII except '$' and '[' (which the TI charset puts elsewhere).
        if !overlined && (c.is_ascii_graphic() || c == ' ') && c != '$' && c != '[' {
            out.push(c as u8);
            rest = &rest[1..];
            continue;
        }
        // Longest match wins (so "x̅" beats "x"); among equals, the lowest code.
        let best = (1..=255usize)
            .rev()
            .filter(|&i| rest.starts_with(TI83P_CHARSET[i]))
            .max_by_key(|&i| TI83P_CHARSET[i].len());
        match best {
            Some(i) => {
                out.push(i as u8);
                rest = &rest[TI83P_CHARSET[i].len()..];
            }
            None => {
                out.push(b'?');
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    out
}

fn digit_index(tok2: u8) -> String {
    // Pic1..Pic9, Pic0 for 9 (likewise Str, GDB, Image).
    if tok2 == 9 {
        "0".into()
    } else {
        (tok2 + 1).to_string()
    }
}

fn shift(v: u8) -> u8 {
    if v == 0 { 9 } else { v - 1 }
}

/// Name as used on the wire / shown to the user, from a raw file name.
pub fn raw_to_wire(raw: &[u8], type_id: u8) -> String {
    match type_id {
        types::WINDOW => return "Window".into(),
        types::RCL_WINDOW => return "RclWin".into(),
        types::TABLE_RANGE => return "TblSet".into(),
        _ => {}
    }
    let raw: Vec<u8> = raw
        .iter()
        .copied()
        .take_while(|&b| b != 0)
        .take(8)
        .collect();
    let tok1 = raw.first().copied().unwrap_or(0);
    let tok2 = raw.get(1).copied().unwrap_or(0);
    match tok1 {
        0x3C if type_id == types::IMAGE => format!("Image{}", digit_index(tok2)),
        0x5C => {
            let c = if tok2 <= 9 {
                (b'A' + tok2) as char
            } else {
                '?'
            };
            format!("[{c}]")
        }
        0x5D => match tok2 {
            0x00..=0x09 => format!("L{}", SUBSCRIPTS[(tok2 as usize + 1) % 10]),
            0x40 => "IDList".into(),
            _ => ti_to_utf8(&raw[1..]),
        },
        0x5E => match tok2 {
            0x10..=0x19 => format!("Y{}", SUBSCRIPTS[((tok2 - 0x10) as usize + 1) % 10]),
            0x20..=0x2B => {
                let xy = if tok2 & 1 == 0 { 'X' } else { 'Y' };
                format!("{xy}{}⊺", SUBSCRIPTS[((tok2 - 0x20) >> 1) as usize + 1])
            }
            0x40..=0x45 => format!("r{}", SUBSCRIPTS[(tok2 - 0x40) as usize + 1]),
            0x80 => "u".into(),
            0x81 => "v".into(),
            0x82 => "w".into(),
            _ => "?".into(),
        },
        0x60 => format!("Pic{}", digit_index(tok2)),
        0x61 => format!("GDB{}", digit_index(tok2)),
        0xAA => format!("Str{}", digit_index(tok2)),
        _ => ti_to_utf8(&raw),
    }
}

/// Raw file name (up to 8 bytes, unpadded) for a wire name.
pub fn wire_to_raw(name: &str, type_id: u8) -> Vec<u8> {
    match type_id {
        types::WINDOW | types::RCL_WINDOW | types::TABLE_RANGE => return Vec::new(),
        _ => {}
    }
    if matches!(name, "Window" | "RclWin" | "RclWindw" | "TblSet") {
        return Vec::new();
    }
    let chars: Vec<char> = name.chars().collect();
    let sub = |c: char| SUBSCRIPTS.iter().position(|&s| s == c).map(|i| i as u8);
    let digit = |c: char| c.to_digit(10).map(|d| d as u8);

    // Matrices [A]..[J]
    if chars.len() == 3 && chars[0] == '[' && chars[2] == ']' && ('A'..='J').contains(&chars[1]) {
        return vec![0x5C, chars[1] as u8 - b'A'];
    }
    // Lists L₁..L₆ (and L₇..L₀)
    if chars.len() == 2
        && chars[0] == 'L'
        && let Some(d) = sub(chars[1])
    {
        return vec![0x5D, shift(d)];
    }
    if name == "IDList" {
        return vec![0x5D, 0x40];
    }
    if type_id == types::EQU || type_id == types::REAL {
        if chars.len() == 2
            && chars[0] == 'Y'
            && let Some(d) = sub(chars[1])
        {
            return vec![0x5E, 0x10 + shift(d)];
        }
        if chars.len() == 3
            && (chars[0] == 'X' || chars[0] == 'Y')
            && chars[2] == '⊺'
            && let Some(d) = sub(chars[1]).filter(|d| (1..=6).contains(d))
        {
            return vec![0x5E, 0x20 + 2 * (d - 1) + (chars[0] == 'Y') as u8];
        }
        if chars.len() == 2
            && chars[0] == 'r'
            && let Some(d) = sub(chars[1]).filter(|d| (1..=6).contains(d))
        {
            return vec![0x5E, 0x40 + d - 1];
        }
        match name {
            "u" => return vec![0x5E, 0x80],
            "v" => return vec![0x5E, 0x81],
            "w" => return vec![0x5E, 0x82],
            _ => {}
        }
    }
    let numbered = |prefix: &str, tok: u8| -> Option<Vec<u8>> {
        let rest = name.strip_prefix(prefix)?;
        let mut it = rest.chars();
        let d = digit(it.next()?)?;
        it.next().is_none().then(|| vec![tok, shift(d)])
    };
    if let Some(v) = numbered("Pic", 0x60) {
        return v;
    }
    if let Some(v) = numbered("GDB", 0x61) {
        return v;
    }
    if let Some(v) = numbered("Str", 0xAA) {
        return v;
    }
    if type_id == types::IMAGE
        && let Some(v) = numbered("Image", 0x3C)
    {
        return v;
    }
    let ti = utf8_to_ti(name);
    if type_id == types::LIST || type_id == types::CLIST {
        // Named list: the list token, then up to 7 characters.
        let mut v = vec![0x5D];
        v.extend(ti.into_iter().take(7));
        return v;
    }
    ti.into_iter().take(8).collect()
}

/// Short type name for the UI, like TiLP's type column.
pub fn type_name(t: u8) -> &'static str {
    match t {
        0x00 => "REAL",
        0x01 => "LIST",
        0x02 => "MATRX",
        0x03 => "EQU",
        0x04 => "STRNG",
        0x05 => "PRGM",
        0x06 => "PPGM",
        0x07 => "PIC",
        0x08 => "GDB",
        0x0B => "SMART",
        0x0C => "CPLX",
        0x0D => "CLIST",
        0x0F => "WINDW",
        0x10 => "ZSTO",
        0x11 => "TABLE",
        0x13 => "BKUP",
        0x14 => "DEL",
        0x15 => "APPV",
        0x16 => "TPRGM",
        0x17 => "GROUP",
        0x18 => "FRAC",
        0x19 => "DIR",
        0x1A => "IMAGE",
        0x1B => "EXCPLXFRAC",
        0x1C => "EXREALRAD",
        0x1D => "EXCPLXRAD",
        0x1E => "EXCPLXPI",
        0x1F => "EXCPLXPIFRAC",
        0x20 => "EXREALPI",
        0x21 => "EXREALPIFRAC",
        0x23 => "OS",
        0x24 => "APPL",
        0x25 => "CERT",
        0x26 => "IDLIST",
        0x27 => "GETCERT",
        0x29 => "CLOCK",
        _ => "?",
    }
}

/// File extension TiLP uses for each type on the 84+CE.
pub fn file_extension(t: u8) -> &'static str {
    match t {
        0x00 | 0x18 | 0x1C | 0x20 | 0x21 => "8xn",
        0x01 | 0x0D => "8xl",
        0x02 => "8xm",
        0x03 => "8xy",
        0x04 => "8xs",
        0x05 | 0x06 | 0x16 => "8xp",
        0x07 => "8ci",
        0x08 => "8xd",
        0x0C | 0x1B | 0x1D | 0x1E | 0x1F => "8xc",
        0x0F => "8xw",
        0x10 => "8xz",
        0x11 => "8xt",
        0x13 => "8xb",
        0x15 => "8xv",
        0x17 => "8xg",
        0x1A => "8ca",
        0x23 => "8eu",
        0x24 => "8ek",
        0x26 => "8xidl",
        _ => "8x?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips() {
        let cases: &[(&[u8], u8, &str)] = &[
            (b"ABC", types::PROGRAM, "ABC"),
            (&[0x5D, 0x00], types::LIST, "L₁"),
            (&[0x5D, 0x05], types::LIST, "L₆"),
            (&[0x5D, b'F', b'O', b'O'], types::LIST, "FOO"),
            (&[0x5C, 0x00], types::MATRIX, "[A]"),
            (&[0x5E, 0x10], types::EQU, "Y₁"),
            (&[0x5E, 0x19], types::EQU, "Y₀"),
            (&[0x5E, 0x21], types::EQU, "Y₁⊺"),
            (&[0xAA, 0x00], types::STRING, "Str1"),
            (&[0xAA, 0x09], types::STRING, "Str0"),
            (&[0x60, 0x01], types::PICTURE, "Pic2"),
            (&[0x3C, 0x00], types::IMAGE, "Image1"),
            (&[b'A', 0x5B], types::REAL, "Aθ"),
            (b"MYAPPVAR", types::APPVAR, "MYAPPVAR"),
        ];
        for &(raw, t, wire) in cases {
            assert_eq!(raw_to_wire(raw, t), wire, "raw {raw:02X?}");
            assert_eq!(wire_to_raw(wire, t), raw, "wire {wire}");
        }
    }

    #[test]
    fn special_types() {
        assert_eq!(raw_to_wire(&[], types::WINDOW), "Window");
        assert_eq!(raw_to_wire(&[], types::RCL_WINDOW), "RclWin");
        assert_eq!(wire_to_raw("TblSet", types::TABLE_RANGE), b"");
    }
}
