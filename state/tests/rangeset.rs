//! Port of `@codemirror/state` 6.7.6 `test/test-rangeset.ts`.

use std::sync::{Arc, OnceLock};

use editor_state::{
    ChangeSet, ChangeSpec, Range, RangeComparator, RangeSet, RangeSetUpdate, RangeValue,
    SpanIterator,
};

struct Value {
    start_side: i32,
    end_side: i32,
    point: bool,
    name: Option<String>,
    pos: Option<usize>,
}

impl RangeValue for Value {
    fn range_eq(&self, other: &Self) -> bool {
        other.name == self.name
    }
    fn start_side(&self) -> i32 {
        self.start_side
    }
    fn end_side(&self) -> i32 {
        self.end_side
    }
    fn point(&self) -> bool {
        self.point
    }
}

fn names(values: &[Arc<Value>]) -> String {
    values
        .iter()
        .filter(|v| v.name.is_some() || v.point)
        .map(|v| v.name.clone().unwrap_or_else(|| "POINT".into()))
        .collect::<Vec<_>>()
        .join("/")
}

/// The spec object upstream's `mk` takes.
#[derive(Default, Clone)]
struct Spec {
    start_side: Option<i32>,
    end_side: Option<i32>,
    point: bool,
    name: Option<String>,
    pos: Option<usize>,
}

fn spec() -> Spec {
    Spec::default()
}

impl Spec {
    fn name(mut self, name: &str) -> Spec {
        self.name = Some(name.into());
        self
    }
    fn start(mut self, side: i32) -> Spec {
        self.start_side = Some(side);
        self
    }
    fn end(mut self, side: i32) -> Spec {
        self.end_side = Some(side);
        self
    }
    fn point(mut self) -> Spec {
        self.point = true;
        self
    }
    fn pos(mut self, pos: usize) -> Spec {
        self.pos = Some(pos);
        self
    }
}

impl From<&str> for Spec {
    fn from(name: &str) -> Spec {
        spec().name(name)
    }
}

fn mk(from: usize, to: usize, spec: impl Into<Spec>) -> Range<Value> {
    let spec = spec.into();
    let empty = from == to;
    let start_side = spec.start_side.unwrap_or(1);
    let value = Arc::new(Value {
        start_side,
        end_side: spec
            .end_side
            .or(spec.start_side)
            .unwrap_or(if empty { 1 } else { -1 }),
        point: empty || spec.point,
        name: spec.name,
        pos: spec.pos,
    });
    value.range(from, to)
}

/// Upstream's `mk(pos, spec)`, an empty range.
fn mk_at(pos: usize, spec: impl Into<Spec>) -> Range<Value> {
    mk(pos, pos, spec)
}

fn mk_set(ranges: Vec<Range<Value>>) -> RangeSet<Value> {
    RangeSet::of(ranges, false)
}

fn sorted(mut ranges: Vec<Range<Value>>) -> Vec<Range<Value>> {
    ranges.sort_by_key(|r| r.from);
    ranges
}

fn change_set(changes: &[(usize, usize, usize)]) -> ChangeSet {
    let mut collect = Vec::new();
    for &(from, to, len) in changes {
        if len > 0 {
            collect.push(ChangeSpec::replace(from, to, "x".repeat(len)));
        }
        if from < to {
            collect.push(ChangeSpec::delete(from, to));
        }
    }
    ChangeSet::of(ChangeSpec::List(collect), 5100, None).unwrap()
}

fn small_ranges() -> Vec<Range<Value>> {
    (0..5000)
        .map(|i| mk(i, i + 1 + (i % 4), spec().pos(i)))
        .collect()
}

fn set0() -> &'static RangeSet<Value> {
    static SET0: OnceLock<RangeSet<Value>> = OnceLock::new();
    SET0.get_or_init(|| mk_set(small_ranges()))
}

fn check_set(set: &RangeSet<Value>) {
    let mut count = 0;
    set.between(0, set.len(), |from, _, value| {
        count += 1;
        if let Some(pos) = value.pos {
            assert_eq!(from, pos);
        }
        true
    });
    assert_eq!(count, set.size());
}

fn filter_update<'a>(
    filter: &'a mut dyn FnMut(usize, usize, &Arc<Value>) -> bool,
) -> RangeSetUpdate<'a, Value> {
    RangeSetUpdate {
        filter: Some(filter),
        ..Default::default()
    }
}

fn add_update(add: Vec<Range<Value>>) -> RangeSetUpdate<'static, Value> {
    RangeSetUpdate {
        add,
        ..Default::default()
    }
}

