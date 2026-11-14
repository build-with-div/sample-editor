//! Port of the state-only cases of `@codemirror/commands` 6.11.1
//! `test/test-commands.ts`, with its `test/state.ts` helpers, followed by
//! port-specific cases for state-only commands upstream does not test.

use editor_commands::{
    copy_line_down, copy_line_up, cursor_char_backward_logical, cursor_char_forward_logical,
    cursor_doc_end, cursor_doc_start, delete_trailing_whitespace, insert_newline,
    insert_newline_keep_indent, move_line_down, move_line_up, select_all,
    select_char_forward_logical, select_doc_end, select_doc_start, select_line, simplify_selection,
    split_line, transpose_chars,
};
use editor_state::{
    facets, EditorSelection, EditorState, EditorStateConfig, Extension, StateCommand,
};

/// `mkState`: `|` marks a cursor and `<...>` an undirectional range.
fn mk_state(spec: &str) -> EditorState {
    let mut doc = String::new();
    let mut ranges = Vec::new();
    let mut rest = spec;
    while let Some(i) = rest.find(['|', '<']) {
        doc.push_str(&rest[..i]);
        if rest[i..].starts_with('|') {
            ranges.push(EditorSelection::cursor(doc.len()));
            rest = &rest[i + 1..];
        } else {
            let end = rest[i + 1..].find('>').expect("a closing >") + i + 1;
            let from = doc.len();
            doc.push_str(&rest[i + 1..end]);
            ranges.push(EditorSelection::undirectional_range(from, doc.len()));
            rest = &rest[end + 1..];
        }
    }
    doc.push_str(rest);
    EditorState::create(EditorStateConfig {
        doc: Some(doc.into()),
        selection: (!ranges.is_empty()).then(|| EditorSelection::create(ranges, 0).unwrap()),
        extensions: Extension::from(vec![facets::ALLOW_MULTIPLE_SELECTIONS.of(true)]),
    })
    .unwrap()
}

/// `stateStr`.
fn state_str(state: &EditorState) -> String {
    let mut doc = state.doc().to_string();
    for range in state.selection().ranges().iter().rev() {
        if range.is_empty() {
            doc.insert(range.from(), '|');
        } else {
            doc.insert(range.to(), '>');
            doc.insert(range.from(), '<');
        }
    }
    doc
}

fn cmd(state: &EditorState, command: StateCommand) -> EditorState {
    let mut result = state.clone();
    command(state, &mut |tr| result = tr.state().clone());
    result
}

fn test(command: StateCommand, from: &str, to: &str) {
    assert_eq!(state_str(&cmd(&mk_state(from), command)), to);
}

// insertNewlineKeepIndent

#[test]
fn insert_newline_keep_indent_keeps_indentation() {
    test(insert_newline_keep_indent, "  one|", "  one\n  |");
}

#[test]
fn insert_newline_keep_indent_keeps_zero_indentation() {
    test(insert_newline_keep_indent, "one|two", "one\n|two");
}

#[test]
fn insert_newline_keep_indent_deletes_the_selection() {
    test(
        insert_newline_keep_indent,
        "if x:\n  one<two\n  three>four",
        "if x:\n  one\n  |four",
    );
}

// deleteTrailingWhitespace

fn test_whitespace(from: &str, to: &str) {
    assert_eq!(
        cmd(&mk_state(from), delete_trailing_whitespace)
            .doc()
            .to_string(),
        to
    );
}

#[test]
fn delete_trailing_whitespace_deletes_trailing_whitespace() {
    test_whitespace("foo   ", "foo");
}

#[test]
fn delete_trailing_whitespace_checks_multiple_lines() {
    test_whitespace("one\ntwo \nthree   \n   ", "one\ntwo\nthree\n");
}

#[test]
fn delete_trailing_whitespace_can_handle_empty_lines() {
    test_whitespace("one  \n\ntwo ", "one\n\ntwo");
}

// moveLineUp

#[test]
fn move_line_up_can_move_a_line_up() {
    test(move_line_up, "one\ntwo|\nthree", "two|\none\nthree");
}

#[test]
fn move_line_up_preserves_multiple_cursors_on_a_single_line() {
    test(move_line_up, "one\nt|w|o|\n", "t|w|o|\none\n");
}

#[test]
fn move_line_up_moves_selected_blocks_as_one() {
    test(
        move_line_up,
        "one\ntwo\nthr<ee\nfour\nfive>\n",
        "one\nthr<ee\nfour\nfive>\ntwo\n",
    );
}

#[test]
fn move_line_up_moves_blocks_made_of_multiple_ranges_as_one() {
    test(
        move_line_up,
        "one\n<two\nth>ree\nfo|u<r\nfive>\n",
        "<two\nth>ree\nfo|u<r\nfive>\none\n",
    );
}

