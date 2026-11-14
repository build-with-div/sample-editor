//! Port of `char.ts`: character helpers.
//!
//! Upstream works on UTF-16 code units. Here positions are UTF-8 byte
//! offsets, so [`code_point_size`] gives a character's UTF-8 length, the
//! amount of positions it takes up in a Rust string.

pub use crate::cluster::find_cluster_break;

/// The code point at byte offset `pos` of `s` (`codePointAt`).
///
/// # Panics
///
/// When `pos` is not a character boundary inside `s`.
pub fn code_point_at(s: &str, pos: usize) -> u32 {
    s[pos..]
        .chars()
        .next()
        .unwrap_or_else(|| panic!("No character at position {pos}")) as u32
}

/// The string for a code point (`fromCodePoint`). Surrogate code points,
/// which a Rust string cannot hold, give U+FFFD.
pub fn from_code_point(code: u32) -> String {
    char::from_u32(code)
        .unwrap_or(char::REPLACEMENT_CHARACTER)
        .to_string()
}

/// The amount of positions a character takes up in a string
/// (`codePointSize`): its UTF-8 length, from 1 to 4.
pub fn code_point_size(code: u32) -> usize {
    match code {
        0..0x80 => 1,
        0x80..0x800 => 2,
        0x800..0x10000 => 3,
        _ => 4,
    }
}
