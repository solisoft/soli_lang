//! Hex color parsing. Template colors are hex strings WITHOUT a leading `#`,
//! either 3-digit (`"fff"`) or 6-digit (`"EEEEEE"`). Case-insensitive.

/// An RGB color with components in `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Rgb {
    pub const BLACK: Rgb = Rgb {
        r: 0.0,
        g: 0.0,
        b: 0.0,
    };
    pub const WHITE: Rgb = Rgb {
        r: 1.0,
        g: 1.0,
        b: 1.0,
    };
    /// Default light-grey used for table borders when none is specified.
    pub const LIGHT_GREY: Rgb = Rgb {
        r: 0.8,
        g: 0.8,
        b: 0.8,
    };

    pub fn new(r: f32, g: f32, b: f32) -> Self {
        Rgb { r, g, b }
    }
}

/// Parse a hex color (3 or 6 hex digits, no `#`). Returns `None` if malformed.
pub fn parse_hex(s: &str) -> Option<Rgb> {
    let s = s.trim().trim_start_matches('#');
    let bytes = s.as_bytes();
    let (r, g, b) = match bytes.len() {
        3 => {
            let r = hex_nibble(bytes[0])?;
            let g = hex_nibble(bytes[1])?;
            let b = hex_nibble(bytes[2])?;
            // "f" -> 0xff, "0" -> 0x00
            (r * 17, g * 17, b * 17)
        }
        6 => {
            let r = hex_byte(bytes[0], bytes[1])?;
            let g = hex_byte(bytes[2], bytes[3])?;
            let b = hex_byte(bytes[4], bytes[5])?;
            (r, g, b)
        }
        _ => return None,
    };
    Some(Rgb::new(
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
    ))
}

/// Parse a hex color, falling back to `default` when missing or malformed.
pub fn parse_hex_or(s: Option<&str>, default: Rgb) -> Rgb {
    s.and_then(parse_hex).unwrap_or(default)
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn hex_byte(hi: u8, lo: u8) -> Option<u8> {
    Some(hex_nibble(hi)? * 16 + hex_nibble(lo)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_digit() {
        assert_eq!(parse_hex("fff"), Some(Rgb::WHITE));
        assert_eq!(parse_hex("000"), Some(Rgb::BLACK));
        assert_eq!(parse_hex("f00"), Some(Rgb::new(1.0, 0.0, 0.0)));
    }

    #[test]
    fn six_digit() {
        let g = parse_hex("EEEEEE").unwrap();
        assert!((g.r - 0.9333).abs() < 0.01);
        assert_eq!(parse_hex("000000"), Some(Rgb::BLACK));
        assert_eq!(parse_hex("ffffff"), Some(Rgb::WHITE));
    }

    #[test]
    fn leading_hash_and_bad() {
        assert_eq!(parse_hex("#fff"), Some(Rgb::WHITE));
        assert_eq!(parse_hex("xyz"), None);
        assert_eq!(parse_hex(""), None);
        assert_eq!(parse_hex("ff"), None);
    }
}
