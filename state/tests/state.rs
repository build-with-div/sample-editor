//! Port of `@codemirror/state` 6.7.6 `test/test-state.ts` and
//! `test/test-charcategory.ts`.

use std::collections::HashMap;
use std::sync::Arc;

use editor_state::{
    facets, Annotation, AnnotationType, ChangeFilter, ChangeFilterResult, ChangeSpec, CharCategory,
    Compartment, Dep, EditorSelection, EditorState, EditorStateConfig, Extension, Facet,
    LanguageData, LanguageDataProvider, RangeChange, SpecItem, StateEffect, StateEffectType,
    StateField, StateFieldSpec, Transaction, TransactionExtender, TransactionExtension,
    TransactionFilter, TransactionSpec,
};

fn create(doc: &str, extensions: Vec<Extension>) -> EditorState {
    EditorState::create(EditorStateConfig {
        doc: Some(doc.into()),
        extensions: extensions.into(),
        ..Default::default()
    })
    .unwrap()
}

fn update(state: &EditorState, spec: TransactionSpec) -> EditorState {
    state.update(spec).unwrap().state().clone()
}

fn changes(spec: ChangeSpec) -> TransactionSpec {
    TransactionSpec {
        changes: Some(spec),
        ..Default::default()
    }
}

fn effect(effect: StateEffect) -> TransactionSpec {
    TransactionSpec {
        effects: vec![effect],
        ..Default::default()
    }
}