#[test]
fn divides_a_set_into_chunks_and_layers() {
    let mut ranges = small_ranges();
    ranges.push(mk(1000, 4000, spec().pos(1000)));
    ranges.push(mk(2000, 3000, spec().pos(2000)));
    let set = mk_set(sorted(ranges));
    assert_eq!(set.size(), 5002);
    assert!(set.chunk_count() > 1);
    assert!(set.layer_below().map_or(0, |l| l.size()) > 0);
    check_set(&set);
}

#[test]
#[should_panic(expected = "sorted")]
fn complains_about_misordered_ranges() {
    mk_set(vec![mk(8, 9, spec()), mk(7, 10, spec())]);
}

#[test]
#[should_panic(expected = "sorted")]
fn complains_about_misordered_sides() {
    mk_set(vec![mk(1, 1, spec().start(1)), mk(1, 1, spec().start(-1))]);
}

// update

#[test]
fn update_can_add_ranges() {
    let set = set0().update(add_update(vec![mk_at(4000, spec().pos(4000))]));
    assert_eq!(set.size(), 5001);
    assert!(set.shares_chunk(set0(), 0));
}

#[test]
fn update_can_add_a_large_amount_of_ranges() {
    let ranges = (0..4000).step_by(2).map(|i| mk_at(i, spec())).collect();
    let set = set0().update(add_update(ranges));
    assert_eq!(set.size(), 2000 + set0().size());
    check_set(&set);
}

#[test]
fn update_can_filter_ranges() {
    let set = set0().update(filter_update(&mut |from, _, _| from >= 2500));
    assert_eq!(set.size(), 2500);
    assert!(set.chunk_count() < set0().chunk_count());
    check_set(&set);
}

#[test]
fn update_can_filter_all_over() {
    let set = set0().update(filter_update(&mut |from, _, _| from % 200 >= 100));
    assert_eq!(set.size(), 2500);
    check_set(&set);
}

#[test]
fn update_collapses_the_chunks_when_removing_almost_all_ranges() {
    let set = set0().update(filter_update(&mut |from, _, _| from == 500 || from == 501));
    assert_eq!(set.size(), 2);
    assert_eq!(set.chunk_count(), 1);
}

#[test]
fn update_calls_filter_on_precisely_those_ranges_touching_the_filter_range() {
    let ranges = (0..1000).map(|i| mk(i, i + 1, spec().pos(i))).collect();
    let set = mk_set(ranges);
    let mut called = Vec::new();
    set.update(RangeSetUpdate {
        filter: Some(&mut |from, to, _| {
            called.push((from, to));
            true
        }),
        filter_from: Some(400),
        filter_to: Some(600),
        ..Default::default()
    });
    assert_eq!(called.len(), 202);
    for (j, i) in (399..=600).enumerate() {
        assert_eq!(called[j], (i, i + 1));
    }
}

#[test]
fn update_returns_the_empty_set_when_filter_removes_everything() {
    // Upstream checks identity with `RangeSet.empty`.
    assert!(set0()
        .update(filter_update(&mut |_, _, _| false))
        .is_empty());
}

// map

/// A position, or a `from`-`to` pair, as upstream's expectations.
fn map_test(
    positions: Vec<Range<Value>>,
    changes: &[(usize, usize, usize)],
    expected: &[(usize, usize)],
) {
    let set = mk_set(positions);
    let mapped = set.map(change_set(changes).desc());
    let mut out = Vec::new();
    let mut iter = mapped.iter(0);
    while iter.value().is_some() {
        out.push((iter.from(), iter.to()));
        iter.next();
    }
    assert_eq!(out, expected);
}

#[test]
fn map_can_map_through_changes() {
    map_test(
        vec![mk_at(1, spec()), mk_at(4, spec()), mk_at(10, spec())],
        &[(0, 0, 1), (2, 3, 0), (8, 8, 20)],
        &[(2, 2), (4, 4), (30, 30)],
    );
}

#[test]
fn map_takes_inclusivity_into_account() {
    map_test(
        vec![mk(1, 2, spec().start(-1).end(1))],
        &[(1, 1, 2), (2, 2, 2)],
        &[(1, 6)],
    );
}

#[test]
fn map_uses_side_to_determine_mapping_of_points() {
    map_test(
        vec![
            mk(1, 1, spec().start(-1).end(-1)),
            mk(1, 1, spec().start(1).end(1)),
        ],
        &[(1, 1, 2)],
        &[(1, 1), (3, 3)],
    );
}

