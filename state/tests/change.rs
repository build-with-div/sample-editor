//! Port of `@codemirror/state` 6.7.6 `test/test-change.ts`.

mod common;

use common::{r, r_str};
use editor_state::{ChangeDesc, ChangeSet, ChangeSpec, MapMode, Text};

/// A change description from upstream's notation: `len` for a kept range,
/// `len:ins` for a replaced one.
fn mk(spec: &str) -> ChangeDesc {
    let mut sections = Vec::new();
    for part in spec.split_whitespace() {
        match part.split_once(':') {
            Some((len, ins)) => {
                sections.extend([len.parse::<isize>().unwrap(), ins.parse::<isize>().unwrap()])
            }
            None => sections.extend([part.parse::<isize>().unwrap(), -1]),
        }
    }
    ChangeDesc::from_json(&sections).unwrap()
}

fn r_change(len: usize) -> ChangeSpec {
    if len == 0 || r(3) == 0 {
        // Upstream draws the inserted text before the position.
        let text = r_str(r(5) + 1);
        return ChangeSpec::insert(r(len), text);
    }
    let from = r(len - 1);
    let to = (from + r(5) + 1).min(len);
    if r(2) == 0 {
        ChangeSpec::replace(from, to, r_str(r(2) + 1))
    } else {
        ChangeSpec::delete(from, to)
    }
}

fn r_changes(len: usize, count: usize) -> ChangeSpec {
    ChangeSpec::List((0..count).map(|_| r_change(len)).collect())
}

fn set(spec: impl Into<ChangeSpec>, len: usize) -> ChangeSet {
    ChangeSet::of(spec, len, None).unwrap()
}

mod composition {
    use super::*;

    fn comp(specs: &[&str]) {
        let (result, sets) = specs.split_last().unwrap();
        let composed = sets
            .iter()
            .map(|s| mk(s))
            .reduce(|a, b| a.compose_desc(&b))
            .unwrap();
        assert_eq!(composed.to_string(), *result);
    }

    #[test]
    fn can_compose_unrelated_changes() {
        comp(&["5 0:2", "1 2:0 4", "1 2:0 2 0:2"]);
    }

    #[test]
    fn cancels_insertions_with_deletions() {
        comp(&["2 0:2 2", "2 2:0 2", "4"]);
    }

    #[test]
    fn joins_adjacent_insertions() {
        comp(&["2 0:2 2", "4 0:3 2", "2 0:5 2"]);
    }

    #[test]
    fn joins_adjacent_deletions() {
        comp(&["2 5:0", "1 1:0", "1 6:0"]);
    }

    #[test]
    fn allows_a_delete_to_shadow_multiple_operations() {
        comp(&["2 2:0 0:3", "5:0", "4:0"]);
    }

    #[test]
    fn can_handle_empty_sets() {
        comp(&["", "0:8", "8:0", "", ""]);
    }

    #[test]
    fn can_join_multiple_replaces() {
        comp(&["2 2:2 2:2 2", "1 2:2 2:2 2:2 1", "1 6:6 1"]);
        comp(&["1 2:2 2:2 2:2 1", "2 2:2 2:2 2", "1 6:6 1"]);
        comp(&["1 2:3 3:2 1", "2 3:1 2", "1 5:3 1"]);
    }

    fn throws(a: &str, b: &str) {
        let (a, b) = (mk(a), mk(b));
        assert!(std::panic::catch_unwind(|| a.compose_desc(&b)).is_err());
    }

    #[test]
    fn throws_for_inconsistent_lengths() {
        throws("2 0:2", "1 0:1");
        throws("2 0:2", "30 0:1");
        throws("2 2:0 0:3", "7:0");
    }
}

mod mapping {
    use super::*;

    fn over(a: &str, b: &str, result: &str) {
        assert_eq!(mk(a).map_desc(&mk(b), false).to_string(), result);
    }

    fn under(a: &str, b: &str, result: &str) {
        assert_eq!(mk(a).map_desc(&mk(b), true).to_string(), result);
    }

