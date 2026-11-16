//! Port of `@codemirror/commands` 6.11.1 `test/test-comment.ts`.
//!
//! Its last case ("toggles line comment in multi-language doc") is not
//! ported: it reads the comment tokens of JavaScript inside an HTML
//! script from `@codemirror/lang-html`'s nested syntax tree, which comes
//! with `@codemirror/language`'s tree (M8).

use std::collections::HashMap;
use std::sync::Arc;

use editor_commands::{
    toggle_block_comment, toggle_block_comment_by_line, toggle_line_comment, BlockTokens,
    CommentTokens,
};
use editor_state::{
    facets, AnyValue, EditorSelection, EditorState, EditorStateConfig, Extension, LanguageData,
    LanguageDataProvider, StateCommand,
};

/// `s`: a state of `doc`, where each `|` marks a cursor (when there is
/// one) or one end of a range (pairs of them), with `config` as the
/// `"commentTokens"` language data.
fn s(doc: &str, config: &CommentTokens) -> EditorState {
    let mut doc = doc.to_string();
    let mut markers = Vec::new();
    while let Some(pos) = doc.find('|') {
        markers.push(pos);
        doc.remove(pos);
    }
    let ranges = if markers.len() == 1 {
        vec![EditorSelection::cursor(markers[0])]
    } else {
        assert!(
            markers.len() % 2 == 0,
            "Markers for multiple selections need to be even."
        );
        let mut ranges: Vec<_> = markers
            .chunks(2)
            .map(|pair| EditorSelection::range(pair[0], pair[1]))
            .collect();
        if ranges.is_empty() {
            ranges.push(EditorSelection::cursor(0));
        }
        ranges
    };
    let data: LanguageData = Arc::new(HashMap::from([(
        "commentTokens".to_string(),
        Arc::new(config.clone()) as AnyValue,
    )]));
    EditorState::create(EditorStateConfig {
        doc: Some(doc.into()),
        selection: Some(EditorSelection::create(ranges, 0).unwrap()),
        extensions: Extension::from(vec![
            facets::ALLOW_MULTIPLE_SELECTIONS.of(true),
            facets::LANGUAGE_DATA.of(LanguageDataProvider::new(move |_, _, _| vec![data.clone()])),
        ]),
    })
    .unwrap()
}

/// `same`: equal documents and selections, as upstream compares the
/// selections' JSON (anchors, heads and the main index).
fn same(actual: &EditorState, expected: &EditorState, step: usize) {
    assert_eq!(
        actual.doc().to_string(),
        expected.doc().to_string(),
        "document after toggle {step}"
    );
    let ranges = |state: &EditorState| -> Vec<(usize, usize)> {
        state
            .selection()
            .ranges()
            .iter()
            .map(|r| (r.anchor(), r.head()))
            .collect()
    };
    assert_eq!(
        (ranges(actual), actual.selection().main_index()),
        (ranges(expected), expected.selection().main_index()),
        "selection after toggle {step}"
    );
}

/// `checkToggleChain`: toggling `docs[0]` gives each next document, and
/// toggling the last gives the one before it again.
fn check_toggle_chain(toggle: StateCommand, config: &CommentTokens, docs: &[String]) {
    let mut state = s(&docs[0], config);
    for i in 1..=docs.len() {
        let current = state.clone();
        toggle(&current, &mut |tr| state = tr.state().clone());
        let expected = if i == docs.len() { docs.len() - 2 } else { i };
        same(&state, &s(&docs[expected], config), i);
    }
}

// Line comments, with `{k}` standing for the token.
fn check_line(k: &str, docs: &[&str]) {
    let docs: Vec<String> = docs.iter().map(|doc| doc.replace("{k}", k)).collect();
    let config = CommentTokens {
        line: Some(k.into()),
        block: None,
    };
    check_toggle_chain(toggle_line_comment, &config, &docs);
}

fn line_toggles_in_an_empty_single_selection(k: &str) {
    check_line(
        k,
        &[
            "\nline 1\n  {k} {k} {k} {k}line| 2\nline 3\n",
            "\nline 1\n  {k} {k} {k}line| 2\nline 3\n",
            "\nline 1\n  {k} {k}line| 2\nline 3\n",
            "\nline 1\n  {k}line| 2\nline 3\n",
            "\nline 1\n  line| 2\nline 3\n",
            "\nline 1\n  {k} line| 2\nline 3\n",
        ],
    );
    check_line(
        k,
        &[
            "\nline 1\n  {k}line 2|\nline 3\n",
            "\nline 1\n  line 2|\nline 3\n",
            "\nline 1\n  {k} line 2|\nline 3\n",
        ],
    );
    check_line(
        k,
        &[
            "\nline 1\n|  {k}line 2\nline 3\n",
            "\nline 1\n|  line 2\nline 3\n",
            "\nline 1\n|  {k} line 2\nline 3\n",
        ],
    );
    check_line(
        k,
        &[
            "\nline 1\n|{k}\nline 3\n",
            "\nline 1\n|\nline 3\n",
            "\nline 1\n{k} |\nline 3\n",
        ],
    );
    check_line(
        k,
        &[
            "\nline 1\n line 2\nline 3\n|{k}",
            "\nline 1\n line 2\nline 3\n|",
            "\nline 1\n line 2\nline 3\n{k} |",
        ],
    );
}

