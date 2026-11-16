//! The deletion, indentation and tab commands that take the view, and
//! the parts of the state ones upstream's `test-commands.ts` does not
//! reach. Upstream has no tests for these beyond its `deleteGroup*`,
//! `indentMore` and `indentLess` cases (ported in `commands.rs`), so
//! these are the port's own, from the sources.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use div::Document;
use editor_commands::{
    delete_char_backward, delete_char_backward_strict, delete_char_forward, delete_group_backward,
    delete_group_backward_view, delete_group_forward_win, delete_line_boundary_backward,
    delete_line_boundary_forward, delete_to_line_end, delete_to_line_start, indent_less,
    indent_more, insert_tab, temporarily_set_tab_focus_mode, toggle_tab_focus_mode,
};
use editor_language::indent_unit;
use editor_state::{
    facets, EditorSelection, EditorState, EditorStateConfig, Extension, RangeSet, StateCommand,
};
use editor_view::{
    command, Decoration, EditorView, EditorViewConfig, KeyBinding, KeyBindings, MarkSpec,
    WindowSize, KEYMAP,
};

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

fn view_of(document: &Document, state: EditorState) -> EditorView {
    EditorView::new(
        document,
        EditorViewConfig {
            state,
            parent: Some(document.body()),
            scroll_to: None,
            window: WindowSize {
                width: 800.0,
                height: 600.0,
            },
        },
    )
    .unwrap()
}

/// Run `command` on a view of `before`; return whether it applied.
fn run(before: &str, after: &str, command: Command, extensions: Extension) -> bool {
    let document = Document::new(800, 600).unwrap();
    let mut view = view_of(&document, mk_state(before, extensions));
    let applied = command(&mut view);
    assert_eq!(state_str(view.state()), after, "from {before:?}");
    view.destroy().unwrap();
    applied
}

fn test_cmd(before: &str, after: &str, command: Command) {
    run(before, after, command, none());
}

fn none() -> Extension {
    Extension::from(vec![])
}

fn test_state(command: StateCommand, before: &str, after: &str, extensions: Extension) {
    let state = mk_state(before, extensions);
    let mut result = state.clone();
    command(&state, &mut |tr| result = tr.state().clone());
    assert_eq!(state_str(&result), after, "from {before:?}");
}

fn atomic(from: usize, to: usize) -> Extension {
    let mark = Decoration::mark(MarkSpec::default())
        .range(from, to)
        .unwrap();
    let set = RangeSet::of(vec![mark], false);
    EditorView::atomic_ranges(move |_| set.clone())
}

#[test]
fn delete_char_backward_deletes_a_character_or_the_selection() {
    test_cmd("ab|c", "a|c", delete_char_backward);
    test_cmd("a<bc>d", "a|d", delete_char_backward);
    test_cmd("a|b|c", "|c", delete_char_backward);
    test_cmd("ab\n|cd", "ab|cd", delete_char_backward);
    assert!(!run("|ab", "|ab", delete_char_backward, none()));
}

#[test]
fn delete_char_backward_deletes_an_indentation_unit() {
    // The default unit is two spaces.
    test_cmd("    |x", "  |x", delete_char_backward);
    // To the previous multiple of the unit.
    test_cmd("   |x", "  |x", delete_char_backward);
    // A tab goes on its own.
    test_cmd("  \t|x", "  |x", delete_char_backward);
    // Only spaces before the cursor count as indentation.
    test_cmd("a   |x", "a  |x", delete_char_backward);
    // A four-space unit.
    run(
        "      |x",
        "    |x",
        delete_char_backward,
        indent_unit("    "),
    );
    // The strict form deletes one character.
    test_cmd("    |x", "   |x", delete_char_backward_strict);
}

#[test]
fn delete_char_follows_clusters() {
    // Backward, an extending character goes on its own.
    test_cmd("e\u{301}|x", "e|x", delete_char_backward);
    // Unless it is a variation selector, which takes its base along.
    test_cmd("a\u{2764}\u{fe0f}|", "a|", delete_char_backward);
    // Forward, the whole cluster goes.
    test_cmd("|e\u{301}x", "|x", delete_char_forward);
    test_cmd("ab|\ncd", "ab|cd", delete_char_forward);
    assert!(!run("ab|", "ab|", delete_char_forward, none()));
}

#[test]
fn deletion_skips_atomic_ranges() {
    test_cmd("abcd|e", "abc|e", delete_char_backward);
    run("abcd|e", "ab|e", delete_char_backward, atomic(2, 4));
    run("a|bcde", "a|cde", delete_char_forward, atomic(2, 4));
    run("ab|cde", "ab|e", delete_char_forward, atomic(2, 4));
    // A selection grows to the atomic ranges it ends in.
    run("a<bc>de", "a|e", delete_char_backward, atomic(2, 4));
    // The state command form has no view, so no atomic ranges; the view
    // form does.
    run("abcd e|f", "abcd |f", delete_group_backward_view, none());
    run("ab cd|e", "ab|e", delete_group_backward_view, atomic(2, 4));
}

