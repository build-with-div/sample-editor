//! The view motion commands: the cases of `@codemirror/commands` 6.11.1
//! `test/webtest-commands.ts` that do not need the view's coordinates,
//! then port-specific cases for the motion upstream does not test there.
//!
//! Upstream's `cursorLineDown`, `cursorLineUp`, `selectLineDown` and
//! `selectLineUp` cases move vertically, which needs Div's Range geometry
//! and `caretPositionFromPoint` (gaps 5 and 6); they come with that
//! motion.

use std::collections::HashMap;
use std::sync::Arc;

use div::Document;
use editor_commands::{
    cursor_char_backward, cursor_char_forward, cursor_char_left, cursor_char_right,
    cursor_group_backward, cursor_group_forward, cursor_group_forward_win, cursor_group_left,
    cursor_line_boundary_backward, cursor_line_boundary_forward, cursor_line_end,
    cursor_line_start, cursor_subword_backward, cursor_subword_forward, select_char_left,
    select_char_right, select_group_forward, select_line_boundary_backward, select_line_end,
    select_line_start,
};
use editor_state::{
    facets, EditorSelection, EditorState, EditorStateConfig, Extension, LanguageData,
    LanguageDataProvider, RangeSet,
};
use editor_view::{Decoration, EditorView, EditorViewConfig, MarkSpec, StyleSpec, WindowSize};

type Command = fn(&mut EditorView) -> bool;

