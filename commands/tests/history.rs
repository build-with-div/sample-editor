//! Port of `@codemirror/commands` 6.11.1 `test/test-history.ts`.

use editor_commands::{
    history, inverted_effects_fn, join_to_event_fn, redo, redo_depth, redo_selection, undo,
    undo_depth, undo_selection, HistoryConfig, Isolate, INVERTED_EFFECTS, ISOLATE_HISTORY,
};
use editor_state::{
    facets, ChangeDesc, ChangeSpec, EditorSelection, EditorState, EditorStateConfig, Extension,
    StateCommand, StateEffect, StateEffectType, StateField, StateFieldSpec, Transaction,
    TransactionSpec,
};

fn mk_state(config: HistoryConfig, doc: &str) -> EditorState {
    EditorState::create(EditorStateConfig {
        extensions: Extension::from(vec![
            history(config),
            facets::ALLOW_MULTIPLE_SELECTIONS.of(true),
        ]),
        doc: Some(doc.into()),
        ..Default::default()
    })
    .unwrap()
}

fn mk() -> EditorState {
    mk_state(HistoryConfig::default(), "")
}

fn update(state: &EditorState, spec: TransactionSpec) -> EditorState {
    state.update(spec).unwrap().state().clone()
}

fn changes(changes: ChangeSpec) -> TransactionSpec {
    TransactionSpec {
        changes: Some(changes),
        ..Default::default()
    }
}

fn select(anchor: usize, head: usize) -> TransactionSpec {
    TransactionSpec {
        selection: Some(EditorSelection::single(anchor, head)),
        ..Default::default()
    }
}

fn isolate(isolate: Isolate) -> TransactionSpec {
    TransactionSpec {
        annotations: vec![ISOLATE_HISTORY.of(isolate)],
        ..Default::default()
    }
}

fn type_at(state: &EditorState, text: &str, at: usize) -> EditorState {
    update(state, changes(ChangeSpec::insert(at, text)))
}

fn type_(state: &EditorState, text: &str) -> EditorState {
    type_at(state, text, state.doc().len())
}

fn timed_type(state: &EditorState, text: &str, at_time: f64) -> EditorState {
    update(
        state,
        TransactionSpec {
            annotations: vec![Transaction::TIME.of(at_time)],
            ..changes(ChangeSpec::insert(state.doc().len(), text))
        },
    )
}

fn receive(state: &EditorState, text: &str, from: usize, to: usize) -> EditorState {
    update(
        state,
        TransactionSpec {
            annotations: vec![Transaction::ADD_TO_HISTORY.of(false)],
            ..changes(ChangeSpec::replace(from, to, text))
        },
    )
}

fn command_ok(state: &EditorState, cmd: StateCommand, success: bool) -> EditorState {
    let mut result = state.clone();
    assert_eq!(cmd(state, &mut |tr| result = tr.state().clone()), success);
    result
}

fn command(state: &EditorState, cmd: StateCommand) -> EditorState {
    command_ok(state, cmd, true)
}

fn doc(state: &EditorState) -> String {
    state.doc().to_string()
}

fn head(state: &EditorState) -> usize {
    state.selection().main().head()
}

#[test]
fn allows_to_undo_a_change() {
    let mut state = mk();
    state = type_(&state, "newtext");
    state = command(&state, undo);
    assert_eq!(doc(&state), "");
}

#[test]
fn allows_to_undo_nearby_changes_in_one_change() {
    let mut state = mk();
    state = type_(&state, "new");
    state = type_(&state, "text");
    state = command(&state, undo);
    assert_eq!(doc(&state), "");
}

#[test]
fn allows_to_redo_a_change() {
    let mut state = mk();
    state = type_(&state, "newtext");
    state = command(&state, undo);
    state = command(&state, redo);
    assert_eq!(doc(&state), "newtext");
}

#[test]
fn allows_to_redo_nearby_changes_in_one_change() {
    let mut state = mk();
    state = type_(&state, "new");
    state = type_(&state, "text");
    state = command(&state, undo);
    state = command(&state, redo);
    assert_eq!(doc(&state), "newtext");
}