    #[test]
    fn can_map_over_an_insertion() {
        over("4 0:1", "0:3 4", "7 0:1");
    }

    #[test]
    fn can_map_over_a_deletion() {
        over("4 0:1", "2:0 2", "2 0:1");
    }

    #[test]
    fn orders_insertions() {
        over("2 0:1 2", "2 0:1 2", "3 0:1 2");
        under("2 0:1 2", "2 0:1 2", "2 0:1 3");
    }

    #[test]
    fn can_map_a_deletion_over_an_overlapping_replace() {
        over("2 2:0", "2 1:2 1", "4 1:0");
        under("2 2:0", "2 1:2 1", "4 1:0");
    }

    #[test]
    fn can_handle_changes_after() {
        over("0:1 2:0 8", "6 1:0 0:5 3", "0:1 2:0 12");
    }

    #[test]
    fn joins_deletions() {
        over("5:0 2 3:0 2", "4 4:0 4", "6:0 2");
    }

    #[test]
    fn keeps_insertions_in_deletions() {
        under("2 0:1 2", "4:0", "0:1");
        over("4 0:1 4", "2 4:0 2", "2 0:1 2");
    }

    #[test]
    fn keeps_replacements() {
        over("2 2:2 2", "0:2 6", "4 2:2 2");
        over("2 2:2 2", "3:0 3", "1:2 2");
        over("1 4:4 1", "3 0:2 3", "1 2:4 2 2:0 1");
        over("1 4:4 1", "2 2:0 2", "1 2:4 1");
        over("2 2:2 2", "3 2:0 1", "2 1:2 1");
    }

    #[test]
    fn doesnt_join_replacements() {
        over("2:2 2 2:2", "2 2:0 2", "2:2 2:2");
    }

    #[test]
    fn drops_duplicate_deletion() {
        under("2 2:0 2", "2 2:0 2", "4");
        over("2 2:0 2", "2 2:0 2", "4");
    }

    #[test]
    fn handles_overlapping_replaces() {
        over("1 1:2 1", "1 1:1 1", "2 0:2 1");
        under("1 1:2 1", "1 1:1 1", "1 0:2 2");
        over("1 1:2 2", "1 2:1 1", "1 0:2 2");
        over("2 1:2 1", "1 2:1 1", "2 0:2 1");
        over("2:1 1", "1 2:2", "1:1 2");
        over("1 2:1", "2:2 1", "2 1:1");
    }
}

mod map_pos {
    use super::*;

    /// `(from, to, opt)`: `opt` is an assoc number, or `D`, `A` or `B` for
    /// a tracking mode with assoc -1.
    enum Opt {
        Assoc(i32),
        Mode(MapMode),
    }
    use Opt::*;

    const D: Opt = Mode(MapMode::TrackDel);
    const A: Opt = Mode(MapMode::TrackAfter);
    const B: Opt = Mode(MapMode::TrackBefore);
    const N: Opt = Assoc(-1);

    fn map(spec: &str, cases: &[(usize, Option<usize>, Opt)]) {
        let set = mk(spec);
        for (from, to, opt) in cases {
            let got = match opt {
                Assoc(assoc) => Some(set.map_pos(*from, *assoc)),
                Mode(mode) => set.map_pos_mode(*from, -1, *mode),
            };
            assert_eq!(got, *to, "{spec}: {from}");
        }
    }

    #[test]
    fn maps_through_an_insertion() {
        map(
            "4 0:2 4",
            &[
                (0, Some(0), N),
                (4, Some(4), N),
                (4, Some(6), Assoc(1)),
                (5, Some(7), N),
                (8, Some(10), N),
            ],
        );
    }

    #[test]
    fn maps_through_deletion() {
        map(
            "4 4:0 4",
            &[
                (0, Some(0), N),
                (4, Some(4), N),
                (4, Some(4), D),
                (4, Some(4), B),
                (4, None, A),
                (5, Some(4), N),
                (5, None, D),
                (5, None, B),
                (5, None, A),
                (7, Some(4), N),
                (8, Some(4), N),
                (8, Some(4), D),
                (8, None, B),
                (8, Some(4), A),
                (9, Some(5), N),
                (12, Some(8), N),
            ],
        );
    }