#[test]
fn map_defaults_to_exclusive_on_both_sides() {
    map_test(vec![mk(1, 2, spec())], &[(1, 1, 2), (4, 4, 2)], &[(3, 4)]);
}

#[test]
fn map_drops_point_ranges() {
    map_test(vec![mk(1, 2, spec())], &[(1, 2, 0), (1, 1, 1)], &[]);
}

#[test]
fn map_drops_ranges_in_deleted_regions() {
    map_test(vec![mk(1, 2, spec()), mk_at(3, spec())], &[(0, 4, 0)], &[]);
}

#[test]
fn map_shrinks_ranges() {
    map_test(
        vec![mk(2, 4, spec()), mk(2, 8, spec()), mk(6, 8, spec())],
        &[(3, 7, 0)],
        &[(2, 3), (2, 4), (3, 4)],
    );
}

#[test]
fn map_leaves_point_ranges_on_change_boundaries() {
    map_test(
        vec![mk_at(2, spec()), mk_at(4, spec())],
        &[(2, 4, 6)],
        &[(2, 2), (8, 8)],
    );
}

#[test]
fn map_can_collapse_chunks() {
    let smaller = set0().map(change_set(&[(30, 4500, 0)]).desc());
    assert!(smaller.chunk_count() < set0().chunk_count());
    // Upstream checks identity with `RangeSet.empty`.
    let empty = smaller.map(change_set(&[(0, 1000, 0)]).desc());
    assert!(empty.is_empty());
}

#[test]
fn map_rearranges_points_that_change_order() {
    let set = mk_set(vec![
        mk_at(1, spec().name("a").start(1)),
        mk_at(2, spec().name("b").start(-1)),
    ])
    .map(change_set(&[(1, 2, 0)]).desc());
    let mut iter = set.iter(0);
    assert_eq!(iter.from(), 1);
    assert_eq!(iter.value().unwrap().name.as_deref(), Some("b"));
    iter.next();
    assert_eq!(iter.from(), 1);
    assert_eq!(iter.value().unwrap().name.as_deref(), Some("a"));
    iter.next();
    assert!(iter.value().is_none());
}

#[test]
fn map_spills_ranges_that_overlap() {
    let set = mk_set(vec![
        mk(0, 1, spec().name("a").end(1)),
        mk(2, 3, spec().name("b").start(-1)),
    ])
    .map(change_set(&[(1, 2, 0)]).desc())
    .map(change_set(&[(1, 1, 1)]).desc());
    assert!(set.layer_below().is_some());
}

#[test]
fn map_spills_ranges_that_mapping_puts_out_of_order() {
    map_test(
        vec![
            mk(0, 1, spec().start(-1).end(1)),
            mk(2, 3, spec().start(-1).end(1)),
            mk_at(4, spec().start(-2)),
        ],
        &[(0, 4, 0)],
        &[(0, 0), (0, 0), (0, 0)],
    );
}

#[test]
fn map_rearranges_a_point_that_moves_before_several_others() {
    let set = mk_set(vec![
        mk_at(1, spec().name("a").start(0)),
        mk_at(1, spec().name("b").start(1)),
        mk_at(1, spec().name("c").start(1)),
        mk_at(7, spec().name("d").start(0)),
    ])
    .map(change_set(&[(1, 7, 0)]).desc());
    let mut names = Vec::new();
    let mut iter = set.iter(0);
    while let Some(value) = iter.value() {
        assert_eq!(iter.from(), 1);
        assert_eq!(iter.to(), 1);
        names.push(value.name.clone().unwrap());
        iter.next();
    }
    assert_eq!(names.join(","), "a,d,b,c");
}

#[test]
fn map_keeps_chunk_offsets_relative_when_reordering_points() {
    map_test(
        vec![
            mk_at(10, spec().start(0)),
            mk_at(10, spec().start(1)),
            mk_at(10, spec().start(1)),
            mk_at(16, spec().start(0)),
        ],
        &[(10, 16, 0)],
        &[(10, 10), (10, 10), (10, 10), (10, 10)],
    );
}

#[test]
fn map_keeps_a_point_after_a_range_that_starts_before_it() {
    map_test(
        vec![
            mk(0, 10, spec().start(-1).end(1)),
            mk(12, 20, spec().start(-1).end(1)),
            mk_at(25, spec().start(0)),
        ],
        &[(15, 25, 0)],
        &[(0, 10), (12, 15), (15, 15)],
    );
}