#[test]
fn puts_the_cursor_after_the_change_on_redo() {
    let mut state = mk_state(HistoryConfig::default(), "one\n\ntwo");
    state = update(
        &state,
        TransactionSpec {
            selection: Some(EditorSelection::single(4, 4)),
            ..changes(ChangeSpec::insert(3, "!"))
        },
    );
    let end = state.doc().len();
    state = update(&state, select(end, end));
    state = command(&state, undo);
    state = command(&state, redo);
    assert_eq!(head(&state), 4);
}

#[test]
fn restores_selection_on_undo_redo_undo() {
    let mut state = mk_state(HistoryConfig::default(), "1\n2\n3");
    state = update(&state, select(5, 5));
    state = update(
        &state,
        TransactionSpec {
            selection: Some(EditorSelection::single(6, 6)),
            ..changes(ChangeSpec::insert(5, "."))
        },
    );
    state = update(&state, select(0, 0));
    state = command(&state, undo);
    assert_eq!(head(&state), 5);
    state = update(&state, select(0, 0));
    state = command(&state, redo);
    assert_eq!(head(&state), 6);
    state = update(&state, select(0, 0));
    state = command(&state, undo);
    assert_eq!(head(&state), 5);
}

#[test]
fn tracks_multiple_levels_of_history() {
    let mut state = mk_state(HistoryConfig::default(), "one");
    state = type_(&state, "new");
    state = type_(&state, "text");
    state = type_at(&state, "some", 0);
    assert_eq!(doc(&state), "someonenewtext");
    state = command(&state, undo);
    assert_eq!(doc(&state), "onenewtext");
    state = command(&state, undo);
    assert_eq!(doc(&state), "one");
    state = command(&state, redo);
    assert_eq!(doc(&state), "onenewtext");
    state = command(&state, redo);
    assert_eq!(doc(&state), "someonenewtext");
    state = command(&state, undo);
    assert_eq!(doc(&state), "onenewtext");
}

#[test]
fn starts_a_new_event_when_new_group_delay_elapses() {
    let config = HistoryConfig {
        new_group_delay: Some(1000.0),
        ..Default::default()
    };
    let mut state = mk_state(config, "");
    state = timed_type(&state, "a", 1000.0);
    state = timed_type(&state, "b", 1600.0);
    assert_eq!(undo_depth(&state), 1);
    state = timed_type(&state, "c", 2700.0);
    assert_eq!(undo_depth(&state), 2);
    state = command(&state, undo);
    state = timed_type(&state, "d", 2800.0);
    assert_eq!(undo_depth(&state), 2);
}

#[test]
fn supports_a_custom_join_predicate() {
    let config = HistoryConfig {
        join_to_event: Some(join_to_event_fn(|tr, adj| {
            if !adj {
                return false;
            }
            let mut space = false;
            tr.changes().iter_changes(
                |_, _, _, _, text| {
                    if !text.is_empty() && text.slice_string(0, 1) == " " {
                        space = true;
                    }
                },
                false,
            );
            !space
        })),
        ..Default::default()
    };
    let mut state = mk_state(config, "");
    for ch in ["a", "b", " ", "c", "d"] {
        state = type_(&state, ch);
    }
    assert_eq!(doc(&state), "ab cd");
    state = command(&state, undo);
    assert_eq!(doc(&state), "ab");
    state = command(&state, undo);
    assert_eq!(doc(&state), "");
}

#[test]
fn allows_changes_that_arent_part_of_the_history() {
    let mut state = mk();
    state = type_(&state, "hello");
    state = receive(&state, "oops", 0, 0);
    state = receive(&state, "!", 9, 9);
    state = command(&state, undo);
    assert_eq!(doc(&state), "oops!");
}

#[test]
fn doesnt_get_confused_by_an_undo_not_adding_any_redo_item() {
    let mut state = mk_state(HistoryConfig::default(), "ab");
    state = type_at(&state, "cd", 1);
    state = receive(&state, "123", 0, 4);
    state = command_ok(&state, undo, false);
    command_ok(&state, redo, false);
}

