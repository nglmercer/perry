//! Cell colors. v0.1 supports the 16-color ANSI palette (default + 8
//! named + 8 bright). 24-bit truecolor lands in Phase 3.5 alongside
//! Taffy — both depend on the cell-grid containing a richer encoding,
//! and we're saving that bytes-per-cell cost until we actually need it.

/// 16-color palette index (0..15). 16 = "default" sentinel (don't emit
/// any SGR for this side; let the terminal use its own default).
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
#[repr(u8)]
pub enum Color {
    #[default]
    Default = 16,
    Black = 0,
    Red = 1,
    Green = 2,
    Yellow = 3,
    Blue = 4,
    Magenta = 5,
    Cyan = 6,
    White = 7,
    BrightBlack = 8,
    BrightRed = 9,
    BrightGreen = 10,
    BrightYellow = 11,
    BrightBlue = 12,
    BrightMagenta = 13,
    BrightCyan = 14,
    BrightWhite = 15,
}

impl Color {
    /// Foreground SGR code (3x or 9x for bright). Caller is responsible
    /// for the leading `\x1b[` and trailing `m`.
    pub fn fg_code(self) -> u8 {
        match self {
            Color::Default => 39, // ESC[39m = default fg
            c if (c as u8) < 8 => 30 + (c as u8),
            _ => 90 + ((self as u8) - 8),
        }
    }

    /// Background SGR code (4x or 10x for bright).
    pub fn bg_code(self) -> u8 {
        match self {
            Color::Default => 49, // ESC[49m = default bg
            c if (c as u8) < 8 => 40 + (c as u8),
            _ => 100 + ((self as u8) - 8),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fg_codes_match_ansi_spec() {
        assert_eq!(Color::Default.fg_code(), 39);
        assert_eq!(Color::Black.fg_code(), 30);
        assert_eq!(Color::Red.fg_code(), 31);
        assert_eq!(Color::White.fg_code(), 37);
        assert_eq!(Color::BrightBlack.fg_code(), 90);
        assert_eq!(Color::BrightRed.fg_code(), 91);
        assert_eq!(Color::BrightWhite.fg_code(), 97);
    }

    #[test]
    fn bg_codes_match_ansi_spec() {
        assert_eq!(Color::Default.bg_code(), 49);
        assert_eq!(Color::Black.bg_code(), 40);
        assert_eq!(Color::Red.bg_code(), 41);
        assert_eq!(Color::White.bg_code(), 47);
        assert_eq!(Color::BrightWhite.bg_code(), 107);
    }
}