// compare

#[derive(Default)]
struct Comparator {
    ranges: Vec<usize>,
}

impl Comparator {
    fn add_range(&mut self, from: usize, to: usize) {
        if self.ranges.last() == Some(&from) {
            *self.ranges.last_mut().unwrap() = to;
        } else {
            self.ranges.extend([from, to]);
        }
    }
}

impl RangeComparator<Value> for Comparator {
    fn compare_range(&mut self, from: usize, to: usize, _: &[Arc<Value>], _: &[Arc<Value>]) {
        self.add_range(from, to);
    }
    fn compare_point(
        &mut self,
        from: usize,
        to: usize,
        _: Option<&Arc<Value>>,
        _: Option<&Arc<Value>>,
    ) {
        self.add_range(from, to);
    }
}

#[derive(Default)]
struct CompareUpdate {
    changes: Option<Vec<(usize, usize, usize)>>,
    add: Vec<Range<Value>>,
    filter: Option<fn(usize) -> bool>,
}

fn compare_test(set: RangeSet<Value>, update: CompareUpdate, expected: &[usize]) {
    let doc_changes = change_set(update.changes.as_deref().unwrap_or(&[]));
    let mut new_set = set.clone();
    if update.changes.is_some() {
        new_set = new_set.map(doc_changes.desc());
    }
    if update.filter.is_some() || !update.add.is_empty() {
        let mut filter = update
            .filter
            .map(|f| move |from: usize, _: usize, _: &Arc<Value>| f(from));
        new_set = new_set.update(RangeSetUpdate {
            add: update.add,
            filter: filter
                .as_mut()
                .map(|f| f as &mut dyn FnMut(usize, usize, &Arc<Value>) -> bool),
            ..Default::default()
        });
    }
    let mut comp = Comparator::default();
    RangeSet::compare(&[set], &[new_set], doc_changes.desc(), &mut comp, -1);
    assert_eq!(comp.ranges, expected);
}

#[test]
fn compare_notices_added_ranges() {
    compare_test(
        mk_set(vec![mk(2, 4, "a"), mk(8, 11, "a")]),
        CompareUpdate {
            add: vec![mk(3, 9, "b"), mk(106, 107, "b")],
            ..Default::default()
        },
        &[3, 9, 106, 107],
    );
}

#[test]
fn compare_notices_deleted_ranges() {
    compare_test(
        mk_set(vec![
            mk(4, 6, "a"),
            mk(5, 7, "b"),
            mk(6, 8, "c"),
            mk(20, 30, "d"),
        ]),
        CompareUpdate {
            filter: Some(|from| from != 5 && from != 20),
            ..Default::default()
        },
        &[5, 7, 20, 30],
    );
}

#[test]
fn compare_recognizes_identical_ranges() {
    compare_test(
        mk_set(vec![mk(0, 50, "a")]),
        CompareUpdate {
            add: vec![mk(10, 40, "a")],
            filter: Some(|_| false),
            ..Default::default()
        },
        &[0, 10, 40, 50],
    );
}

#[test]
fn compare_skips_changes() {
    compare_test(
        mk_set(vec![mk(0, 20, "a")]),
        CompareUpdate {
            changes: Some(vec![(5, 15, 20)]),
            filter: Some(|_| false),
            ..Default::default()
        },
        &[0, 5, 25, 30],
    );
}

// Upstream also makes reading the first chunk's values throw, to prove the
// comparison skips the shared chunk; Rust has no property traps, so this
// port checks the result only.
#[test]
fn compare_ignores_identical_sub_nodes() {
    let ranges = (0..1000).map(|i| mk(i, i + 1, "a")).collect();
    compare_test(
        mk_set(ranges),
        CompareUpdate {
            changes: Some(vec![(900, 1000, 0)]),
            add: vec![mk(850, 860, "b")],
            ..Default::default()
        },
        &[850, 860],
    );
}

#[test]
fn compare_ignores_changes_in_points() {
    let mut ranges = vec![mk(3, 997, spec().point())];
    ranges.extend((0..1000).step_by(2).map(|i| mk(i, i + 1, "a")));
    let set = mk_set(sorted(ranges));
    compare_test(
        set,
        CompareUpdate {
            changes: Some(vec![(300, 500, 100)]),
            ..Default::default()
        },
        &[],
    );
}

#[test]
fn compare_notices_adding_a_point() {
    compare_test(
        mk_set(vec![mk(3, 50, spec().point())]),
        CompareUpdate {
            add: vec![mk(40, 80, spec().point())],
            ..Default::default()
        },
        &[50, 80],
    );
}