#[test]
fn accurately_maps_changes_through_each_other() {
    let mut state = mk_state(HistoryConfig::default(), "123");
    state = update(
        &state,
        changes(ChangeSpec::List(vec![
            ChangeSpec::replace(0, 1, "ab"),
            ChangeSpec::replace(1, 2, "cd"),
            ChangeSpec::replace(2, 3, "ef"),
        ])),
    );
    state = receive(&state, "!!!!!!!!", 2, 2);
    state = command(&state, undo);
    state = command(&state, redo);
    assert_eq!(doc(&state), "ab!!!!!!!!cdef");
}

#[test]
fn can_handle_complex_editing_sequences() {
    let mut state = mk();
    state = type_(&state, "hello");
    state = update(&state, isolate(Isolate::Before));
    state = type_(&state, "!");
    state = receive(&state, "....", 0, 0);
    state = type_at(&state, "\n\n", 2);
    assert_eq!(doc(&state), "..\n\n..hello!");
    state = receive(&state, "\n\n", 1, 1);
    state = command(&state, undo);
    state = command(&state, undo);
    assert_eq!(doc(&state), ".\n\n...hello");
    state = command(&state, undo);
    assert_eq!(doc(&state), ".\n\n...");
}

#[test]
fn supports_overlapping_edits() {
    let mut state = mk();
    state = type_(&state, "hello");
    state = update(&state, isolate(Isolate::Before));
    state = update(&state, changes(ChangeSpec::delete(0, 5)));
    assert_eq!(doc(&state), "");
    state = command(&state, undo);
    assert_eq!(doc(&state), "hello");
    state = command(&state, undo);
    assert_eq!(doc(&state), "");
}

#[test]
fn supports_overlapping_edits_that_arent_collapsed() {
    let mut state = mk();
    state = receive(&state, "h", 0, 0);
    state = type_(&state, "ello");
    state = update(&state, isolate(Isolate::Before));
    state = update(&state, changes(ChangeSpec::delete(0, 5)));
    assert_eq!(doc(&state), "");
    state = command(&state, undo);
    assert_eq!(doc(&state), "hello");
    state = command(&state, undo);
    assert_eq!(doc(&state), "h");
}

#[test]
fn supports_overlapping_unsynced_deletes() {
    let mut state = mk();
    state = type_(&state, "hi");
    state = update(&state, isolate(Isolate::Before));
    state = type_(&state, "hello");
    state = update(
        &state,
        TransactionSpec {
            annotations: vec![Transaction::ADD_TO_HISTORY.of(false)],
            ..changes(ChangeSpec::delete(0, 7))
        },
    );
    assert_eq!(doc(&state), "");
    state = command_ok(&state, undo, false);
    assert_eq!(doc(&state), "");
}

#[test]
fn can_go_back_and_forth_through_history_multiple_times() {
    let mut state = mk();
    state = type_(&state, "one");
    state = type_(&state, " two");
    state = update(&state, isolate(Isolate::Before));
    state = type_(&state, " three");
    state = type_at(&state, "zero ", 0);
    state = update(&state, isolate(Isolate::Before));
    state = type_at(&state, "\n\n", 0);
    state = type_at(&state, "top", 0);
    for i in 0..6 {
        let re = i % 2 == 1;
        for _ in 0..4 {
            state = command(&state, if re { redo } else { undo });
        }
        assert_eq!(
            doc(&state),
            if re { "top\n\nzero one two three" } else { "" }
        );
    }
}

#[test]
fn supports_non_tracked_changes_next_to_tracked_changes() {
    let mut state = mk();
    state = type_(&state, "o");
    state = type_at(&state, "\n\n", 0);
    state = receive(&state, "zzz", 3, 3);
    state = command(&state, undo);
    assert_eq!(doc(&state), "zzz");
}