fn line_toggles_in_a_single_line_with_the_cursor_at_its_start(k: &str) {
    check_line(
        k,
        &[
            "line 1\n  |line 2\nline 3\n",
            "line 1\n  {k} |line 2\nline 3\n",
        ],
    );
}

fn line_toggles_in_a_single_line_selection(k: &str) {
    check_line(
        k,
        &[
            "line 1\n  {k}li|ne |2\nline 3\n",
            "line 1\n  li|ne |2\nline 3\n",
            "line 1\n  {k} li|ne |2\nline 3\n",
        ],
    );
}

fn line_toggles_in_a_multi_line_selection(k: &str) {
    check_line(
        k,
        &[
            "\n  {k}lin|e 1\n  {k}  line 2\n  {k} line |3\n",
            "\n  lin|e 1\n   line 2\n  line |3\n",
            "\n  {k} lin|e 1\n  {k}  line 2\n  {k} line |3\n",
        ],
    );
    check_line(
        k,
        &[
            "\n  {k}lin|e 1\n  {k}  line 2\n   line 3\n  {k} li|ne 4\n",
            "\n  {k} {k}lin|e 1\n  {k} {k}  line 2\n  {k}  line 3\n  {k} {k} li|ne 4\n",
        ],
    );
    check_line(
        k,
        &[
            "\n  {k} lin|e 1\n\n  {k} line |3\n",
            "\n  lin|e 1\n\n  line |3\n",
        ],
    );
    check_line(
        k,
        &[
            "\n  {k} lin|e 1\n     \n  {k} line |3\n",
            "\n  lin|e 1\n     \n  line |3\n",
        ],
    );
    check_line(
        k,
        &["\n|\n  {k} line 2\n    | \n", "\n|\n  line 2\n    | \n"],
    );
    check_line(k, &["\n|\n\n    | \n", "\n|\n\n    | \n"]);
}

fn line_toggles_in_a_multi_line_multi_range_selection(k: &str) {
    check_line(
        k,
        &[
            "\n  lin|e 1\n  line |2\n  line 3\n  l|ine 4\n  line| 5\n",
            "\n  {k} lin|e 1\n  {k} line |2\n  line 3\n  {k} l|ine 4\n  {k} line| 5\n",
        ],
    );
}

fn line_handles_multiple_selections_on_one_line(k: &str) {
    check_line(k, &["|line| |with| |ranges|", "{k} |line| |with| |ranges|"]);
}

fn line_leaves_out_lines_a_selection_range_ends_at(k: &str) {
    check_line(
        k,
        &[
            "line| 1\nline 2\n|line 3",
            "{k} line| 1\n{k} line 2\n|line 3",
        ],
    );
}

fn line_leaves_empty_lines_alone(k: &str) {
    check_line(k, &["line| 1\n\nline 3|", "{k} line| 1\n\n{k} line 3|"]);
}

fn line_comments_empty_lines_with_a_cursor(k: &str) {
    check_line(k, &["|\nline 2", "{k} |\nline 2"]);
}

macro_rules! line_comment_tests {
    ($module:ident, $token:expr) => {
        mod $module {
            #[test]
            fn toggles_in_an_empty_single_selection() {
                super::line_toggles_in_an_empty_single_selection($token);
            }
            #[test]
            fn toggles_in_a_single_line_with_the_cursor_at_its_start() {
                super::line_toggles_in_a_single_line_with_the_cursor_at_its_start($token);
            }
            #[test]
            fn toggles_in_a_single_line_selection() {
                super::line_toggles_in_a_single_line_selection($token);
            }
            #[test]
            fn toggles_in_a_multi_line_selection() {
                super::line_toggles_in_a_multi_line_selection($token);
            }
            #[test]
            fn toggles_in_a_multi_line_multi_range_selection() {
                super::line_toggles_in_a_multi_line_multi_range_selection($token);
            }
            #[test]
            fn handles_multiple_selections_on_one_line() {
                super::line_handles_multiple_selections_on_one_line($token);
            }
            #[test]
            fn leaves_out_lines_a_selection_range_ends_at() {
                super::line_leaves_out_lines_a_selection_range_ends_at($token);
            }
            #[test]
            fn leaves_empty_lines_alone() {
                super::line_leaves_empty_lines_alone($token);
            }
            #[test]
            fn comments_empty_lines_with_a_cursor() {
                super::line_comments_empty_lines_with_a_cursor($token);
            }
        }
    };
}