#[test]
fn compare_notices_removing_a_point() {
    compare_test(
        mk_set(vec![mk(3, 50, spec().point())]),
        CompareUpdate {
            filter: Some(|_| false),
            ..Default::default()
        },
        &[3, 50],
    );
}

#[test]
fn compare_can_handle_multiple_changes() {
    let ranges = (0..200)
        .step_by(2)
        .map(|i: usize| {
            let end = i + 1 + i.div_ceil(50);
            mk(i, end, format!("{i}-{end}").as_str())
        })
        .collect();
    compare_test(
        mk_set(ranges),
        CompareUpdate {
            changes: Some(vec![(0, 0, 50), (50, 100, 0), (150, 200, 0)]),
            filter: Some(|from| from % 50 > 0),
            ..Default::default()
        },
        &[50, 51, 100, 103, 148, 153],
    );
}

#[test]
fn compare_reports_point_decorations_with_different_cover() {
    compare_test(
        mk_set(vec![
            mk(0, 4, spec().start(1).end(-1)),
            mk(1, 3, spec().point().start(-1).end(1)),
        ]),
        CompareUpdate {
            changes: Some(vec![(2, 4, 0)]),
            ..Default::default()
        },
        &[1, 2],
    );
}

// spans

#[derive(Default)]
struct Builder {
    spans: Vec<String>,
}

impl SpanIterator<Value> for Builder {
    fn span(&mut self, from: usize, to: usize, active: &[Arc<Value>], _: usize) {
        let name = names(active);
        let suffix = if name.is_empty() {
            String::new()
        } else {
            format!("={name}")
        };
        self.spans.push(format!("{}{suffix}", to - from));
    }
    fn point(
        &mut self,
        from: usize,
        to: usize,
        value: &Arc<Value>,
        _: &[Arc<Value>],
        _: usize,
        _: usize,
    ) {
        let size = if to > from {
            format!("{}=", to - from)
        } else {
            String::new()
        };
        let name = value
            .name
            .as_ref()
            .map_or_else(|| "ø".to_string(), |name| format!("[{name}]"));
        self.spans.push(format!("{size}{name}"));
    }
}

fn spans_test(sets: &[RangeSet<Value>], start: usize, end: usize, expected: &str) {
    let mut builder = Builder::default();
    RangeSet::spans(sets, start, end, &mut builder, -1);
    assert_eq!(builder.spans.join(" "), expected);
}

#[test]
fn spans_separates_the_range_in_covering_spans() {
    spans_test(
        &[mk_set(vec![
            mk(3, 8, "one"),
            mk(5, 8, "two"),
            mk(10, 12, "three"),
        ])],
        0,
        15,
        "3 2=one 3=two/one 2 2=three 3",
    );
}

#[test]
fn spans_can_retrieve_a_limited_range() {
    let mut decos = vec![mk(0, 200, "wide")];
    decos.extend((0..100).map(|i| mk(i * 2, i * 2 + 2, format!("span{i}").as_str())));
    let set = mk_set(decos);
    let (start, end) = (20, 26);
    let mut expected = Vec::new();
    let mut pos = start;
    while pos < end {
        let step = if pos % 2 == 1 { 1 } else { 2 };
        expected.push(format!(
            "{}=span{}/wide",
            end.min(pos + step) - pos,
            pos / 2
        ));
        pos += step;
    }
    spans_test(&[set], start, end, &expected.join(" "));
}

#[test]
fn spans_reads_from_multiple_sets_at_once() {
    let one = mk_set(vec![mk(2, 3, "x"), mk(5, 10, "y"), mk(10, 12, "z")]);
    let two = mk_set(vec![mk(0, 6, "a"), mk(10, 12, "b")]);
    spans_test(&[one, two], 0, 12, "2=a 1=x/a 2=a 1=y/a 4=y 2=z/b");
}

#[test]
fn spans_orders_active_ranges_by_origin_set() {
    let one = mk_set(vec![mk(2, 10, "a"), mk(20, 30, "a")]);
    let two = mk_set(vec![mk(3, 4, "b"), mk(8, 12, "b"), mk(18, 22, "b")]);
    let three = mk_set(vec![mk(0, 25, "c")]);
    spans_test(
        &[one, two, three],
        0,
        25,
        "2=c 1=a/c 1=a/b/c 4=a/c 2=a/b/c 2=b/c 6=c 2=b/c 2=a/b/c 3=a/c",
    );
}