#[test]
fn can_go_back_and_forth_through_history_when_preserving_items() {
    let mut state = mk();
    state = type_(&state, "one");
    state = type_(&state, " two");
    state = update(&state, isolate(Isolate::Before));
    let end = state.doc().len();
    state = receive(&state, "xxx", end, end);
    state = type_(&state, " three");
    state = type_at(&state, "zero ", 0);
    state = update(&state, isolate(Isolate::Before));
    state = type_at(&state, "\n\n", 0);
    state = type_at(&state, "top", 0);
    state = receive(&state, "yyy", 0, 0);
    for _ in 0..3 {
        for _ in 0..4 {
            state = command(&state, undo);
        }
        assert_eq!(doc(&state), "yyyxxx");
        for _ in 0..4 {
            state = command(&state, redo);
        }
        assert_eq!(doc(&state), "yyytop\n\nzero one twoxxx three");
    }
}

#[test]
fn restores_selection_on_undo() {
    let mut state = mk();
    state = type_(&state, "hi");
    state = update(&state, isolate(Isolate::Before));
    state = update(&state, select(0, 2));
    let selection = state.selection().clone();
    state = update(
        &state,
        changes(ChangeSpec::List(vec![
            ChangeSpec::delete(0, 2),
            ChangeSpec::insert(0, "hello"),
        ])),
    );
    let selection2 = state.selection().clone();
    state = command(&state, undo);
    assert!(state.selection().eq_with(&selection, false));
    state = command(&state, redo);
    assert!(state.selection().eq_with(&selection2, false));
}

#[test]
fn restores_the_selection_before_the_first_change_in_an_item() {
    let mut state = mk();
    state = update(
        &state,
        TransactionSpec {
            selection: Some(EditorSelection::single(1, 1)),
            ..changes(ChangeSpec::insert(0, "a"))
        },
    );
    state = update(
        &state,
        TransactionSpec {
            selection: Some(EditorSelection::single(2, 2)),
            ..changes(ChangeSpec::insert(1, "b"))
        },
    );
    state = command(&state, undo);
    assert_eq!(doc(&state), "");
    assert_eq!(state.selection().main().anchor(), 0);
}

#[test]
fn doesnt_merge_document_changes_if_theres_a_selection_change_in_between() {
    let mut state = mk();
    state = type_(&state, "hi");
    state = update(&state, select(0, 2));
    state = update(&state, state.replace_selection("hello"));
    assert_eq!(undo_depth(&state), 2);
}

#[test]
fn rebases_selection_on_undo() {
    let mut state = mk();
    state = type_(&state, "hi");
    state = update(&state, isolate(Isolate::Before));
    state = update(&state, select(0, 2));
    state = type_at(&state, "hello", 0);
    state = receive(&state, "---", 0, 0);
    state = command(&state, undo);
    assert_eq!(state.selection().ranges()[0].head(), 5);
}

#[test]
fn supports_querying_for_the_undo_and_redo_depth() {
    let mut state = mk();
    state = type_(&state, "a");
    assert_eq!(undo_depth(&state), 1);
    assert_eq!(redo_depth(&state), 0);
    state = receive(&state, "b", 0, 0);
    assert_eq!(undo_depth(&state), 1);
    assert_eq!(redo_depth(&state), 0);
    state = command(&state, undo);
    assert_eq!(undo_depth(&state), 0);
    assert_eq!(redo_depth(&state), 1);
    state = command(&state, redo);
    assert_eq!(undo_depth(&state), 1);
    assert_eq!(redo_depth(&state), 0);
}

#[test]
fn all_functions_gracefully_handle_editor_states_without_history() {
    let state = EditorState::create(EditorStateConfig::default()).unwrap();
    assert_eq!(undo_depth(&state), 0);
    assert_eq!(redo_depth(&state), 0);
    command_ok(&state, undo, false);
    command_ok(&state, redo, false);
}

#[test]
fn truncates_history() {
    let config = HistoryConfig {
        min_depth: Some(10),
        ..Default::default()
    };
    let mut state = mk_state(config, "");
    for _ in 0..40 {
        state = type_(&state, "a");
        state = update(&state, isolate(Isolate::Before));
    }
    assert!(undo_depth(&state) < 40);
}