    #[test]
    fn maps_through_multiple_insertions() {
        map(
            "0:2 2 0:2 2 0:2",
            &[
                (0, Some(0), N),
                (0, Some(2), Assoc(1)),
                (1, Some(3), N),
                (2, Some(4), N),
                (2, Some(6), Assoc(1)),
                (3, Some(7), N),
                (4, Some(8), N),
                (4, Some(10), Assoc(1)),
            ],
        );
    }

    #[test]
    fn maps_through_multiple_deletions() {
        map(
            "2:0 2 2:0 2 2:0",
            &[
                (0, Some(0), N),
                (1, Some(0), N),
                (2, Some(0), N),
                (3, Some(1), N),
                (4, Some(2), N),
                (5, Some(2), N),
                (6, Some(2), N),
                (7, Some(3), N),
                (8, Some(4), N),
                (9, Some(4), N),
                (10, Some(4), N),
            ],
        );
    }

    #[test]
    fn maps_through_mixed_edits() {
        map(
            "2 0:2 2:0 0:2 2 2:0 0:2",
            &[
                (0, Some(0), N),
                (2, Some(2), N),
                (2, Some(4), Assoc(1)),
                (3, Some(4), N),
                (4, Some(4), N),
                (4, Some(6), Assoc(1)),
                (5, Some(7), N),
                (6, Some(8), N),
                (7, Some(8), N),
                (8, Some(8), N),
                (8, Some(10), Assoc(1)),
            ],
        );
    }

    #[test]
    fn stays_on_its_own_side_of_replacements() {
        map(
            "2 2:2 2",
            &[
                (2, Some(2), Assoc(1)),
                (2, Some(2), Assoc(-1)),
                (2, Some(2), D),
                (2, Some(2), B),
                (2, None, A),
                (3, Some(2), Assoc(-1)),
                (3, Some(4), Assoc(1)),
                (3, None, D),
                (3, None, B),
                (3, None, A),
                (4, Some(4), Assoc(1)),
                (4, Some(4), Assoc(-1)),
                (4, Some(4), D),
                (4, None, B),
                (4, Some(4), A),
            ],
        );
    }

    #[test]
    fn maps_through_insertions_around_replacements() {
        map(
            "0:1 2:2 0:1",
            &[
                (0, Some(0), Assoc(-1)),
                (0, Some(1), Assoc(1)),
                (1, Some(1), Assoc(-1)),
                (1, Some(3), Assoc(1)),
                (2, Some(3), Assoc(-1)),
                (2, Some(4), Assoc(1)),
            ],
        );
    }

    #[test]
    fn stays_in_between_replacements() {
        map(
            "2:2 2:2",
            &[(2, Some(2), Assoc(-1)), (2, Some(2), Assoc(1))],
        );
    }
}

fn doc10() -> Text {
    Text::of(&["0123456789"]).unwrap()
}

#[test]
fn can_create_change_sets() {
    assert_eq!(
        set(vec![ChangeSpec::insert(5, "hi")], 10)
            .desc()
            .to_string(),
        "5 0:2 5"
    );
    assert_eq!(
        set(vec![ChangeSpec::delete(5, 7)], 10).desc().to_string(),
        "5 2:0 3"
    );
    assert_eq!(
        set(
            vec![
                ChangeSpec::insert(5, "hi"),
                ChangeSpec::insert(5, "ok"),
                ChangeSpec::delete(0, 3),
                ChangeSpec::delete(4, 6),
                ChangeSpec::insert(8, "boo"),
            ],
            10
        )
        .desc()
        .to_string(),
        "3:0 1 1:0 0:4 1:0 2 0:3 2"
    );
}

