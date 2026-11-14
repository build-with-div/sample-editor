//! Port of [`@marijn/find-cluster-break`] 1.0.4 (MIT), which
//! `@codemirror/state` re-exports as `findClusterBreak`.
//!
//! The package approximates grapheme clusters with a small table of
//! extending characters (Unicode 17 `Grapheme_Cluster_Break=Extend`, plus
//! the spacing marks UAX #29 does not exclude), zero-width joiners and
//! regional-indicator pairs. The port keeps the table and the algorithm, so
//! cluster boundaries match the package, not full UAX #29. Positions are
//! UTF-8 byte offsets; see the crate docs.
//!
//! [`@marijn/find-cluster-break`]: https://www.npmjs.com/package/@marijn/find-cluster-break

use std::sync::LazyLock;

// Compressed representation of the extending ranges, copied verbatim
// from the package's `src/index.js`. Each pair of elements is a range, as
// an offset from the previous range and a length, in base 36, with the
// empty string standing for 1. The package builds it with
// `bin/build-extenders.js`.
const EXTENDERS: &str = concat!(
    "lc,34,7n,7,7b,19,,,,2,,2,,,20,b,1c,l,g,,2t,7,2,6,2,2,,4,z,,u,r,2j,b,1m,",
    "9,9,,o,4,,9,,3,,5,17,3,1n,9,16,o,,x,1i,3,,i,,7,a,2,t,3,1k,,,7,2,2,2,3,9,",
    ",a,2,q,,2,3,1k,,,5,4,2,2,3,3,,u,2,3,,b,3,1k,,,8,,3,,3,k,2,m,6,,3,1k,,,7,",
    "2,2,2,3,7,3,a,2,u,,1n,5,3,3,,4,9,,14,5,1j,,,7,,3,,4,7,2,b,2,t,3,1k,,,7,,",
    "3,,4,7,2,b,2,f,,c,4,1j,2,,7,,3,,4,9,,a,2,t,3,1y,,4,6,,,,8,i,2,1p,,,8,c,",
    "8,2q,,,a,b,7,21,2,r,,,,,,4,2,1d,k,,2,5,b,,10,9,,2u,b,,6,n,4,4,3,g,4,d,,,",
    "3,6,,f,,jj,3,qa,4,s,3,t,2,u,2,1s,w,9,,19,3,,,39,2,y,,3a,c,4,c,63,5,1l,a,",
    ",,,,2,o,2,,1c,1a,2,c,k,5,1b,h,12,9,c,3,u,d,1k,e,1c,k,48,3,,l,4,,6,,2,3,",
    "5i,1s,ek,,5f,x,2da,3,3x,,2o,w,fe,6,2x,2,n9w,4,,a,w,2,28,2,7k,,3,,4,,n,5,",
    "4,,2b,2,1e,i,q,i,d,,12,8,p,d,18,4,1b,e,10,,1v,e,c,,8,2,1a,,1f,,,3,2,2,5,",
    "2,,,15,5,5,2,6k,8,,2,fn4,,kh,g,g,g,a6,2,gt,,6a,,45,5,1ae,3,,2,5,4,14,3,",
    "4,,4l,2,fx,4,1t,5,8t,2,25,6,1y,b,1d,4,3e,3,1h,f,15,,2,2,a,4,19,b,7,,1p,",
    "3,10,e,g,2,18,,c,3,1c,e,8,4,,2,2k,c,6,,2,,4d,c,l,4,1j,2,,7,2,2,2,3,9,,a,",
    "2,2,7,3,5,1v,9,,,2,,,4,,5,,,e,2,2a,i,n,,29,k,6j,7,2,9,r,2,2a,h,2y,d,2t,",
    "3,2,a,74,f,6t,6,,2,2,4,,,,2,3x,7,2,7,3,,s,a,14,7,,4,8,,9,b,1a,g,5i,8,5j,",
    "8,,8,2a,m,,e,3e,6,3,,,2,,7,,,1u,5,,2,,5,9n,4,9,2,,,1c,7,3,5,n,,44l,,6,f,",
    "8ug,i,1xc,5,1n,7,t4,,,1j,7,4,29,,b,2,f57,2,3mp,1a,2,n,f2,5,3,6,8,8,2,7,",
    "u,4,44,3,1iz,1j,4,1e,8,,e,,m,5,,f,11s,7,,h,2,7,,2,,5,2s,,4g,7,af,,1p,4,",
    "e4,4,72,2,6r,,2,,7,2,5,,d6,7,31,7,240,5",
);

