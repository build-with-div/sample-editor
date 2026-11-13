//! Helpers shared by the ported upstream tests.
#![allow(dead_code)]

use std::cell::Cell;

thread_local! {
    static SEED: Cell<u64> = const { Cell::new(0x9E37_79B9_7F4A_7C15) };
}

/// `Math.floor(Math.random() * n)` upstream, from a fixed-seed xorshift
/// generator so failures reproduce.
pub fn r(n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    SEED.with(|seed| {
        let mut x = seed.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        seed.set(x);
        (x % n as u64) as usize
    })
}

/// `r() < .5` upstream.
pub fn coin() -> bool {
    r(2) == 0
}

/// A random lowercase ASCII string of `len` characters.
pub fn r_str(len: usize) -> String {
    (0..len).map(|_| (b'a' + r(26) as u8) as char).collect()
}