#[test]
fn move_line_up_does_not_include_a_trailing_line_after_a_range() {
    test(
        move_line_up,
        "one\n<two\nthree\n>four",
        "<two\nthree\n>one\nfour",
    );
}

// moveLineDown

#[test]
fn move_line_down_can_move_a_line_down() {
    test(move_line_down, "one\ntwo|\nthree", "one\nthree\ntwo|");
}

#[test]
fn move_line_down_preserves_multiple_cursors_on_a_single_line() {
    test(move_line_down, "one\nt|w|o|\nthree", "one\nthree\nt|w|o|");
}

#[test]
fn move_line_down_moves_selected_blocks_as_one() {
    test(
        move_line_down,
        "one\ntwo\nthr<ee\nfour\nfive>\nsix",
        "one\ntwo\nsix\nthr<ee\nfour\nfive>",
    );
}

#[test]
fn move_line_down_moves_blocks_made_of_multiple_ranges_as_one() {
    test(
        move_line_down,
        "one\n<two\nth>ree\nfo|u<r\nfive>\nsix\n",
        "one\nsix\n<two\nth>ree\nfo|u<r\nfive>\n",
    );
}

#[test]
fn move_line_down_does_not_include_a_trailing_line_after_a_range() {
    test(
        move_line_down,
        "one\n<two\nthree\n>four\n",
        "one\nfour\n<two\nthree\n>",
    );
}

#[test]
fn move_line_down_clips_the_selection_when_moving_to_the_end_of_the_doc() {
    test(
        move_line_down,
        "one\n<two\nthree\n>four",
        "one\nfour\n<two\nthree>",
    );
}

// Port-specific cases: upstream has no tests for these state-only
// commands. The expectations follow the upstream source.

#[test]
fn cursor_char_logical_moves_by_cluster_and_across_lines() {
    test(cursor_char_forward_logical, "a|b\nc", "ab|\nc");
    test(cursor_char_forward_logical, "ab|\nc", "ab\n|c");
    test(cursor_char_forward_logical, "|e\u{301}x", "e\u{301}|x");
    test(cursor_char_forward_logical, "<ab>c", "ab|c");
    test(cursor_char_backward_logical, "ab\n|c", "ab|\nc");
    test(cursor_char_backward_logical, "a<bc>", "a|bc");
}

#[test]
fn select_char_forward_logical_extends_the_head() {
    test(select_char_forward_logical, "a|bc", "a<b>c");
    let once = cmd(&mk_state("a|bc"), select_char_forward_logical);
    assert_eq!(state_str(&cmd(&once, select_char_forward_logical)), "a<bc>");
}

#[test]
fn doc_start_and_end_commands() {
    test(cursor_doc_start, "ab|c", "|abc");
    test(cursor_doc_end, "ab|c", "abc|");
    test(select_doc_start, "ab|c", "<ab>c");
    test(select_doc_end, "a|bc", "a<bc>");
}

#[test]
fn select_all_selects_the_document() {
    test(select_all, "a|b\nc", "<ab\nc>");
}

#[test]
fn select_line_covers_whole_lines() {
    test(select_line, "one\nt|wo\nthree", "one\n<two\n>three");
    test(select_line, "one\ntw|o", "one\n<two>");
}

#[test]
fn simplify_selection_keeps_the_main_range_or_a_cursor() {
    test(simplify_selection, "a|b|c", "a|bc");
    test(simplify_selection, "<ab>c", "ab|c");
    let state = mk_state("a|bc");
    assert!(!simplify_selection(&state, &mut |_| {}));
}

#[test]
fn split_line_leaves_the_cursor_before_the_break() {
    test(split_line, "ab|cd", "ab|\ncd");
    test(split_line, "a<bc>d", "a|\nd");
}

#[test]
fn transpose_chars_swaps_around_the_cursor() {
    test(transpose_chars, "ab|cd", "acb|d");
    test(transpose_chars, "ab\n|cd", "abc\n|d");
    test(transpose_chars, "ab|\ncd", "a\nb|cd");
    let state = mk_state("|ab");
    assert!(!transpose_chars(&state, &mut |_| {}));
}

#[test]
fn copy_line_keeps_the_selection_in_the_named_copy() {
    test(copy_line_down, "o|ne\ntwo", "one\no|ne\ntwo");
    test(copy_line_up, "o|ne\ntwo", "o|ne\none\ntwo");
}

#[test]
fn insert_newline_replaces_the_selection() {
    test(insert_newline, "ab|cd", "ab\n|cd");
    test(insert_newline, "a<bc>d", "a\n|d");
}
