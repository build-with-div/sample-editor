//! Port of `@codemirror/state` 6.7.6 `test/test-facet.ts`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use editor_state::{
    ChangeSpec, Dep, EditorSelection, EditorState, EditorStateConfig, Extension, Facet,
    FacetConfig, Prec, StateEffect, StateEffectType, StateField, StateFieldSpec, TransactionSpec,
};

fn mk(extensions: Vec<Extension>) -> EditorState {
    EditorState::create(EditorStateConfig {
        extensions: extensions.into(),
        ..Default::default()
    })
    .unwrap()
}

fn update(state: &EditorState, spec: TransactionSpec) -> EditorState {
    state.update(spec).unwrap().state().clone()
}

fn insert_hello() -> TransactionSpec {
    TransactionSpec {
        changes: Some(ChangeSpec::insert(0, "hello")),
        ..Default::default()
    }
}

fn effects(effects: Vec<StateEffect>) -> TransactionSpec {
    TransactionSpec {
        effects,
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

static NUM: LazyLock<Facet<i32>> = LazyLock::new(Facet::define);
static STR: LazyLock<Facet<String>> = LazyLock::new(Facet::define);
static BOOL: LazyLock<Facet<bool>> = LazyLock::new(Facet::define);

fn s(value: &str) -> String {
    value.to_string()
}

#[test]
fn allows_querying_of_facets() {
    let st = mk(vec![NUM.of(10), NUM.of(20), STR.of(s("x")), STR.of(s("y"))]);
    assert_eq!(join(st.facet(&NUM)), "10,20");
    assert_eq!(join(st.facet(&STR)), "x,y");
}

#[test]
fn includes_sub_extenders() {
    let e = |s: &str| Extension::from(vec![NUM.of(s.len() as i32), NUM.of(s.parse().unwrap())]);
    let st = mk(vec![NUM.of(5), e("20"), NUM.of(40), e("100")]);
    assert_eq!(join(st.facet(&NUM)), "5,2,20,40,3,100");
}

#[test]
fn only_includes_duplicated_extensions_once() {
    let e = NUM.of(50);
    let st = mk(vec![NUM.of(1), e.clone(), NUM.of(4), e]);
    assert_eq!(join(st.facet(&NUM)), "1,50,4");
}

#[test]
fn returns_an_empty_array_for_absent_facet() {
    let st = mk(vec![]);
    assert!(st.facet(&NUM).is_empty());
}

#[test]
fn sorts_extensions_by_priority() {
    let st = mk(vec![
        STR.of(s("a")),
        STR.of(s("b")),
        Prec::high(STR.of(s("c"))),
        Prec::highest(STR.of(s("d"))),
        Prec::low(STR.of(s("e"))),
        Prec::high(STR.of(s("f"))),
        STR.of(s("g")),
    ]);
    assert_eq!(join(st.facet(&STR)), "d,c,f,a,b,g,e");
}

#[test]
fn lets_sub_extensions_inherit_their_parents_priority() {
    let e = |n: i32| NUM.of(n);
    let st = mk(vec![NUM.of(1), Prec::highest(e(2)), e(4)]);
    assert_eq!(join(st.facet(&NUM)), "2,1,4");
}

#[test]
fn supports_dynamic_facet() {
    let st = mk(vec![NUM.of(1), NUM.compute([], |_| 88)]);
    assert_eq!(join(st.facet(&NUM)), "1,88");
}

#[test]
fn only_recomputes_a_facet_value_when_necessary() {
    let st = mk(vec![
        NUM.of(1),
        NUM.compute([Dep::from(&*STR)], |s| join(s.facet(&STR)).len() as i32),
        STR.of(s("hello")),
    ]);
    let array = st.facet(&NUM);
    assert_eq!(join(array), "1,5");
    let next = update(&st, TransactionSpec::default());
    assert!(std::ptr::eq(next.facet(&NUM), array));
}

#[test]
fn can_handle_dependencies_on_facets_that_arent_present_in_the_state() {
    let st = mk(vec![
        NUM.compute([Dep::from(&*STR)], |s| join(s.facet(&STR)).len() as i32),
        STR.compute([Dep::from(&*BOOL)], |s| join(s.facet(&BOOL))),
    ]);
    assert_eq!(
        join(update(&st, TransactionSpec::default()).facet(&NUM)),
        "0"
    );
}

#[test]
fn can_specify_a_dependency_on_the_document() {
    let count = Arc::new(AtomicI32::new(0));
    let c = count.clone();
    let mut st =
        mk(vec![NUM.compute([Dep::DOC], move |_| {
            c.fetch_add(1, Ordering::SeqCst)
        })]);
    assert_eq!(join(st.facet(&NUM)), "0");
    st = update(&st, insert_hello());
    assert_eq!(join(st.facet(&NUM)), "1");
    st = update(&st, TransactionSpec::default());
    assert_eq!(join(st.facet(&NUM)), "1");
}

#[test]
fn can_specify_a_dependency_on_the_selection() {
    let count = Arc::new(AtomicI32::new(0));
    let c = count.clone();
    let mut st =
        mk(vec![NUM.compute([Dep::SELECTION], move |_| {
            c.fetch_add(1, Ordering::SeqCst)
        })]);
    assert_eq!(join(st.facet(&NUM)), "0");
    st = update(&st, insert_hello());
    assert_eq!(join(st.facet(&NUM)), "1");
    st = update(
        &st,
        TransactionSpec {
            selection: Some(EditorSelection::single(2, 2)),
            ..Default::default()
        },
    );
    assert_eq!(join(st.facet(&NUM)), "2");
    st = update(&st, TransactionSpec::default());
    assert_eq!(join(st.facet(&NUM)), "2");
}

#[test]
fn can_provide_multiple_values_at_once() {
    let mut st = mk(vec![
        NUM.compute_n([Dep::DOC], |s| {
            if s.doc().len() % 2 == 1 {
                vec![100, 10]
            } else {
                vec![]
            }
        }),
        NUM.of(1),
    ]);
    assert_eq!(join(st.facet(&NUM)), "1");
    st = update(&st, insert_hello());
    assert_eq!(join(st.facet(&NUM)), "100,10,1");
}

fn sum_facet() -> Facet<i32, i32> {
    Facet::define_with(FacetConfig::new(|ns: &[i32]| ns.iter().sum()))
}

#[test]
fn works_with_a_static_combined_facet() {
    let f = sum_facet();
    let st = mk(vec![f.of(1), f.of(2), f.of(3)]);
    assert_eq!(*st.facet(&f), 6);
}

#[test]
fn works_with_a_dynamic_combined_facet() {
    let f = sum_facet();
    let mut st = mk(vec![
        f.of(1),
        f.compute([Dep::DOC], |s| s.doc().len() as i32),
        f.of(3),
    ]);
    assert_eq!(*st.facet(&f), 4);
    st = update(&st, insert_hello());
    assert_eq!(*st.facet(&f), 9);
}

#[test]
fn survives_reconfiguration() {
    let st = mk(vec![
        NUM.compute([Dep::DOC], |s| s.doc().len() as i32),
        NUM.of(2),
        STR.of(s("3")),
    ]);
    let st2 = update(
        &st,
        effects(vec![StateEffect::reconfigure(vec![
            NUM.compute([Dep::DOC], |s| s.doc().len() as i32),
            NUM.of(2),
        ])]),
    );
    assert!(std::ptr::eq(st.facet(&NUM), st2.facet(&NUM)));
    assert_eq!(st2.facet(&STR).len(), 0);
}

#[test]
fn survives_unrelated_reconfiguration_even_without_deep_compare() {
    struct Count {
        #[allow(dead_code)]
        count: usize,
    }
    // Compared by identity, like upstream's default `===` on objects.
    let f: Facet<i32, Count> = Facet::define_with(FacetConfig::with_compare(
        |v: &[i32]| Count { count: v.len() },
        |a, b| std::ptr::eq(a, b),
    ));
    let st = mk(vec![
        f.compute([Dep::DOC], |s| s.doc().len() as i32),
        f.of(2),
    ]);
    let st2 = update(
        &st,
        effects(vec![StateEffect::append_config(STR.of(s("hi")))]),
    );
    assert!(std::ptr::eq(st.facet(&f), st2.facet(&f)));
}

#[test]
fn preserves_static_facets_across_reconfiguration() {
    let st = mk(vec![NUM.of(1), NUM.of(2), STR.of(s("3"))]);
    let st2 = update(
        &st,
        effects(vec![StateEffect::reconfigure(vec![NUM.of(1), NUM.of(2)])]),
    );
    assert!(std::ptr::eq(st.facet(&NUM), st2.facet(&NUM)));
}

#[test]
fn creates_newly_added_fields_when_reconfiguring() {
    let mut st = mk(vec![NUM.of(2)]);
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let (e1, e2) = (events.clone(), events.clone());
    let field = StateField::define(StateFieldSpec::new(
        move |_| {
            e1.lock().unwrap().push("create".into());
            0
        },
        move |val: &i32, _| {
            e2.lock().unwrap().push(format!("update {val}"));
            val + 1
        },
    ));
    st = update(&st, effects(vec![StateEffect::append_config(&field)]));
    assert_eq!(events.lock().unwrap().join(", "), "create, update 0");
    assert_eq!(*st.field(&field), 1);
}

#[test]
fn applies_effects_from_reconfiguring_transaction_to_new_fields() {
    let st = mk(vec![]);
    let effect = StateEffectType::<i32>::define();
    let e = effect.clone();
    let field = StateField::define(StateFieldSpec::new(
        |state| state.facet(&NUM).first().copied().unwrap_or(0),
        move |val: &i32, tr| {
            tr.effects().iter().fold(*val, |val, eff| {
                val + eff.value_of(&e).copied().unwrap_or(0)
            })
        },
    ));
    let st = update(
        &st,
        effects(vec![
            StateEffect::append_config(vec![(&field).into(), NUM.of(10)]),
            effect.of(5),
        ]),
    );
    assert_eq!(*st.field(&field), 15);
}

#[test]
#[should_panic(expected = "Cyclic dependency")]
fn errors_on_cyclic_dependencies() {
    mk(vec![
        NUM.compute([Dep::from(&*STR)], |s| s.facet(&STR).len() as i32),
        STR.compute([Dep::from(&*NUM)], |s| join(s.facet(&NUM))),
    ]);
}

#[test]
fn updates_facets_computed_from_static_values_on_reconfigure() {
    let mut st = mk(vec![
        NUM.compute([Dep::from(&*STR)], |state| state.facet(&STR).len() as i32),
        STR.of(s("A")),
    ]);
    st = update(
        &st,
        effects(vec![StateEffect::append_config(STR.of(s("B")))]),
    );
    assert_eq!(join(st.facet(&NUM)), "2");
    let next = update(
        &st,
        effects(vec![StateEffect::append_config(BOOL.of(false))]),
    );
    assert!(std::ptr::eq(st.facet(&NUM), next.facet(&NUM)));
}

#[test]
fn preserves_dynamic_facet_values_when_dependencies_stay_the_same() {
    #[derive(Clone, PartialEq)]
    struct A {
        a: i32,
    }
    let f: Facet<A> = Facet::define();
    let st1 = mk(vec![f.compute([], |_| A { a: 1 }), STR.of(s("A"))]);
    let st2 = update(
        &st1,
        effects(vec![StateEffect::append_config(BOOL.of(true))]),
    );
    assert!(std::ptr::eq(st1.facet(&f), st2.facet(&f)));
    assert_eq!(st2.facet(&f)[0].a, 1);
}

// Not upstream tests: API behaviour upstream documents.

#[test]
#[should_panic(expected = "Can't compute a static facet")]
fn rejects_computed_inputs_for_static_facets() {
    let f: Facet<i32> = Facet::define_with(FacetConfig::array().static_facet());
    f.compute([], |_| 1);
}

#[test]
fn adds_enabled_extensions_where_the_facet_is_provided() {
    let f: Facet<i32> = Facet::define_with(FacetConfig::array().enables(STR.of(s("on"))));
    assert_eq!(join(mk(vec![]).facet(&STR)), "");
    assert_eq!(join(mk(vec![f.of(1)]).facet(&STR)), "on");
}

#[test]
fn reads_facets_through_readers_and_defaults() {
    let f = sum_facet();
    let reader = f.reader();
    assert_eq!(*reader.default_value(), 0);
    assert_eq!(*mk(vec![f.of(4)]).facet(&reader), 4);
}

#[test]
fn takes_facet_inputs_from_fields() {
    let field = StateField::define(
        StateFieldSpec::new(
            |_| 7,
            |val: &i32, tr| if tr.doc_changed() { val + 1 } else { *val },
        )
        .provide(|field| NUM.from(field)),
    );
    let st = mk(vec![(&field).into()]);
    assert_eq!(join(st.facet(&NUM)), "7");
    assert_eq!(join(update(&st, insert_hello()).facet(&NUM)), "8");
}
