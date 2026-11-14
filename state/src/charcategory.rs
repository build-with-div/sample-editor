//! Port of `charcategory.ts`: word, space and other characters.

/// The categories a [character
/// categorizer](crate::EditorState::char_categorizer) produces, used for
/// things like selecting by word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharCategory {
    /// Word characters.
    Word,
    /// Whitespace.
    Space,
    /// Anything else.
    Other,
}

// JavaScript's `\s`: WhiteSpace and LineTerminator. This differs from
// `char::is_whitespace` (Unicode White_Space) at U+0085 and U+FEFF.
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

// Upstream tests `/[\p{Alphabetic}\p{Number}_]/u`. `char::is_alphabetic`
// is the Alphabetic property and `char::is_numeric` the Number categories
// (Nd, Nl and No).
fn has_word_char(s: &str) -> bool {
    s.chars()
        .any(|c| c.is_alphabetic() || c.is_numeric() || c == '_')
}

/// A categorizer that also counts the characters in `word_chars` as word
/// characters (`makeCategorizer`).
pub(crate) fn make_categorizer(word_chars: String) -> impl Fn(&str) -> CharCategory {
    move |s: &str| {
        if !s.chars().any(|c| !is_js_space(c)) {
            CharCategory::Space
        } else if has_word_char(s) || word_chars.chars().any(|c| s.contains(c)) {
            CharCategory::Word
        } else {
            CharCategory::Other
        }
    }
}
