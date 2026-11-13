//! Shared helpers for the editor probes in `tests/`.
//!
//! Each probe is a headless `TestApp` test. Sizes come from `PROBE_SCALE`:
//! unset (CI) runs small sizes that check behaviour quickly; `PROBE_SCALE=full`
//! runs the sizes `docs/design.md` reports.

use std::time::{Duration, Instant};

/// True when the probes should run the sizes the design reports.
pub fn full() -> bool {
    std::env::var("PROBE_SCALE").is_ok_and(|value| value == "full")
}

/// `small` in CI, `large` with `PROBE_SCALE=full`.
pub fn size(small: usize, large: usize) -> usize {
    if full() {
        large
    } else {
        small
    }
}

/// Run `f` and return its result and how long it took.
pub fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let value = f();
    (value, start.elapsed())
}

/// A milliseconds string for probe output.
pub fn ms(duration: Duration) -> String {
    format!("{:.1} ms", duration.as_secs_f64() * 1000.0)
}

/// A line of Rust-like source, deterministic in `n`, about 5-55 bytes.
pub fn source_line(n: usize) -> String {
    match n % 5 {
        0 => format!("fn item_{n}(value: u32) -> Option<u32> {{"),
        1 => format!("    let total = value.wrapping_mul({n}) + 7; // step"),
        2 => format!("    if total > {n} {{ return Some(total); }}"),
        3 => "    None".to_string(),
        _ => "}".to_string(),
    }
}

/// `lines` source lines joined with `\n`.
pub fn source_text(lines: usize) -> String {
    let mut text = String::new();
    for n in 0..lines {
        if n > 0 {
            text.push('\n');
        }
        text.push_str(&source_line(n));
    }
    text
}

/// Split a line into (token, class) pairs the way a highlighter would:
/// identifiers, numbers, runs of spaces, comments and single punctuation
/// characters become separate tokens.
pub fn tokens(line: &str) -> Vec<(String, &'static str)> {
    let mut out: Vec<(String, &'static str)> = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        let mut token = String::from(c);
        let class = if c.is_alphabetic() || c == '_' {
            while let Some(&n) = chars.peek() {
                if n.is_alphanumeric() || n == '_' {
                    token.push(n);
                    chars.next();
                } else {
                    break;
                }
            }
            match token.as_str() {
                "fn" | "let" | "if" | "return" => "kw",
                "u32" | "Option" | "Some" | "None" => "ty",
                _ => "id",
            }
        } else if c.is_ascii_digit() {
            while let Some(&n) = chars.peek() {
                if n.is_ascii_digit() {
                    token.push(n);
                    chars.next();
                } else {
                    break;
                }
            }
            "num"
        } else if c == ' ' {
            while chars.peek() == Some(&' ') {
                token.push(' ');
                chars.next();
            }
            "ws"
        } else if c == '/' && chars.peek() == Some(&'/') {
            token.extend(chars.by_ref());
            "com"
        } else {
            "punct"
        };
        out.push((token, class));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_cover_the_line() {
        for n in 0..5 {
            let line = source_line(n);
            let joined: String = tokens(&line).into_iter().map(|(t, _)| t).collect();
            assert_eq!(joined, line);
        }
    }
}