#[test]
fn can_apply_change_sets() {
    let doc10 = doc10();
    assert_eq!(
        set(vec![ChangeSpec::insert(2, "ok")], 10)
            .apply(&doc10)
            .to_string(),
        "01ok23456789"
    );
    assert_eq!(
        set(vec![ChangeSpec::delete(1, 9)], 10)
            .apply(&doc10)
            .to_string(),
        "09"
    );
    assert_eq!(
        set(
            vec![ChangeSpec::delete(2, 8), ChangeSpec::insert(1, "hi")],
            10
        )
        .apply(&doc10)
        .to_string(),
        "0hi189"
    );
}

#[test]
fn can_apply_composed_sets() {
    let doc10 = doc10();
    assert_eq!(
        set(vec![ChangeSpec::insert(8, "ABCD")], 10)
            .compose(&set(vec![ChangeSpec::delete(8, 11)], 14))
            .apply(&doc10)
            .to_string(),
        "01234567D89"
    );
    assert_eq!(
        set(
            vec![ChangeSpec::insert(2, "hi"), ChangeSpec::insert(8, "ok")],
            10
        )
        .compose(&set(
            vec![
                ChangeSpec::insert(4, "!"),
                ChangeSpec::delete(6, 8),
                ChangeSpec::insert(12, "?")
            ],
            14
        ))
        .apply(&doc10)
        .to_string(),
        "01hi!2367ok?89"
    );
}

#[test]
fn can_clip_inserted_strings_on_compose() {
    assert_eq!(
        set(
            vec![ChangeSpec::insert(2, "abc"), ChangeSpec::insert(4, "def")],
            10
        )
        .compose(&set(vec![ChangeSpec::delete(4, 8)], 16))
        .apply(&doc10())
        .to_string(),
        "01abef456789"
    );
}

#[test]
fn can_apply_mapped_sets() {
    let set0 = set(
        vec![ChangeSpec::insert(5, "hi"), ChangeSpec::delete(8, 10)],
        10,
    );
    let set1 = set(
        vec![ChangeSpec::insert(10, "ok"), ChangeSpec::delete(6, 7)],
        10,
    );
    assert_eq!(
        set0.compose(&set1.map(set0.desc(), false))
            .apply(&doc10())
            .to_string(),
        "01234hi57ok"
    );
}

#[test]
fn can_apply_inverted_sets() {
    let doc10 = doc10();
    let set0 = set(
        vec![ChangeSpec::insert(5, "hi"), ChangeSpec::delete(8, 10)],
        10,
    );
    assert_eq!(
        set0.invert(&doc10).apply(&set0.apply(&doc10)).to_string(),
        doc10.to_string()
    );
}

#[test]
fn can_be_iterated() {
    let set = set(
        vec![ChangeSpec::insert(4, "ok"), ChangeSpec::delete(6, 8)],
        10,
    );
    let mut result = Vec::new();
    set.iter_changes(
        |from_a, to_a, from_b, to_b, inserted| {
            result.push((from_a, to_a, from_b, to_b, inserted.to_string()))
        },
        false,
    );
    assert_eq!(
        result,
        [(4, 4, 4, 6, "ok".to_string()), (6, 8, 8, 8, String::new())]
    );
    let mut gaps = Vec::new();
    set.iter_gaps(|from_a, to_a, len| gaps.push((from_a, to_a, len)));
    assert_eq!(gaps, [(0, 0, 4), (4, 6, 2), (8, 8, 2)]);
}

#[test]
fn mapping_before_produces_the_same_result_as_mapping_the_other_after() {
    let total = 100;
    for i in 0..total {
        let size = r(20);
        let count = i / (total / 10) + 1;
        let (a, b) = (r_changes(size, count), r_changes(size, count));
        let set_a = set(a.clone(), size);
        let set_b = set(b.clone(), size);
        let set_a1 = set_a.map(set_b.desc(), true);
        let set_b1 = set_b.map(set_a.desc(), false);
        let doc = Text::of(&[r_str(size)]).unwrap();
        let set_ab = set_a.compose(&set_b1);
        let set_ba = set_b.compose(&set_a1);
        assert_eq!(
            set_ab.apply(&doc).to_string(),
            set_ba.apply(&doc).to_string(),
            "size = {size}\na = {a:?}\nb = {b:?}"
        );
    }
}