#[test]
fn doesnt_undo_selection_only_transactions() {
    let mut state = mk_state(HistoryConfig::default(), "abc");
    assert_eq!(head(&state), 0);
    state = update(&state, select(2, 2));
    state = command_ok(&state, undo, false);
    assert_eq!(head(&state), 2);
}

#[test]
fn isolates_transactions_when_asked_to() {
    let mut state = mk();
    let step = |state: &EditorState, at: usize, text: &str, iso: Option<Isolate>| {
        update(
            state,
            TransactionSpec {
                annotations: iso.map(|i| ISOLATE_HISTORY.of(i)).into_iter().collect(),
                ..changes(ChangeSpec::insert(at, text))
            },
        )
    };
    state = step(&state, 0, "a", Some(Isolate::After));
    state = step(&state, 1, "a", None);
    state = step(&state, 2, "c", Some(Isolate::After));
    state = step(&state, 3, "d", None);
    state = step(&state, 4, "e", Some(Isolate::Full));
    state = step(&state, 5, "f", None);
    assert_eq!(undo_depth(&state), 5);
}

#[test]
fn can_group_events_around_a_non_history_transaction() {
    let mut state = mk();
    state = update(&state, changes(ChangeSpec::insert(0, "a")));
    state = update(
        &state,
        TransactionSpec {
            annotations: vec![Transaction::ADD_TO_HISTORY.of(false)],
            ..changes(ChangeSpec::insert(1, "b"))
        },
    );
    state = update(&state, changes(ChangeSpec::insert(1, "c")));
    state = command(&state, undo);
    assert_eq!(doc(&state), "b");
}

#[test]
fn properly_maps_selections_through_non_history_changes() {
    let mut state = mk_state(HistoryConfig::default(), "abc");
    let cursors = (0..3).map(EditorSelection::cursor).collect();
    state = update(
        &state,
        TransactionSpec {
            selection: Some(EditorSelection::create(cursors, 0).unwrap()),
            ..Default::default()
        },
    );
    state = update(&state, changes(ChangeSpec::replace(0, 3, "d")));
    state = update(
        &state,
        TransactionSpec {
            annotations: vec![Transaction::ADD_TO_HISTORY.of(false)],
            ..changes(ChangeSpec::List(vec![
                ChangeSpec::insert(0, "x"),
                ChangeSpec::insert(1, "y"),
            ]))
        },
    );
    state = command(&state, undo);
    assert_eq!(doc(&state), "xabcy");
    let froms: Vec<usize> = state
        .selection()
        .ranges()
        .iter()
        .map(|r| r.from())
        .collect();
    assert_eq!(froms, [0, 2, 3]);
}

#[test]
fn restores_selection_on_redo() {
    let mut state = mk_state(HistoryConfig::default(), "a\nb\nc\n");
    let cursors = [1, 3, 5].into_iter().map(EditorSelection::cursor).collect();
    state = update(
        &state,
        TransactionSpec {
            selection: Some(EditorSelection::create(cursors, 0).unwrap()),
            ..Default::default()
        },
    );
    state = update(&state, state.replace_selection("-"));
    state = update(&state, select(0, 0));
    state = command(&state, undo);
    state = update(&state, select(0, 0));
    state = command(&state, redo);
    let heads: Vec<usize> = state
        .selection()
        .ranges()
        .iter()
        .map(|r| r.head())
        .collect();
    assert_eq!(heads, [2, 5, 8]);
}

// undoSelection

fn select_event(anchor: usize, head: usize) -> TransactionSpec {
    TransactionSpec {
        user_event: Some("select".into()),
        ..select(anchor, head)
    }
}

#[test]
fn undo_selection_allows_to_undo_a_change() {
    let mut state = mk();
    state = type_(&state, "newtext");
    state = command(&state, undo_selection);
    assert_eq!(doc(&state), "");
}

#[test]
fn undo_selection_allows_to_undo_selection_only_transactions() {
    let mut state = mk_state(HistoryConfig::default(), "abc");
    assert_eq!(head(&state), 0);
    state = update(&state, select(2, 2));
    state = command(&state, undo_selection);
    assert_eq!(head(&state), 0);
}