// `(from, to)` ranges of extending code points, `to` exclusive.
static RANGES: LazyLock<Vec<(u32, u32)>> = LazyLock::new(|| {
    let mut ranges = Vec::new();
    let mut n = 0u32;
    let mut from = 0u32;
    for (i, s) in EXTENDERS.split(',').enumerate() {
        n += if s.is_empty() {
            1
        } else {
            u32::from_str_radix(s, 36).expect("base-36 table entry")
        };
        if i % 2 == 0 {
            from = n;
        } else {
            ranges.push((from, n));
        }
    }
    ranges
});

const ZWJ: u32 = 0x200d;

/// Whether `code` is an extending character (`isExtendingChar`).
pub fn is_extending_char(code: u32) -> bool {
    if code < 768 {
        return false;
    }
    let ranges = &*RANGES;
    let (mut from, mut to) = (0, ranges.len());
    loop {
        let mid = (from + to) >> 1;
        if code < ranges[mid].0 {
            to = mid;
        } else if code >= ranges[mid].1 {
            from = mid + 1;
        } else {
            return true;
        }
        if from == to {
            return false;
        }
    }
}

fn is_regional_indicator(code: u32) -> bool {
    (0x1F1E6..=0x1F1FF).contains(&code)
}

fn code_at(s: &str, pos: usize) -> u32 {
    s[pos..].chars().next().expect("a character") as u32
}

/// Returns the next cluster break _after_ (not equal to) `pos` when
/// `forward` is true, or before it otherwise. Returns `pos` itself when
/// there is no further break in the string.
///
/// # Panics
///
/// When `pos` is past the end of `s` or not on a character boundary.
pub fn find_cluster_break(s: &str, pos: usize, forward: bool, include_extending: bool) -> usize {
    assert!(
        s.is_char_boundary(pos),
        "Position {pos} is not a character boundary"
    );
    if forward {
        next_cluster_break(s, pos, include_extending)
    } else {
        prev_cluster_break(s, pos, include_extending)
    }
}

fn next_cluster_break(s: &str, mut pos: usize, include_extending: bool) -> usize {
    if pos == s.len() {
        return pos;
    }
    // Upstream first moves a position inside a surrogate pair to the
    // pair's start; a UTF-8 position is always a character start.
    let mut prev = code_at(s, pos);
    pos += char_len(prev);
    while pos < s.len() {
        let next = code_at(s, pos);
        if prev == ZWJ || next == ZWJ || include_extending && is_extending_char(next) {
            pos += char_len(next);
            prev = next;
        } else if is_regional_indicator(next) {
            let mut count_before = 0;
            let mut i = pos;
            while let Some(c) = s[..i].chars().next_back() {
                if !is_regional_indicator(c as u32) {
                    break;
                }
                count_before += 1;
                i -= c.len_utf8();
            }
            if count_before % 2 == 0 {
                break;
            }
            pos += char_len(next);
        } else {
            break;
        }
    }
    pos
}

fn char_len(code: u32) -> usize {
    char::from_u32(code).map_or(1, char::len_utf8)
}

// A position in UTF-16 code units, held as the byte offset of the
// character it falls in and the unit offset inside that character (1 only
// for the second half of a surrogate pair).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Unit {
    byte: usize,
    half: u8,
}

impl Unit {
    // One UTF-16 unit back, or `None` at the start.
    fn back(self, s: &str) -> Option<Unit> {
        if self.half == 1 {
            return Some(Unit {
                byte: self.byte,
                half: 0,
            });
        }
        let c = s[..self.byte].chars().next_back()?;
        let byte = self.byte - c.len_utf8();
        Some(Unit {
            byte,
            half: (c.len_utf16() - 1) as u8,
        })
    }
}

// Upstream steps back two UTF-16 units at a time from `pos` and returns
// the first next-break found before it. The search runs over UTF-16 unit
// positions so that it visits the same candidates as upstream.
fn prev_cluster_break(s: &str, pos: usize, include_extending: bool) -> usize {
    let mut pos = Unit { byte: pos, half: 0 };
    while let Some(start) = pos.back(s).and_then(|p| p.back(s)) {
        // `nextClusterBreak` moves a position inside a pair to its start.
        let found = Unit {
            byte: next_cluster_break(s, start.byte, include_extending),
            half: 0,
        };
        if found < pos {
            return found.byte;
        }
        pos = pos.back(s).expect("two units back exist");
    }
    0
}
