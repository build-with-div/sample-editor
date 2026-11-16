//! Port of the indentation unit of `indent.ts`: `indentUnit`,
//! `getIndentUnit` and `indentString`.

use std::sync::LazyLock;

use editor_state::{EditorState, Extension, Facet, FacetConfig};

// JavaScript's `\s`.
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// The unit by which indentation happens (`indentUnit`): a string of one
/// whitespace character repeated, two spaces when not set.
///
/// # Panics
///
/// Reading the facet panics when its first value is empty, holds a
/// character other than whitespace, or mixes characters, where upstream's
/// combine function throws `Invalid indent unit`.
pub static INDENT_UNIT: LazyLock<Facet<String, String>> = LazyLock::new(|| {
    Facet::define_with(FacetConfig::new(|values: &[String]| {
        let Some(unit) = values.first() else {
            return "  ".to_string();
        };
        let first = unit.chars().next();
        if first.is_none()
            || unit.chars().any(|c| !is_js_space(c))
            || unit.chars().any(|c| Some(c) != first)
        {
            panic!("Invalid indent unit: {unit:?}");
        }
        unit.clone()
    }))
});

/// Set the indentation unit (`indentUnit.of`).
pub fn indent_unit(unit: impl Into<String>) -> Extension {
    INDENT_UNIT.of(unit.into())
}

/// The column width of the state's indentation unit (`getIndentUnit`): its
/// length, or its length times the tab size when it holds tabs.
pub fn get_indent_unit(state: &EditorState) -> usize {
    let unit = state.facet(&INDENT_UNIT);
    // Upstream counts UTF-16 units. A valid unit is whitespace, all of
    // which is in the Basic Multilingual Plane: one unit per character.
    let len = unit.chars().count();
    if unit.starts_with('\t') {
        state.tab_size() * len
    } else {
        len
    }
}

/// An indentation string covering columns 0 to `cols` (`indentString`):
/// tabs for as much as the tab size allows when the unit holds tabs, then
/// the unit's character.
pub fn indent_string(state: &EditorState, cols: usize) -> String {
    let mut result = String::new();
    let mut cols = cols;
    let ts = state.tab_size();
    let unit = state.facet(&INDENT_UNIT);
    let mut ch = unit.chars().next().unwrap_or(' ');
    if ch == '\t' {
        while cols >= ts {
            result.push('\t');
            cols -= ts;
        }
        ch = ' ';
    }
    for _ in 0..cols {
        result.push(ch);
    }
    result
}