/// `mkState`: `|` marks a cursor and `<...>` an undirectional range.
fn mk_state(spec: &str, extensions: Extension) -> EditorState {
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
        extensions: Extension::from(vec![extensions, facets::ALLOW_MULTIPLE_SELECTIONS.of(true)]),
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

/// `testCmd`: run `command` on a view of `before`.
fn test_cmd(before: &str, after: &str, command: Command, extensions: Extension) {
    let document = Document::new(800, 600).unwrap();
    let mut view = EditorView::new(
        &document,
        EditorViewConfig {
            state: mk_state(before, extensions),
            parent: Some(document.body()),
            scroll_to: None,
            window: WindowSize {
                width: 800.0,
                height: 600.0,
            },
        },
    )
    .unwrap();
    command(&mut view);
    assert_eq!(state_str(view.state()), after, "from {before}");
    view.destroy().unwrap();
}

fn none() -> Extension {
    Extension::from(vec![])
}

// `dashWordChar`.
fn dash_word_char() -> Extension {
    let data: LanguageData = Arc::new(HashMap::from([(
        "wordChars".to_string(),
        Arc::new("-".to_string()) as editor_state::AnyValue,
    )]));
    facets::LANGUAGE_DATA.of(LanguageDataProvider::new(move |_, _, _| vec![data.clone()]))
}

#[test]
fn cursor_subword_forward_stops_at_first_camelcase_boundary() {
    test_cmd(
        "|CamelCaseWord",
        "Camel|CaseWord",
        cursor_subword_forward,
        none(),
    );
}

#[test]
fn cursor_subword_forward_stops_at_inner_camelcase_boundary() {
    test_cmd(
        "Camel|CaseWord",
        "CamelCase|Word",
        cursor_subword_forward,
        none(),
    );
}

#[test]
fn cursor_subword_forward_stops_at_last_camelcase_boundary() {
    test_cmd(
        "CamelCase|Word",
        "CamelCaseWord|",
        cursor_subword_forward,
        none(),
    );
}

#[test]
fn cursor_subword_forward_treats_ranges_of_capitals_as_a_single_word() {
    test_cmd(
        "Eat|CSSToken",
        "EatCSS|Token",
        cursor_subword_forward,
        none(),
    );
}

#[test]
fn cursor_subword_forward_stops_at_the_end_of_word() {
    test_cmd("o|kay.", "okay|.", cursor_subword_forward, none());
}

#[test]
fn cursor_subword_forward_stops_before_underscores() {
    test_cmd("|snake_case", "snake|_case", cursor_subword_forward, none());
}

#[test]
fn cursor_subword_forward_stops_after_underscores() {
    test_cmd("snake|_case", "snake_|case", cursor_subword_forward, none());
}

#[test]
fn cursor_subword_forward_stops_before_dashes() {
    test_cmd(
        "|kebab-case",
        "kebab|-case",
        cursor_subword_forward,
        dash_word_char(),
    );
}

#[test]
fn cursor_subword_forward_stops_after_dashes() {
    test_cmd(
        "kebab|-case",
        "kebab-|case",
        cursor_subword_forward,
        dash_word_char(),
    );
}

#[test]
fn cursor_subword_forward_stops_on_dashes_at_end_of_word() {
    test_cmd(
        "one|--..",
        "one--|..",
        cursor_subword_forward,
        dash_word_char(),
    );
}

// Upstream runs this only where `Intl.Segmenter` exists.
#[test]
#[ignore = "div-gap 30: Intl.Segmenter word segmentation"]
fn cursor_subword_forward_stops_on_cjk_word_boundaries() {
    test_cmd(
        "|马在路上小跑着。",
        "马|在路上小跑着。",
        cursor_subword_forward,
        none(),
    );
    test_cmd(
        "马|在路上小跑着。",
        "马在|路上小跑着。",
        cursor_subword_forward,
        none(),
    );
    test_cmd(
        "马在|路上小跑着。",
        "马在路上|小跑着。",
        cursor_subword_forward,
        none(),
    );
}

#[test]
fn cursor_subword_backward_stops_at_camelcase_boundary() {
    test_cmd(
        "CamelCaseWord|",
        "CamelCase|Word",
        cursor_subword_backward,
        none(),
    );
}

#[test]
fn cursor_subword_backward_stops_at_last_camelcase_boundary() {
    test_cmd(
        "Camel|CaseWord",
        "|CamelCaseWord",
        cursor_subword_backward,
        none(),
    );
}

#[test]
fn cursor_subword_backward_treats_ranges_of_capitals_as_a_single_word() {
    test_cmd(
        "EatCSS|Token",
        "Eat|CSSToken",
        cursor_subword_backward,
        none(),
    );
}

#[test]
fn cursor_subword_backward_stops_at_the_end_of_word() {
    test_cmd(".o|kay", ".|okay", cursor_subword_backward, none());
}

#[test]
fn cursor_subword_backward_stops_before_underscores() {
    test_cmd(
        "snake_case|",
        "snake_|case",
        cursor_subword_backward,
        none(),
    );
}

#[test]
fn cursor_subword_backward_stops_after_underscores() {
    test_cmd(
        "snake_|case",
        "snake|_case",
        cursor_subword_backward,
        none(),
    );
}

#[test]
fn cursor_subword_backward_stops_before_dashes() {
    test_cmd(
        "kebab-case|",
        "kebab-|case",
        cursor_subword_backward,
        dash_word_char(),
    );
}

#[test]
fn cursor_subword_backward_stops_after_dashes() {
    test_cmd(
        "kebab--|case",
        "kebab|--case",
        cursor_subword_backward,
        dash_word_char(),
    );
}

#[test]
fn cursor_subword_backward_stops_on_dashes_at_end_of_word() {
    test_cmd(
        "..--one|",
        "..--|one",
        cursor_subword_backward,
        dash_word_char(),
    );
}

// Upstream runs this only where `Intl.Segmenter` exists.
#[test]
#[ignore = "div-gap 30: Intl.Segmenter word segmentation"]
fn cursor_subword_backward_stops_on_cjk_word_boundaries() {
    test_cmd(
        "马在路上小跑着|。",
        "马在路上小跑|着。",
        cursor_subword_backward,
        none(),
    );
    test_cmd(
        "马在路上小跑|着。",
        "马在路上|小跑着。",
        cursor_subword_backward,
        none(),
    );
    test_cmd(
        "马在路上|小跑着。",
        "马在|路上小跑着。",
        cursor_subword_backward,
        none(),
    );
}

fn rtl_content() -> Extension {
    EditorView::theme(
        &StyleSpec::new().nest(".cm-content", StyleSpec::new().set("direction", "rtl")),
        false,
    )
    .unwrap()
}

#[test]
fn select_char_left_selects_backwards_left_to_right_text() {
    test_cmd("ab|cd", "a<b>cd", select_char_left, none());
}

#[test]
fn select_char_left_selects_forward_in_right_to_left_text() {
    test_cmd("ال|شفرة", "ال<ش>فرة", select_char_left, rtl_content());
}

#[test]
fn select_char_left_expands_the_start_of_an_undirectional_selection() {
    test_cmd("a<bc>d", "<abc>d", select_char_left, none());
}

#[test]
fn select_char_right_expands_the_end_of_an_undirectional_selection() {
    test_cmd("a<bc>d", "a<bcd>", select_char_right, none());
}

// The cases below are not upstream tests; they check the port against
// upstream's documented behaviour.

#[test]
fn moves_by_character_across_line_breaks() {
    test_cmd("ab|\ncd", "ab\n|cd", cursor_char_right, none());
    test_cmd("ab\n|cd", "ab|\ncd", cursor_char_left, none());
    test_cmd("|ab", "|ab", cursor_char_backward, none());
    test_cmd("ab|", "ab|", cursor_char_forward, none());
    test_cmd("a<bc>d", "a|bcd", cursor_char_backward, none());
}

#[test]
fn moves_by_cluster() {
    test_cmd("|e\u{301}x", "e\u{301}|x", cursor_char_forward, none());
    test_cmd("e\u{301}|x", "|e\u{301}x", cursor_char_backward, none());
}

#[test]
fn moves_left_and_right_visually_in_right_to_left_text() {
    test_cmd("ال|شفرة", "الش|فرة", cursor_char_left, rtl_content());
    test_cmd("ال|شفرة", "ا|لشفرة", cursor_char_right, rtl_content());
}

#[test]
fn moves_by_group() {
    test_cmd("|one two", "one| two", cursor_group_forward, none());
    test_cmd("one| two", "one two|", cursor_group_forward, none());
    test_cmd("one two|", "one |two", cursor_group_backward, none());
    test_cmd("a|..b", "a..|b", cursor_group_forward, none());
    test_cmd("one |two", "|one two", cursor_group_left, none());
}

#[test]
fn moves_by_group_in_windows_style() {
    test_cmd("|one two", "one |two", cursor_group_forward_win, none());
    test_cmd("one| two", "one |two", cursor_group_forward_win, none());
}

#[test]
fn selects_by_group() {
    test_cmd("|one two", "<one> two", select_group_forward, none());
}

#[test]
fn moves_to_line_boundaries() {
    test_cmd(
        "on|e\ntwo",
        "one|\ntwo",
        cursor_line_boundary_forward,
        none(),
    );
    test_cmd(
        "one\ntw|o",
        "one\n|two",
        cursor_line_boundary_backward,
        none(),
    );
    test_cmd("on|e", "|one", cursor_line_start, none());
    test_cmd("on|e", "one|", cursor_line_end, none());
}

#[test]
fn line_boundary_backward_stops_after_indentation_first() {
    test_cmd("  on|e", "  |one", cursor_line_boundary_backward, none());
    test_cmd("  |one", "|  one", cursor_line_boundary_backward, none());
    test_cmd("  on|e", "  <on>e", select_line_boundary_backward, none());
}

#[test]
fn selects_to_line_start_and_end() {
    test_cmd("on|e\ntwo", "<on>e\ntwo", select_line_start, none());
    test_cmd("on|e\ntwo", "on<e>\ntwo", select_line_end, none());
}

#[test]
fn does_nothing_without_motion() {
    let document = Document::new(800, 600).unwrap();
    let mut view = EditorView::new(
        &document,
        EditorViewConfig {
            state: mk_state("|", none()),
            parent: Some(document.body()),
            scroll_to: None,
            window: WindowSize {
                width: 800.0,
                height: 600.0,
            },
        },
    )
    .unwrap();
    assert!(!cursor_char_forward(&mut view));
    assert!(!cursor_group_backward(&mut view));
}

fn atomic(from: usize, to: usize) -> Extension {
    let mark = Decoration::mark(MarkSpec::default())
        .range(from, to)
        .unwrap();
    let set = RangeSet::of(vec![mark], false);
    EditorView::atomic_ranges(move |_| set.clone())
}

#[test]
fn skips_atomic_ranges() {
    test_cmd("ab|cde", "abcd|e", cursor_char_forward, atomic(2, 4));
    test_cmd("abcd|e", "ab|cde", cursor_char_backward, atomic(2, 4));
    // The range's edges are outside it.
    test_cmd("a|bcde", "ab|cde", cursor_char_forward, atomic(2, 4));
}
