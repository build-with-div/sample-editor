//! The indentation unit of `indent.ts`. Upstream's tests reach it only
//! through the commands; these are the port's own, from the sources.

use editor_language::{get_indent_unit, indent_string, indent_unit};
use editor_state::{facets, EditorState, EditorStateConfig, Extension};

fn state(extensions: Extension) -> EditorState {
    EditorState::create(EditorStateConfig {
        doc: None,
        selection: None,
        extensions,
    })
    .unwrap()
}

#[test]
fn defaults_to_two_spaces() {
    let state = state(Extension::from(vec![]));
    assert_eq!(get_indent_unit(&state), 2);
    assert_eq!(indent_string(&state, 5), "     ");
}

#[test]
fn the_first_unit_wins() {
    let state = state(Extension::from(vec![indent_unit("    "), indent_unit(" ")]));
    assert_eq!(get_indent_unit(&state), 4);
}

#[test]
fn a_tab_unit_counts_the_tab_size() {
    let state = state(Extension::from(vec![
        indent_unit("\t"),
        facets::TAB_SIZE.of(8),
    ]));
    assert_eq!(get_indent_unit(&state), 8);
    // Tabs for whole tab stops, then spaces.
    assert_eq!(indent_string(&state, 19), "\t\t   ");
    assert_eq!(indent_string(&state, 0), "");
}

#[test]
fn a_unit_of_another_space_character() {
    let state = state(indent_unit("\u{3000}\u{3000}"));
    assert_eq!(get_indent_unit(&state), 2);
    assert_eq!(indent_string(&state, 3), "\u{3000}\u{3000}\u{3000}");
}

#[test]
#[should_panic(expected = "Invalid indent unit")]
fn rejects_a_unit_with_other_characters() {
    get_indent_unit(&state(indent_unit("  x")));
}

#[test]
#[should_panic(expected = "Invalid indent unit")]
fn rejects_a_mixed_unit() {
    get_indent_unit(&state(indent_unit(" \t")));
}

#[test]
#[should_panic(expected = "Invalid indent unit")]
fn rejects_an_empty_unit() {
    get_indent_unit(&state(indent_unit("")));
}