#[test]
fn undo_selection_merges_selection_only_transactions_from_keyboard() {
    let mut state = mk_state(HistoryConfig::default(), "abc");
    assert_eq!(head(&state), 0);
    state = update(&state, select_event(2, 2));
    state = update(&state, select_event(3, 3));
    state = update(&state, select_event(1, 1));
    state = command(&state, undo_selection);
    assert_eq!(head(&state), 0);
}

#[test]
fn undo_selection_doesnt_merge_selection_only_transactions_from_other_sources() {
    let mut state = mk_state(HistoryConfig::default(), "abc");
    assert_eq!(head(&state), 0);
    state = update(&state, select(2, 2));
    state = update(&state, select(3, 3));
    state = update(&state, select(1, 1));
    state = command(&state, undo_selection);
    assert_eq!(head(&state), 3);
    state = command(&state, undo_selection);
    assert_eq!(head(&state), 2);
    state = command(&state, undo_selection);
    assert_eq!(head(&state), 0);
}

#[test]
fn undo_selection_doesnt_merge_if_they_change_the_number_of_selections() {
    let mut state = mk_state(HistoryConfig::default(), "abc");
    assert_eq!(head(&state), 0);
    state = update(&state, select_event(2, 2));
    let two = EditorSelection::create(
        vec![EditorSelection::cursor(1), EditorSelection::cursor(3)],
        0,
    )
    .unwrap();
    state = update(
        &state,
        TransactionSpec {
            selection: Some(two),
            user_event: Some("select".into()),
            ..Default::default()
        },
    );
    state = update(&state, select_event(1, 1));
    state = command(&state, undo_selection);
    assert_eq!(state.selection().ranges().len(), 2);
    state = command(&state, undo_selection);
    assert_eq!(head(&state), 0);
}

#[test]
fn undo_selection_doesnt_merge_if_a_selection_changes_empty_state() {
    let mut state = mk_state(HistoryConfig::default(), "abc");
    assert_eq!(head(&state), 0);
    state = update(&state, select_event(2, 2));
    state = update(&state, select_event(2, 3));
    state = update(&state, select_event(1, 1));
    state = command(&state, undo_selection);
    assert_eq!(state.selection().main().anchor(), 2);
    assert_eq!(head(&state), 3);
    state = command(&state, undo_selection);
    assert_eq!(head(&state), 0);
}

#[test]
fn undo_selection_allows_to_redo_a_change() {
    let mut state = mk();
    state = type_(&state, "newtext");
    state = command(&state, undo_selection);
    state = command(&state, redo_selection);
    assert_eq!(doc(&state), "newtext");
}

#[test]
fn undo_selection_allows_to_redo_selection_only_transactions() {
    let mut state = mk_state(HistoryConfig::default(), "abc");
    assert_eq!(head(&state), 0);
    state = update(&state, select(2, 2));
    state = command(&state, undo_selection);
    state = command(&state, redo_selection);
    assert_eq!(head(&state), 2);
}

#[test]
fn undo_selection_only_changes_selection() {
    let mut state = mk();
    state = type_(&state, "hi");
    state = update(&state, isolate(Isolate::Before));
    let selection = state.selection().clone();
    state = update(&state, select(0, 2));
    let selection2 = state.selection().clone();
    state = command(&state, undo_selection);
    assert!(state.selection().eq_with(&selection, false));
    assert_eq!(doc(&state), "hi");
    state = command(&state, redo_selection);
    assert!(state.selection().eq_with(&selection2, false));
    state = update(&state, state.replace_selection("hello"));
    state = command(&state, undo_selection);
    assert!(state.selection().eq_with(&selection2, false));
    state = command(&state, redo);
    assert_eq!(head(&state), 5);
}

#[test]
fn undo_selection_can_undo_a_selection_through_remote_changes() {
    let mut state = mk();
    state = type_(&state, "hello");
    let selection = state.selection().clone();
    state = update(&state, select(0, 2));
    state = receive(&state, "oops", 0, 0);
    state = receive(&state, "!", 9, 9);
    assert!(state
        .selection()
        .eq_with(&EditorSelection::single(4, 6), false));
    state = command(&state, undo_selection);
    assert_eq!(doc(&state), "oopshello!");
    assert!(state.selection().eq_with(&selection, false));
}