#[test]
fn spans_doesnt_get_confused_by_same_place_points() {
    spans_test(
        &[mk_set(vec![mk_at(1, "a"), mk_at(1, "b"), mk_at(1, "c")])],
        0,
        2,
        "1 [a] [b] [c] 1",
    );
}

#[test]
fn spans_properly_resyncs_active_ranges_after_points() {
    spans_test(
        &[mk_set(vec![
            mk(0, 20, "r1"),
            mk(1, 10, "r2"),
            mk(3, 12, spec().name("p").point()),
            mk(4, 8, "r3"),
            mk(5, 20, "r4"),
        ])],
        0,
        20,
        "1=r1 2=r2/r1 9=[p] 8=r4/r1",
    );
}

struct Bounds(Vec<usize>);

impl SpanIterator<Value> for Bounds {
    fn span(&mut self, from: usize, to: usize, _: &[Arc<Value>], _: usize) {
        self.0.extend([from, to]);
    }
    fn point(
        &mut self,
        from: usize,
        to: usize,
        _: &Arc<Value>,
        _: &[Arc<Value>],
        _: usize,
        _: usize,
    ) {
        self.0.extend([from, to]);
    }
}

#[test]
fn spans_doesnt_split_spans_on_ignored_ranges() {
    let mut ranges = Bounds(Vec::new());
    RangeSet::spans(
        &[mk_set(vec![
            mk(0, 10, "a"),
            mk(20, 30, spec().name("b").point()),
        ])],
        0,
        30,
        &mut ranges,
        0,
    );
    assert_eq!(ranges.0, [0, 20, 20, 30]);
}

struct CountPoints(usize);

impl SpanIterator<Value> for CountPoints {
    fn span(&mut self, _: usize, _: usize, _: &[Arc<Value>], _: usize) {}
    fn point(&mut self, _: usize, _: usize, _: &Arc<Value>, _: &[Arc<Value>], _: usize, _: usize) {
        self.0 += 1;
    }
}

#[test]
fn spans_omits_points_that_are_covered_by_the_previous_point() {
    let mut points = CountPoints(0);
    RangeSet::spans(
        &[mk_set(vec![
            mk(0, 4, spec().point()),
            mk(1, 5, "a"),
            mk(2, 4, spec().point()),
        ])],
        0,
        10,
        &mut points,
        -1,
    );
    assert_eq!(points.0, 1);
}

#[test]
fn spans_puts_smaller_spans_inside_bigger_ones_with_the_same_rank() {
    spans_test(
        &[mk_set(vec![
            mk(0, 3, "x"),
            mk(0, 1, "a"),
            mk(1, 2, "b"),
            mk(2, 3, "c"),
        ])],
        0,
        3,
        "1=a/x 1=b/x 1=c/x",
    );
}

// iter

#[test]
fn iter_iterates_over_ranges() {
    let mut ranges = small_ranges();
    ranges.push(mk(1000, 4000, spec().pos(1000)));
    ranges.push(mk(2000, 3000, spec().pos(2000)));
    let set = mk_set(sorted(ranges));
    let mut count = 0;
    let mut iter = set.iter(0);
    while iter.value().is_some() {
        let expected = if count > 2001 {
            count - 2
        } else if count > 1000 {
            count - 1
        } else {
            count
        };
        assert_eq!(iter.from(), expected);
        iter.next();
        count += 1;
    }
    assert_eq!(count, 5002);
}

#[test]
fn iter_can_iterate_over_a_subset() {
    let mut count = 0;
    let mut iter = set0().iter(1000);
    while iter.value().is_some() {
        if iter.from() > 2000 {
            break;
        }
        assert_eq!(iter.to(), iter.from() + 1 + (iter.from() % 4));
        iter.next();
        count += 1;
    }
    assert_eq!(count, 1003);
}

// between

#[test]
fn between_iterates_over_ranges() {
    let mut found = 0;
    set0().between(100, 200, |from, to, _| {
        assert_eq!(to, from + 1 + (from % 4));
        assert!(to >= 100);
        assert!(from <= 200);
        found += 1;
        true
    });
    assert_eq!(found, 103);
}

#[test]
fn between_returns_ranges_in_a_zero_length_set() {
    let set = RangeSet::of(vec![mk(0, 0, spec())], false);
    let mut found = Vec::new();
    set.between(0, 0, |from, to, _| {
        found.extend([from, to]);
        true
    });
    assert_eq!(found, [0, 0]);
}