line_comment_tests!(line_comments_slashes, "//");
line_comment_tests!(line_comments_hash, "#");

// Block comments, with `{o}` and `{c}` standing for the tokens.
fn check_block(command: StateCommand, o: &str, c: &str, docs: &[&str]) {
    let docs: Vec<String> = docs
        .iter()
        .map(|doc| doc.replace("{o}", o).replace("{c}", c))
        .collect();
    let config = CommentTokens {
        line: None,
        block: Some(BlockTokens {
            open: o.into(),
            close: c.into(),
        }),
    };
    check_toggle_chain(command, &config, &docs);
}

fn block_toggles_in_a_multi_line_selection(o: &str, c: &str) {
    check_block(
        toggle_block_comment,
        o,
        c,
        &[
            "\n  lin|e 1\n  line 2\n  line 3\n  line |4\n  line 5\n",
            "\n  lin{o} |e 1\n  line 2\n  line 3\n  line | {c}4\n  line 5\n",
        ],
    );
}

fn block_toggles_in_a_multi_line_multi_range_selection(o: &str, c: &str) {
    check_block(
        toggle_block_comment,
        o,
        c,
        &[
            "\n  lin|e 1\n  line |2\n  l|ine 3\n  line 4\n  line |5\n",
            "\n  lin{o} |e 1\n  line | {c}2\n  l{o} |ine 3\n  line 4\n  line | {c}5\n",
        ],
    );
}

fn block_toggles_comments_inside_the_selection(o: &str, c: &str) {
    check_block(
        toggle_block_comment,
        o,
        c,
        &[
            "|{o} one\ntwo {c}| three",
            "|one\ntwo| three",
            "{o} |one\ntwo| {c} three",
        ],
    );
}

fn block_comments_the_entire_line(o: &str, c: &str) {
    check_block(
        toggle_block_comment_by_line,
        o,
        c,
        &["one|\ntwo", "{o} one| {c}\ntwo"],
    );
}

fn block_comments_multiple_lines(o: &str, c: &str) {
    check_block(
        toggle_block_comment_by_line,
        o,
        c,
        &["on|e\nt|wo", "{o} on|e\nt|wo {c}"],
    );
}

fn block_joins_selected_blocks_of_lines(o: &str, c: &str) {
    check_block(
        toggle_block_comment_by_line,
        o,
        c,
        &["on|e\nt|w|o\nth|ree", "{o} on|e\nt|w|o\nth|ree {c}"],
    );
}

fn block_leaves_out_lines_the_selection_stops_at_the_start_of(o: &str, c: &str) {
    check_block(
        toggle_block_comment_by_line,
        o,
        c,
        &["|one\n|two", "{o} |one {c}\n|two"],
    );
}

fn block_includes_lines_with_a_cursor_at_their_start(o: &str, c: &str) {
    check_block(
        toggle_block_comment_by_line,
        o,
        c,
        &["|one\ntwo", "|{o} one {c}\ntwo"],
    );
}

macro_rules! block_comment_tests {
    ($module:ident, $open:expr, $close:expr) => {
        mod $module {
            #[test]
            fn toggles_in_a_multi_line_selection() {
                super::block_toggles_in_a_multi_line_selection($open, $close);
            }
            #[test]
            fn toggles_in_a_multi_line_multi_range_selection() {
                super::block_toggles_in_a_multi_line_multi_range_selection($open, $close);
            }
            #[test]
            fn toggles_comments_inside_the_selection() {
                super::block_toggles_comments_inside_the_selection($open, $close);
            }
            #[test]
            fn comments_the_entire_line() {
                super::block_comments_the_entire_line($open, $close);
            }
            #[test]
            fn comments_multiple_lines() {
                super::block_comments_multiple_lines($open, $close);
            }
            #[test]
            fn joins_selected_blocks_of_lines() {
                super::block_joins_selected_blocks_of_lines($open, $close);
            }
            #[test]
            fn leaves_out_lines_the_selection_stops_at_the_start_of() {
                super::block_leaves_out_lines_the_selection_stops_at_the_start_of($open, $close);
            }
            #[test]
            fn includes_lines_with_a_cursor_at_their_start() {
                super::block_includes_lines_with_a_cursor_at_their_start($open, $close);
            }
        }
    };
}

block_comment_tests!(block_comments_c, "/*", "*/");
block_comment_tests!(block_comments_html, "<!--", "-->");