#[test]
fn undo_selection_preserves_text_inserted_inside_a_change() {
    let mut state = mk();
    state = type_(&state, "1234");
    state = update(
        &state,
        TransactionSpec {
            annotations: vec![Transaction::ADD_TO_HISTORY.of(false)],
            ..changes(ChangeSpec::insert(2, "x"))
        },
    );
    state = command(&state, undo);
    assert_eq!(doc(&state), "x");
}

// effects

fn effects_update(effects: Vec<StateEffect>, iso: Isolate) -> TransactionSpec {
    TransactionSpec {
        effects,
        annotations: vec![ISOLATE_HISTORY.of(iso)],
        ..Default::default()
    }
}

#[test]
fn includes_inverted_effects_in_the_history() {
    let set = StateEffectType::<i32>::define();
    let field = {
        let set = set.clone();
        StateField::define(StateFieldSpec::new(
            |_| 0,
            move |val: &i32, tr: &Transaction| {
                let mut val = *val;
                for effect in tr.effects() {
                    if let Some(v) = effect.value_of(&set) {
                        val = *v;
                    }
                }
                val
            },
        ))
    };
    let invert = {
        let (set, field) = (set.clone(), field.clone());
        INVERTED_EFFECTS.of(inverted_effects_fn(move |tr| {
            for e in tr.effects() {
                if e.is(&set) {
                    return vec![set.of(*tr.start_state().field(&field))];
                }
            }
            Vec::new()
        }))
    };
    let mut state = EditorState::create(EditorStateConfig {
        extensions: Extension::from(vec![
            history(HistoryConfig::default()),
            Extension::from(&field),
            invert,
        ]),
        ..Default::default()
    })
    .unwrap();
    state = update(&state, effects_update(vec![set.of(10)], Isolate::Before));
    state = update(&state, effects_update(vec![set.of(20)], Isolate::Before));
    assert_eq!(*state.field(&field), 20);
    for (cmd, expected) in [
        (undo as StateCommand, 10),
        (undo, 0),
        (redo, 10),
        (redo, 20),
        (undo, 10),
        (redo, 20),
    ] {
        state = command(&state, cmd);
        assert_eq!(*state.field(&field), expected);
    }
}

#[derive(Clone, PartialEq, Debug)]
struct Comment {
    from: usize,
    to: usize,
    text: String,
}

fn map_comment(comment: &Comment, mapping: &ChangeDesc) -> Option<Comment> {
    let from = mapping.map_pos(comment.from, 1);
    let to = mapping.map_pos(comment.to, -1);
    (from < to).then(|| Comment {
        from,
        to,
        text: comment.text.clone(),
    })
}

struct Comments {
    add: StateEffectType<Comment>,
    field: StateField<Vec<Comment>>,
    invert: Extension,
}

fn comments() -> Comments {
    let add = StateEffectType::define_with_map(map_comment);
    let rm = StateEffectType::define_with_map(map_comment);
    let field = {
        let (add, rm) = (add.clone(), rm.clone());
        StateField::define(StateFieldSpec::new(
            |_| Vec::new(),
            move |value: &Vec<Comment>, tr: &Transaction| {
                let mut value: Vec<Comment> = value
                    .iter()
                    .filter_map(|c| map_comment(c, tr.changes().desc()))
                    .collect();
                for effect in tr.effects() {
                    if let Some(c) = effect.value_of(&add) {
                        value.push(c.clone());
                    } else if let Some(c) = effect.value_of(&rm) {
                        value.retain(|x| x != c);
                    }
                }
                value.sort_by_key(|c| c.from);
                value
            },
        ))
    };
    let invert = {
        let (add, rm, field) = (add.clone(), rm.clone(), field.clone());
        INVERTED_EFFECTS.of(inverted_effects_fn(move |tr| {
            let mut effects = Vec::new();
            for effect in tr.effects() {
                let (value, inverse) = if let Some(c) = effect.value_of(&add) {
                    (c, &rm)
                } else if let Some(c) = effect.value_of(&rm) {
                    (c, &add)
                } else {
                    continue;
                };
                if let Some(src) = map_comment(value, &tr.changes().desc().inverted_desc()) {
                    effects.push(inverse.of(src));
                }
            }
            for comment in tr.start_state().field(&field) {
                if map_comment(comment, tr.changes().desc()).is_none() {
                    effects.push(add.of(comment.clone()));
                }
            }
            effects
        }))
    };
    Comments { add, field, invert }
}