// Takes `&Vec` so that the facet's output type can be inferred.
#[allow(clippy::ptr_arg)]
fn join<T: ToString>(values: &Vec<T>) -> String {
    values
        .iter()
        .map(T::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

#[test]
fn holds_doc_and_selection_properties() {
    let state = create("hello", vec![]);
    assert_eq!(state.doc().to_string(), "hello");
    assert_eq!(state.selection().main().from(), 0);
}

#[test]
fn can_apply_changes() {
    let state = create("hello", vec![]);
    let tr = state
        .update(changes(ChangeSpec::List(vec![
            ChangeSpec::replace(2, 4, "w"),
            ChangeSpec::insert(5, "!"),
        ])))
        .unwrap();
    assert_eq!(tr.state().doc().to_string(), "hewo!");
}

#[test]
fn maps_selection_through_changes() {
    let state = EditorState::create(EditorStateConfig {
        doc: Some("abcdefgh".into()),
        extensions: facets::ALLOW_MULTIPLE_SELECTIONS.of(true),
        selection: Some(
            EditorSelection::create([0, 4, 8].map(EditorSelection::cursor).to_vec(), 0).unwrap(),
        ),
    })
    .unwrap();
    let new_state = update(&state, state.replace_selection("Q"));
    assert_eq!(new_state.doc().to_string(), "QabcdQefghQ");
    let froms: Vec<String> = new_state
        .selection()
        .ranges()
        .iter()
        .map(|r| r.from().to_string())
        .collect();
    assert_eq!(froms.join("/"), "1/6/11");
}

#[test]
fn can_store_annotations_on_transactions() {
    let some_annotation = AnnotationType::<i32>::define();
    let tr = create("foo", vec![])
        .update(TransactionSpec {
            annotations: vec![some_annotation.of(55)],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(tr.annotation(some_annotation), Some(&55));
}

#[test]
fn throws_when_a_changes_bounds_are_invalid() {
    // Upstream also passes `from: -1`, which `usize` rules out.
    let state = create("1234", vec![]);
    assert!(state.update(changes(ChangeSpec::delete(2, 1))).is_err());
    assert!(state
        .update(changes(ChangeSpec::replace(2, 10, "x")))
        .is_err());
}

#[test]
fn stores_and_updates_tab_size() {
    let deflt = create("", vec![]);
    let two = create("", vec![facets::TAB_SIZE.of(2)]);
    assert_eq!(deflt.tab_size(), 4);
    assert_eq!(two.tab_size(), 2);
    let updated = update(
        &deflt,
        effect(StateEffect::reconfigure(facets::TAB_SIZE.of(8))),
    );
    assert_eq!(updated.tab_size(), 8);
}

#[test]
fn stores_and_updates_the_line_separator() {
    let deflt = create("", vec![]);
    let crlf = create("", vec![facets::LINE_SEPARATOR.of("\r\n".into())]);
    assert_eq!(*deflt.facet(&facets::LINE_SEPARATOR), None);
    assert_eq!(deflt.to_text("a\nb").lines(), 2);
    assert_eq!(crlf.facet(&facets::LINE_SEPARATOR).as_deref(), Some("\r\n"));
    assert_eq!(crlf.to_text("a\nb").lines(), 1);
    let updated = update(
        &crlf,
        effect(StateEffect::reconfigure(
            facets::LINE_SEPARATOR.of("\n".into()),
        )),
    );
    assert_eq!(
        updated.facet(&facets::LINE_SEPARATOR).as_deref(),
        Some("\n")
    );
}

fn counter() -> StateField<i32> {
    StateField::define(StateFieldSpec::new(|_| 0, |val: &i32, _| val + 1))
}

#[test]
fn stores_and_updates_fields() {
    let field1 = counter();
    let f1 = field1.clone();
    let field2 = StateField::define(StateFieldSpec::new(
        move |state| state.field(&f1) + 10,
        |val: &i32, _| *val,
    ));
    let state = create("", vec![(&field1).into(), (&field2).into()]);
    assert_eq!(*state.field(&field1), 0);
    assert_eq!(*state.field(&field2), 10);
    let new_state = update(&state, TransactionSpec::default());
    assert_eq!(*new_state.field(&field1), 1);
    assert_eq!(*new_state.field(&field2), 10);
}

#[test]
fn allows_fields_to_have_an_initializer() {
    let field = counter();
    let state = create("", vec![field.init(|_| 10)]);
    assert_eq!(*state.field(&field), 10);
    assert_eq!(
        *update(&state, TransactionSpec::default()).field(&field),
        11
    );
}

// Upstream's "can be serialized to JSON" is not ported:
// `EditorState.toJSON` and `fromJSON` are not ported yet.

#[test]
fn can_preserve_fields_across_reconfiguration() {
    let field = counter();
    let start = update(
        &create("", vec![(&field).into()]),
        TransactionSpec::default(),
    );
    assert_eq!(*start.field(&field), 1);
    assert_eq!(
        *update(&start, effect(StateEffect::reconfigure(&field))).field(&field),
        2
    );
    assert_eq!(
        update(&start, effect(StateEffect::reconfigure(Extension::none()))).field_opt(&field),
        None
    );
}

#[test]
fn can_replace_extension_groups() {
    let comp = Compartment::new();
    let f = Facet::<i32>::define();
    let content = f.of(10);
    let state = create("", vec![comp.of(content.clone()), f.of(20)]);
    assert!(comp.get(&state).unwrap().ptr_eq(&content));
    assert_eq!(join(state.facet(&f)), "10,20");
    let content2 = Extension::from(vec![f.of(1), f.of(2)]);
    let state2 = update(&state, effect(comp.reconfigure(content2.clone())));
    assert!(comp.get(&state2).unwrap().ptr_eq(&content2));
    assert_eq!(join(state2.facet(&f)), "1,2,20");
    let state3 = update(&state2, effect(comp.reconfigure(f.of(3))));
    assert_eq!(join(state3.facet(&f)), "3,20");
}

#[test]
fn raises_an_error_on_duplicate_extension_groups() {
    let comp = Compartment::new();
    let f = Facet::<i32>::define();
    let err = EditorState::create(EditorStateConfig {
        extensions: vec![comp.of(f.of(1)), comp.of(f.of(2))].into(),
        ..Default::default()
    })
    .unwrap_err();
    assert!(err
        .0
        .to_lowercase()
        .contains("duplicate use of compartment"));
    let err = EditorState::create(EditorStateConfig {
        extensions: comp.of(comp.of(f.of(1))),
        ..Default::default()
    })
    .unwrap_err();
    assert!(err
        .0
        .to_lowercase()
        .contains("duplicate use of compartment"));
}

#[test]
fn preserves_compartments_on_reconfigure() {
    let comp = Compartment::new();
    let f = Facet::<i32>::define();
    let init = comp.of(f.of(10));
    let mut state = create("", vec![init.clone(), f.of(20)]);
    state = update(&state, effect(comp.reconfigure(f.of(0))));
    assert_eq!(join(state.facet(&f)), "0,20");
    state = update(
        &state,
        effect(StateEffect::reconfigure(vec![init, f.of(2)])),
    );
    assert_eq!(join(state.facet(&f)), "0,2");
}

#[test]
fn forgets_dropped_compartments() {
    let comp = Compartment::new();
    let f = Facet::<i32>::define();
    let init = comp.of(f.of(10));
    let mut state = create("", vec![init.clone(), f.of(20)]);
    state = update(&state, effect(comp.reconfigure(f.of(0))));
    assert_eq!(join(state.facet(&f)), "0,20");
    state = update(&state, effect(StateEffect::reconfigure(f.of(2))));
    assert_eq!(join(state.facet(&f)), "2");
    assert!(comp.get(&state).is_none());
    state = update(
        &state,
        effect(StateEffect::reconfigure(vec![init, f.of(2)])),
    );
    assert_eq!(join(state.facet(&f)), "10,2");
}

#[test]
fn allows_facets_computed_from_fields() {
    let field = StateField::define(StateFieldSpec::new(
        |_| vec![0usize],
        |v: &Vec<usize>, tr| {
            if tr.doc_changed() {
                vec![tr.state().doc().len()]
            } else {
                v.clone()
            }
        },
    ));
    let facet = Facet::<usize>::define();
    let f = field.clone();
    let state = create(
        "",
        vec![
            (&field).into(),
            facet.compute([Dep::from(&field)], move |state| state.field(&f)[0]),
            facet.of(1),
        ],
    );
    assert_eq!(join(state.facet(&facet)), "0,1");
    let state2 = update(&state, TransactionSpec::default());
    assert!(std::ptr::eq(state2.facet(&facet), state.facet(&facet)));
    let state3 = update(&state, changes(ChangeSpec::insert(0, "hi")));
    assert_eq!(join(state3.facet(&facet)), "2,1");
}

#[test]
fn blocks_multiple_selections_when_not_allowed() {
    let cursors = EditorSelection::create(
        vec![EditorSelection::cursor(0), EditorSelection::cursor(1)],
        0,
    )
    .unwrap();
    let state = EditorState::create(EditorStateConfig {
        doc: Some("123".into()),
        selection: Some(cursors.clone()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(state.selection().ranges().len(), 1);
    let next = update(
        &state,
        TransactionSpec {
            selection: Some(cursors),
            ..Default::default()
        },
    );
    assert_eq!(next.selection().ranges().len(), 1);
}

#[test]
fn change_by_range_can_make_simple_changes() {
    let mut state = create("hi", vec![]);
    let spec = state
        .change_by_range(|r| RangeChange {
            changes: Some(ChangeSpec::replace(r.from(), r.from() + 1, "q")),
            range: EditorSelection::cursor(r.from() + 1),
            effects: vec![],
        })
        .unwrap();
    state = update(&state, spec);
    assert_eq!(state.doc().to_string(), "qi");
    assert_eq!(state.selection().main().from(), 1);
}

#[test]
fn change_by_range_does_the_right_thing_when_there_are_multiple_selections() {
    let mut state = EditorState::create(EditorStateConfig {
        doc: Some("1 2 3 4".into()),
        selection: Some(
            EditorSelection::create(
                vec![
                    EditorSelection::range(0, 1),
                    EditorSelection::range(2, 3),
                    EditorSelection::range(4, 5),
                    EditorSelection::range(6, 7),
                ],
                0,
            )
            .unwrap(),
        ),
        extensions: facets::ALLOW_MULTIPLE_SELECTIONS.of(true),
    })
    .unwrap();
    let spec = state
        .change_by_range(|r| RangeChange {
            changes: Some(ChangeSpec::replace(
                r.from(),
                r.to(),
                "-".repeat((r.from() >> 1) + 1),
            )),
            range: EditorSelection::range(r.from(), r.from() + 1 + (r.from() >> 1)),
            effects: vec![],
        })
        .unwrap();
    state = update(&state, spec);
    assert_eq!(state.doc().to_string(), "- -- --- ----");
    let ranges: Vec<String> = state
        .selection()
        .ranges()
        .iter()
        .map(|r| format!("{}-{}", r.from(), r.to()))
        .collect();
    assert_eq!(ranges.join(" "), "0-1 2-4 5-8 9-13");
}

#[test]
fn change_filter_can_cancel_changes() {
    // Cancels all changes that add length.
    let state = create(
        "one two",
        vec![facets::CHANGE_FILTER.of(ChangeFilter::new(|tr| {
            if tr.changes().new_len() <= tr.changes().len() {
                ChangeFilterResult::Allow
            } else {
                ChangeFilterResult::Block
            }
        }))],
    );
    let tr1 = state
        .update(TransactionSpec {
            changes: Some(ChangeSpec::insert(3, " three")),
            selection: Some(EditorSelection::single(13, 13)),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(tr1.state().doc().to_string(), "one two");
    assert_eq!(tr1.state().selection().main().head(), 7);
    let tr2 = state
        .update(changes(ChangeSpec::replace(4, 7, "2")))
        .unwrap();
    assert_eq!(tr2.state().doc().to_string(), "one 2");
}

#[test]
fn change_filter_can_split_changes() {
    // Disallows changes in the middle third of the document.
    let state = create(
        "onetwo",
        vec![facets::CHANGE_FILTER.of(ChangeFilter::new(|tr| {
            let len = tr.start_state().doc().len();
            ChangeFilterResult::Suppress(vec![len / 3, 2 * len / 3])
        }))],
    );
    assert_eq!(
        update(&state, changes(ChangeSpec::delete(0, 6)))
            .doc()
            .to_string(),
        "et"
    );
}

#[test]
fn change_filter_combines_filter_masks() {
    let state = create(
        "onetwo",
        vec![
            facets::CHANGE_FILTER.of(ChangeFilter::new(|_| {
                ChangeFilterResult::Suppress(vec![0, 2])
            })),
            facets::CHANGE_FILTER.of(ChangeFilter::new(|_| {
                ChangeFilterResult::Suppress(vec![4, 6])
            })),
        ],
    );
    assert_eq!(
        update(&state, changes(ChangeSpec::delete(0, 6)))
            .doc()
            .to_string(),
        "onwo"
    );
}

#[test]
fn change_filter_can_be_turned_off() {
    let state = create(
        "",
        vec![facets::CHANGE_FILTER.of(ChangeFilter::new(|_| ChangeFilterResult::Block))],
    );
    assert_eq!(
        update(&state, changes(ChangeSpec::insert(0, "hi")))
            .doc()
            .len(),
        0
    );
    let unfiltered = TransactionSpec {
        changes: Some(ChangeSpec::insert(0, "hi")),
        filter: Some(false),
        ..Default::default()
    };
    assert_eq!(update(&state, unfiltered).doc().len(), 2);
}

#[test]
fn transaction_filter_can_constrain_the_selection() {
    let state = create(
        "one two",
        vec![facets::TRANSACTION_FILTER.of(TransactionFilter::new(|tr| {
            if tr.selection().is_some_and(|s| s.main().to() > 4) {
                vec![
                    tr.clone().into(),
                    TransactionSpec {
                        selection: Some(EditorSelection::single(4, 4)),
                        ..Default::default()
                    }
                    .into(),
                ]
            } else {
                vec![tr.clone().into()]
            }
        }))],
    );
    let select = |at: usize| TransactionSpec {
        selection: Some(EditorSelection::single(at, at)),
        ..Default::default()
    };
    let tr = state.update(select(3)).unwrap();
    assert_eq!(tr.selection().unwrap().main().to(), 3);
    let tr = state.update(select(7)).unwrap();
    assert_eq!(tr.selection().unwrap().main().to(), 4);
}

#[test]
fn transaction_filter_can_append_sequential_changes() {
    let state = create(
        "one two",
        vec![facets::TRANSACTION_FILTER.of(TransactionFilter::new(|tr| {
            vec![
                tr.clone().into(),
                TransactionSpec {
                    changes: Some(ChangeSpec::insert(tr.changes().new_len(), "!")),
                    sequential: true,
                    ..Default::default()
                }
                .into(),
            ]
        }))],
    );
    assert_eq!(
        update(&state, changes(ChangeSpec::insert(3, ",")))
            .doc()
            .to_string(),
        "one, two!"
    );
}

#[test]
fn transaction_extender_can_add_annotations() {
    let ann = AnnotationType::<i32>::define();
    let state = create(
        "",
        vec![
            facets::TRANSACTION_EXTENDER.of(TransactionExtender::new(move |_| {
                Some(TransactionExtension {
                    annotations: vec![ann.of(100)],
                    ..Default::default()
                })
            })),
        ],
    );
    let tr = state.update(changes(ChangeSpec::insert(0, "!"))).unwrap();
    assert_eq!(tr.annotation(ann), Some(&100));
    let tr_no_filter = state
        .update(TransactionSpec {
            changes: Some(ChangeSpec::insert(0, "!")),
            filter: Some(false),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(tr_no_filter.annotation(ann), Some(&100));
}

#[test]
fn transaction_extender_allows_multiple_extenders_to_take_effect() {
    let eff = StateEffectType::<i32>::define();
    let (e1, e2) = (eff.clone(), eff.clone());
    let state = create(
        "",
        vec![
            facets::TRANSACTION_EXTENDER.of(TransactionExtender::new(move |_| {
                Some(TransactionExtension {
                    effects: vec![e1.of(1)],
                    ..Default::default()
                })
            })),
            facets::TRANSACTION_EXTENDER.of(TransactionExtender::new(move |_| {
                Some(TransactionExtension {
                    effects: vec![e2.of(2)],
                    ..Default::default()
                })
            })),
        ],
    );
    let tr = state
        .update(TransactionSpec {
            scroll_into_view: true,
            ..Default::default()
        })
        .unwrap();
    let values: Vec<i32> = tr
        .effects()
        .iter()
        .map(|e| e.value_of(&eff).copied().unwrap_or(0))
        .collect();
    assert_eq!(join(&values), "2,1");
}

// `test-charcategory.ts`

#[test]
fn categorises_into_alphanumeric() {
    let st = create("", vec![]);
    assert_eq!(st.char_categorizer(0)("1"), CharCategory::Word);
    assert_eq!(st.char_categorizer(0)("a"), CharCategory::Word);
}

#[test]
fn categorises_into_whitespace() {
    let st = create("", vec![]);
    assert_eq!(st.char_categorizer(0)(" "), CharCategory::Space);
}

#[test]
fn categorises_into_other() {
    let st = create("", vec![]);
    assert_eq!(st.char_categorizer(0)("/"), CharCategory::Other);
    assert_eq!(st.char_categorizer(0)("<"), CharCategory::Other);
}

// The cases below are not upstream tests; they check the port's API
// against upstream's documented behaviour.

fn word_chars(chars: &str) -> Extension {
    let data: LanguageData = Arc::new(HashMap::from([(
        "wordChars".to_string(),
        Arc::new(chars.to_string()) as editor_state::AnyValue,
    )]));
    facets::LANGUAGE_DATA.of(LanguageDataProvider::new(move |_, _, _| vec![data.clone()]))
}

#[test]
fn counts_word_chars_from_language_data() {
    let st = create("", vec![word_chars("$-")]);
    assert_eq!(
        st.language_data_at::<String>("wordChars", 0, -1),
        vec!["$-".to_string()]
    );
    assert_eq!(st.char_categorizer(0)("$"), CharCategory::Word);
    assert_eq!(st.char_categorizer(0)("<"), CharCategory::Other);
    assert_eq!(
        create("", vec![]).char_categorizer(0)("$"),
        CharCategory::Other
    );
}

#[test]
fn categorises_unicode_and_javascript_whitespace() {
    let st = create("", vec![]);
    let cat = st.char_categorizer(0);
    assert_eq!(cat("é"), CharCategory::Word);
    assert_eq!(cat("中"), CharCategory::Word);
    assert_eq!(cat("٣"), CharCategory::Word);
    assert_eq!(cat("\u{a0}"), CharCategory::Space);
    assert_eq!(cat("\u{feff}"), CharCategory::Space);
    // JavaScript's `\s` does not include U+0085, unlike Unicode
    // White_Space.
    assert_eq!(cat("\u{85}"), CharCategory::Other);
}

#[test]
fn finds_words() {
    let st = create("foo bar_1 éte\n(baz)", vec![word_chars("")]);
    let word = |pos| st.word_at(pos).map(|r| (r.from(), r.to()));
    assert_eq!(word(1), Some((0, 3)));
    assert_eq!(word(3), Some((0, 3)));
    assert_eq!(word(4), Some((4, 9)));
    assert_eq!(word(12), Some((10, 14)));
    assert_eq!(word(15), None);
    assert_eq!(word(16), Some((16, 19)));
}

#[test]
fn looks_up_and_fills_in_phrases() {
    let phrases = Arc::new(HashMap::from([(
        "Find $1".to_string(),
        "Suche $1 ($$)".to_string(),
    )]));
    let st = create("", vec![facets::PHRASES.of(phrases)]);
    assert_eq!(st.phrase("Find $1", &["x"]), "Suche x ($)");
    assert_eq!(st.phrase("Go to $ of $2", &["a", "b"]), "Go to a of b");
    assert_eq!(st.phrase("Keep $3 and $0", &["a"]), "Keep $3 and $0");
    assert_eq!(st.phrase("Untranslated", &[]), "Untranslated");
}

#[test]
fn matches_user_events_by_prefix() {
    let tr = create("", vec![])
        .update(TransactionSpec {
            user_event: Some("select.pointer".into()),
            ..Default::default()
        })
        .unwrap();
    assert!(tr.is_user_event("select"));
    assert!(tr.is_user_event("select.pointer"));
    assert!(!tr.is_user_event("sel"));
    assert!(!tr.is_user_event("input"));
    assert!(tr.annotation(Transaction::TIME).is_some());
}

#[test]
fn reports_reconfiguration_and_reads_options() {
    let st = create("a\nb", vec![facets::READ_ONLY.of(true)]);
    assert!(st.read_only());
    assert_eq!(st.line_break(), "\n");
    assert_eq!(st.slice_doc(0, 3), "a\nb");
    let tr = st.update(TransactionSpec::default()).unwrap();
    assert!(!tr.reconfigured());
    let tr = st
        .update(effect(StateEffect::append_config(facets::TAB_SIZE.of(2))))
        .unwrap();
    assert!(tr.reconfigured());
}

#[test]
fn rejects_selections_outside_the_document() {
    let err = EditorState::create(EditorStateConfig {
        doc: Some("ab".into()),
        selection: Some(EditorSelection::single(3, 3)),
        ..Default::default()
    })
    .unwrap_err();
    assert_eq!(err.0, "Selection points outside of document");
    let st = create("ab", vec![]);
    assert!(st
        .update(TransactionSpec {
            selection: Some(EditorSelection::single(5, 5)),
            ..Default::default()
        })
        .is_err());
}

#[test]
fn maps_effects_through_combined_specs() {
    let pos = StateEffectType::<usize>::define_with_map(|p, mapping| Some(mapping.map_pos(*p, -1)));
    let st = create("abc", vec![]);
    let tr = st
        .update_all([
            SpecItem::from(TransactionSpec {
                changes: Some(ChangeSpec::insert(0, "xx")),
                ..Default::default()
            }),
            SpecItem::from(TransactionSpec {
                changes: Some(ChangeSpec::insert(3, "!")),
                effects: vec![pos.of(4)],
                ..Default::default()
            }),
        ])
        .unwrap();
    assert_eq!(tr.state().doc().to_string(), "xxabc!");
    assert_eq!(tr.effects()[0].value_of(&pos), Some(&6));
}

#[test]
fn annotations_can_be_read_by_type() {
    let a = AnnotationType::<&'static str>::define();
    let ann: Annotation = a.of("x");
    assert!(ann.is(a));
    assert!(!ann.is(Transaction::USER_EVENT));
    assert_eq!(ann.value_of(a), Some(&"x"));
}