#[test]
fn mapping_still_converges_when_mapping_through_multiple_changes() {
    let total = 100;
    for i in 0..total {
        let size = r(20);
        let count = i / (total / 10) + 1;
        let a = set(r_changes(size, count), size);
        let b = set(r_changes(a.new_len(), count), a.new_len());
        let c = set(r_changes(size, count), size);
        let c_a = c.map(a.desc(), false);
        let c_ab = c_a.map(b.desc(), false);
        let a_c = a.map(c.desc(), true);
        let b_ca = b.map(c_a.desc(), true);
        let doc = Text::of(&[r_str(size)]).unwrap();
        assert_eq!(
            a.compose(&b).compose(&c_ab).apply(&doc).to_string(),
            c.compose(&a_c).compose(&b_ca).apply(&doc).to_string()
        );
    }
}

#[test]
fn compose_produces_the_same_result_as_individual_changes() {
    for _ in 0..100 {
        let size = r(20);
        let doc = Text::of(&[r_str(size)]).unwrap();
        let a = set(r_changes(size, r(5) + 1), size);
        let b = set(r_changes(a.new_len(), r(6)), a.new_len());
        assert_eq!(
            b.apply(&a.apply(&doc)).to_string(),
            a.compose(&b).apply(&doc).to_string()
        );
    }
}

#[test]
fn composing_is_associative() {
    for _ in 0..100 {
        let size = r(20);
        let doc = Text::of(&[r_str(size)]).unwrap();
        let a = set(r_changes(size, r(5) + 1), size);
        let b = set(r_changes(a.new_len(), r(6)), a.new_len());
        let c = set(r_changes(b.new_len(), r(5) + 1), b.new_len());
        let left = a.compose(&b).compose(&c);
        let right = a.compose(&b.compose(&c));
        assert_eq!(left.apply(&doc).to_string(), right.apply(&doc).to_string());
    }
}

#[test]
fn survives_random_sequences_of_changes() {
    let doc10 = doc10();
    for _ in 0..50 {
        let mut doc = doc10.clone();
        let mut txt = doc.to_string();
        let mut all: Vec<ChangeSet> = Vec::new();
        let mut inv: Vec<ChangeSet> = Vec::new();
        let mut log = Vec::new();
        for _ in 0..50 {
            let change = r_change(doc.len());
            log.push(format!("{change:?} in {}", doc.len()));
            let ChangeSpec::Change(c) = &change else {
                unreachable!()
            };
            let insert = match &c.insert {
                Some(editor_state::Insertion::Str(s)) => s.as_str(),
                _ => "",
            };
            txt.replace_range(c.from..c.to, insert);
            let set = set(vec![change.clone()], doc.len());
            all.push(set.clone());
            inv.push(set.invert(&doc));
            doc = set.apply(&doc);
            assert_eq!(doc.to_string(), txt, "with changes: {}", log.join(", "));
        }
        let composed = all
            .iter()
            .fold(set(Vec::<ChangeSpec>::new(), doc10.len()), |a, b| {
                a.compose(b)
            });
        assert_eq!(composed.apply(&doc10).to_string(), txt);
        assert_eq!(
            composed.invert(&doc10).apply(&doc).to_string(),
            doc10.to_string()
        );
        for set in inv.iter().rev() {
            doc = set.apply(&doc);
        }
        assert_eq!(doc.to_string(), doc10.to_string());
    }
}

#[test]
fn can_be_serialized_to_json() {
    for _ in 0..100 {
        let size = r(20) + 1;
        let set = set(r_changes(size, r(4)), size);
        assert_eq!(
            ChangeSet::from_json(&set.to_json()).to_string(),
            set.to_string()
        );
    }
}

#[test]
fn rejects_invalid_change_ranges() {
    assert!(ChangeSet::of(ChangeSpec::delete(4, 2), 10, None).is_err());
    assert!(ChangeSet::of(ChangeSpec::delete(4, 11), 10, None).is_err());
    assert!(ChangeSet::of(ChangeSet::empty(5), 10, None).is_err());
}