#[test]
fn deleting_a_selection_announces_it() {
    let document = Document::new(800, 600).unwrap();
    let mut view = view_of(&document, mk_state("a<bc>d", none()));
    assert!(delete_char_backward(&mut view));
    assert_eq!(state_str(view.state()), "a|d");
    assert_eq!(
        view.announce_dom().text_content().unwrap(),
        "Selection deleted"
    );
    // Deleting a character announces nothing.
    let mut view2 = view_of(&document, mk_state("ab|c", none()));
    assert!(delete_char_backward(&mut view2));
    assert_eq!(view2.announce_dom().text_content().unwrap(), "");
    view.destroy().unwrap();
    view2.destroy().unwrap();
}

#[test]
fn deletion_does_nothing_in_a_read_only_state() {
    assert!(!run(
        "ab|c",
        "ab|c",
        delete_char_backward,
        facets::READ_ONLY.of(true)
    ));
    let state = mk_state("<ab>c", facets::READ_ONLY.of(true));
    assert!(!delete_group_backward(&state, &mut |_| {}));
}

#[test]
fn delete_group_forward_win_deletes_the_space_after_a_word() {
    test_cmd("|one two", "|two", delete_group_forward_win);
    test_cmd("one| two", "one|two", delete_group_forward_win);
    test_cmd("one|...  two", "one|two", delete_group_forward_win);
}

#[test]
fn delete_to_line_end_and_start() {
    test_cmd("a|bc\nd", "a|\nd", delete_to_line_end);
    test_cmd("abc|\nd", "abc|d", delete_to_line_end);
    test_cmd("ab|c", "|c", delete_to_line_start);
    test_cmd("a\n|b", "a|b", delete_to_line_start);
    assert!(!run("|ab", "|ab", delete_to_line_start, none()));
}

#[test]
fn delete_line_boundary_without_wrapping() {
    test_cmd("  ab|c", "|c", delete_line_boundary_backward);
    test_cmd("a\n|b", "a|b", delete_line_boundary_backward);
    test_cmd("a|bc\nd", "a|\nd", delete_line_boundary_forward);
    test_cmd("abc|\nd", "abc|d", delete_line_boundary_forward);
}

#[test]
fn indentation_with_a_tab_unit() {
    test_state(indent_more, "a|\nb", "\ta|\nb", indent_unit("\t"));
    test_state(indent_less, "\t\ta|", "\ta|", indent_unit("\t"));
    // With a tab size of 4, six columns less a four-column unit leave
    // two, which take spaces.
    test_state(
        indent_less,
        "\t  a|",
        "  a|",
        Extension::from(vec![indent_unit("\t"), facets::TAB_SIZE.of(4)]),
    );
}

#[test]
fn insert_tab_inserts_a_tab_or_indents() {
    test_state(insert_tab, "a|b", "a\t|b", none());
    test_state(insert_tab, "<ab>\ncd", "  <ab>\ncd", none());
}

#[test]
fn tab_focus_commands_let_tab_through() {
    let document = Document::new(800, 600).unwrap();
    let ran = Arc::new(Mutex::new(0));
    let counter = ran.clone();
    let keymap = KEYMAP.of(KeyBindings::from(vec![KeyBinding {
        key: Some("Tab".to_string()),
        run: Some(command(move |_| {
            *counter.lock().unwrap() += 1;
            true
        })),
        ..Default::default()
    }]));
    let view: Rc<RefCell<EditorView>> = EditorView::new_shared(
        &document,
        EditorViewConfig {
            state: mk_state("one", keymap),
            parent: Some(document.body()),
            scroll_to: None,
            window: WindowSize {
                width: 800.0,
                height: 600.0,
            },
        },
    )
    .unwrap();
    let tab = || {
        let mut event = div::Event::new("keydown", true, true);
        event.key_text = "Tab".to_string();
        event.key_code = 9;
        let content = view.borrow().content_dom().clone();
        document.dispatch_event(&content, &event).unwrap();
        event.default_prevented()
    };
    assert!(tab());
    assert!(toggle_tab_focus_mode(&mut view.borrow_mut()));
    assert!(!tab());
    assert!(toggle_tab_focus_mode(&mut view.borrow_mut()));
    assert!(tab());
    assert!(temporarily_set_tab_focus_mode(&mut view.borrow_mut()));
    assert!(!tab());
    assert_eq!(*ran.lock().unwrap(), 2);
    view.borrow_mut().destroy().unwrap();
}