fn comment_str(state: &EditorState, field: &StateField<Vec<Comment>>) -> String {
    state
        .field(field)
        .iter()
        .map(|c| format!("{}@{}", c.text, c.from))
        .collect::<Vec<_>>()
        .join(",")
}

fn comment_state(c: &Comments, doc: &str) -> EditorState {
    EditorState::create(EditorStateConfig {
        extensions: Extension::from(vec![
            history(HistoryConfig::default()),
            Extension::from(&c.field),
            c.invert.clone(),
        ]),
        doc: Some(doc.into()),
        ..Default::default()
    })
    .unwrap()
}

fn comment(from: usize, to: usize, text: &str) -> Comment {
    Comment {
        from,
        to,
        text: text.into(),
    }
}

#[test]
fn can_map_effects() {
    let c = comments();
    let s = |state: &EditorState| comment_str(state, &c.field);
    let mut state = comment_state(&c, "one two foo");
    state = update(
        &state,
        effects_update(vec![c.add.of(comment(0, 3, "c1"))], Isolate::Full),
    );
    assert_eq!(s(&state), "c1@0");
    state = update(
        &state,
        TransactionSpec {
            changes: Some(ChangeSpec::replace(3, 4, "---")),
            ..effects_update(vec![c.add.of(comment(6, 9, "c2"))], Isolate::Full)
        },
    );
    assert_eq!(s(&state), "c1@0,c2@6");
    state = update(
        &state,
        TransactionSpec {
            annotations: vec![Transaction::ADD_TO_HISTORY.of(false)],
            ..changes(ChangeSpec::insert(0, "---"))
        },
    );
    assert_eq!(s(&state), "c1@3,c2@9");
    state = command(&state, undo);
    assert_eq!(doc(&state), "---one two foo");
    assert_eq!(s(&state), "c1@3");
    state = command(&state, undo);
    assert_eq!(s(&state), "");
    state = command(&state, redo);
    assert_eq!(s(&state), "c1@3");
    state = command(&state, redo);
    assert_eq!(s(&state), "c1@3,c2@9");
    assert_eq!(doc(&state), "---one---two foo");
    state = command(&state, undo);
    state = update(
        &state,
        TransactionSpec {
            annotations: vec![Transaction::ADD_TO_HISTORY.of(false)],
            ..changes(ChangeSpec::replace(10, 11, "---"))
        },
    );
    state = update(
        &state,
        effects_update(vec![c.add.of(comment(13, 16, "c3"))], Isolate::Full),
    );
    assert_eq!(s(&state), "c1@3,c3@13");
    state = command(&state, undo);
    assert_eq!(doc(&state), "---one two---foo");
    assert_eq!(s(&state), "c1@3");
    state = command(&state, redo);
    assert_eq!(s(&state), "c1@3,c3@13");
}

#[test]
fn can_restore_comments_lost_through_deletion() {
    let c = comments();
    let mut state = comment_state(&c, "123456");
    state = update(
        &state,
        effects_update(vec![c.add.of(comment(3, 5, "c1"))], Isolate::Full),
    );
    state = update(&state, changes(ChangeSpec::delete(2, 6)));
    assert_eq!(comment_str(&state, &c.field), "");
    state = command(&state, undo);
    assert_eq!(comment_str(&state, &c.field), "c1@3");
}

// Upstream's JSON test ("survives serialization") needs
// `EditorState.toJSON`/`fromJSON`, which are not ported yet.
