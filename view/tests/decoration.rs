//! Port-specific tests for `decoration.ts`, `attributes.ts` and
//! `ChangedRange`. Upstream checks decorations through its browser tests
//! (`webtest-draw-decoration.ts`), which need the view's DOM; these check
//! the parts that need none, against upstream's source.

use std::sync::Arc;

use div::{Document, Element};

use editor_state::{ChangeSet, ChangeSpec, MapMode, RangeError, RangeSet, RangeValue};
use editor_view::__test::{add_range, height_relevant_deco_changes};
use editor_view::{
    attrs_eq, combine_attrs, Attrs, BlockType, BlockWrapper, BlockWrapperSpec, ChangedRange,
    Decoration, DecorationKind, LineSpec, MarkSpec, ReplaceSpec, Widget, WidgetSpec, WidgetType,
};

struct Label(&'static str);

impl WidgetType for Label {
    fn to_dom(&self, document: &Document) -> Result<Element, div::Error> {
        Element::create(document, "span")
    }

    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

struct Tall;

impl WidgetType for Tall {
    fn to_dom(&self, document: &Document) -> Result<Element, div::Error> {
        Element::create(document, "span")
    }

    fn estimated_height(&self) -> f64 {
        20.0
    }
}

struct Breaking;

impl WidgetType for Breaking {
    fn to_dom(&self, document: &Document) -> Result<Element, div::Error> {
        Element::create(document, "span")
    }

    fn line_breaks(&self) -> usize {
        2
    }
}

fn attrs(pairs: &[(&str, &str)]) -> Attrs {
    pairs.iter().copied().collect()
}

fn mark(class: &str) -> Arc<Decoration> {
    Decoration::mark(MarkSpec {
        class: Some(class.into()),
        ..MarkSpec::default()
    })
}

fn widget<W: WidgetType>(w: W, side: i32, block: bool) -> Arc<Decoration> {
    Decoration::widget(WidgetSpec {
        side,
        block,
        ..WidgetSpec::new(w)
    })
}

#[test]
fn combines_attributes_like_upstream() {
    let combined = combine_attrs(
        &attrs(&[("class", "b"), ("style", "color: red"), ("title", "t")]),
        attrs(&[("class", "a"), ("style", "top: 0")]),
    );
    assert_eq!(combined.get("class"), Some("a b"));
    assert_eq!(combined.get("style"), Some("top: 0;color: red"));
    assert_eq!(combined.get("title"), Some("t"));
    let order: Vec<&str> = combined.iter().map(|(k, _)| k).collect();
    assert_eq!(order, ["class", "style", "title"]);
}

#[test]
fn compares_attributes_in_any_order() {
    let a = attrs(&[("class", "x"), ("id", "y")]);
    let b = attrs(&[("id", "y"), ("class", "x")]);
    assert!(attrs_eq(Some(&a), Some(&b), None));
    assert!(attrs_eq(None, Some(&Attrs::new()), None));
    let c = attrs(&[("class", "x"), ("id", "y"), ("data-v", "1")]);
    assert!(!attrs_eq(Some(&a), Some(&c), None));
    assert!(attrs_eq(Some(&a), Some(&c), Some("data-v")));
    let d = attrs(&[("class", "x"), ("id", "z")]);
    assert!(!attrs_eq(Some(&a), Some(&d), None));
}

#[test]
fn mark_attributes_put_the_class_first() {
    let deco = Decoration::mark(MarkSpec {
        class: Some("cm-a".into()),
        attributes: Some(attrs(&[("title", "x"), ("class", "cm-b")])),
        tag_name: Some("em".into()),
        ..MarkSpec::default()
    });
    let DecorationKind::Mark { tag_name, attrs } = deco.kind() else {
        panic!("not a mark");
    };
    assert_eq!(tag_name, "em");
    let pairs: Vec<(&str, &str)> = attrs.iter().collect();
    assert_eq!(pairs, [("class", "cm-a cm-b"), ("title", "x")]);
    let plain = mark("c");
    let DecorationKind::Mark { tag_name, .. } = plain.kind() else {
        panic!("not a mark");
    };
    assert_eq!(tag_name, "span");
}

#[test]
fn validates_ranges_with_upstream_messages() {
    let err = |message: &str| Err::<(), _>(RangeError(message.to_string()));
    assert_eq!(
        mark("a").range(3, 3).map(|_| ()),
        err("Mark decorations may not be empty")
    );
    assert!(mark("a").range(3, 4).is_ok());
    let line = Decoration::line(LineSpec::default());
    assert_eq!(
        line.range(0, 1).map(|_| ()),
        err("Line decoration ranges must be zero-length")
    );
    assert_eq!(
        widget(Label("w"), 0, false).range(1, 2).map(|_| ()),
        err("Widget decorations can only have zero-length ranges")
    );
    let replace = Decoration::replace(ReplaceSpec::default());
    assert_eq!(
        replace.range(5, 4).map(|_| ()),
        err("Invalid range for replacement decoration")
    );
    // An empty non-inclusive inline replacement starts after it ends.
    assert_eq!(
        replace.range(4, 4).map(|_| ()),
        err("Invalid range for replacement decoration")
    );
    let inclusive = Decoration::replace(ReplaceSpec {
        inclusive: Some(true),
        ..ReplaceSpec::default()
    });
    assert!(inclusive.range(4, 4).is_ok());
}

#[test]
fn sides_follow_upstream() {
    assert_eq!(mark("a").start_side(), 500_000_000);
    assert_eq!(mark("a").end_side(), -600_000_000);
    let inclusive = Decoration::mark(MarkSpec {
        inclusive: Some(true),
        inclusive_end: Some(false),
        ..MarkSpec::default()
    });
    assert_eq!(
        (inclusive.start_side(), inclusive.end_side()),
        (-1, -600_000_000)
    );
    assert_eq!(widget(Label("w"), 3, false).start_side(), 100_000_003);
    assert_eq!(widget(Label("w"), -3, false).start_side(), -100_000_003);
    assert_eq!(widget(Label("w"), 20_000, false).start_side(), 100_010_000);
    assert_eq!(widget(Label("w"), 2, true).start_side(), 300_000_002);
    assert_eq!(widget(Label("w"), 0, true).start_side(), -400_000_000);
    let ordered = Decoration::widget(WidgetSpec {
        side: 2,
        block: true,
        inline_order: true,
        ..WidgetSpec::new(Label("w"))
    });
    assert_eq!(ordered.start_side(), 100_000_002);
    let block = Decoration::replace(ReplaceSpec {
        block: true,
        ..ReplaceSpec::default()
    });
    assert_eq!(
        (block.start_side(), block.end_side()),
        (-300_000_001, 200_000_001)
    );
    let inline = Decoration::replace(ReplaceSpec::default());
    assert_eq!(
        (inline.start_side(), inline.end_side()),
        (499_999_999, -599_999_999)
    );
    let gap = Decoration::replace(ReplaceSpec {
        is_block_gap: true,
        ..ReplaceSpec::default()
    });
    assert_eq!(
        (gap.start_side(), gap.end_side()),
        (-500_000_000, 400_000_000)
    );
    let line = Decoration::line(LineSpec::default());
    assert_eq!(
        (line.start_side(), line.end_side()),
        (-200_000_000, -200_000_000)
    );
}

#[test]
fn kinds_map_modes_and_block_types() {
    assert!(!mark("a").point());
    assert_eq!(mark("a").map_mode(), MapMode::TrackDel);
    let line = Decoration::line(LineSpec::default());
    assert!(line.point());
    assert_eq!(line.map_mode(), MapMode::TrackBefore);
    let inline = widget(Label("w"), 1, false);
    assert_eq!(inline.map_mode(), MapMode::TrackDel);
    let before = widget(Label("w"), -1, true);
    assert_eq!(before.map_mode(), MapMode::TrackBefore);
    assert_eq!(before.block_type(), BlockType::WidgetBefore);
    let after = widget(Label("w"), 1, true);
    assert_eq!(after.map_mode(), MapMode::TrackAfter);
    assert_eq!(after.block_type(), BlockType::WidgetAfter);
    let range = Decoration::replace(ReplaceSpec {
        block: true,
        ..ReplaceSpec::default()
    });
    assert_eq!(range.block_type(), BlockType::WidgetRange);
    assert_eq!(range.map_mode(), MapMode::TrackBefore);
}

#[test]
fn compares_decorations() {
    assert!(mark("a").range_eq(&mark("a")));
    assert!(!mark("a").range_eq(&mark("b")));
    let line = |class: &str| {
        Decoration::line(LineSpec {
            class: Some(class.into()),
            ..LineSpec::default()
        })
    };
    assert!(line("x").range_eq(&line("x")));
    assert!(!line("x").range_eq(&line("y")));
    assert!(!line("x").range_eq(&mark("x")));
    // Widgets compare by type, then by `eq`.
    assert!(widget(Label("a"), 0, false).range_eq(&widget(Label("a"), 0, false)));
    assert!(!widget(Label("a"), 0, false).range_eq(&widget(Label("b"), 0, false)));
    assert!(!widget(Label("a"), 0, false).range_eq(&widget(Label("a"), 1, false)));
    assert!(!widget(Label("a"), 0, false).range_eq(&widget(Label("a"), 0, true)));
    // The default `eq` is false, so only the same instance is equal.
    assert!(!widget(Tall, 0, false).range_eq(&widget(Tall, 0, false)));
    let shared = Widget::new(Tall);
    assert!(shared.compare(&shared.clone()));
    assert!(!shared.compare(&Widget::new(Label("a"))));
    assert!(shared.downcast_ref::<Tall>().is_some());
}

#[test]
fn height_relevance() {
    assert!(!mark("a").height_relevant());
    assert!(!widget(Label("w"), 0, false).height_relevant());
    assert!(widget(Tall, 0, false).height_relevant());
    assert!(widget(Breaking, 0, false).height_relevant());
    assert!(widget(Label("w"), 0, true).height_relevant());
    assert!(!widget(Label("w"), 0, false).has_height());
    assert!(widget(Tall, 0, false).has_height());
}

#[test]
fn sorts_and_maps_decoration_sets() {
    let before = widget(Label("before"), -1, false);
    let after = widget(Label("after"), 1, false);
    let block = widget(Label("block"), -1, true);
    let line = Decoration::line(LineSpec::default());
    let set = Decoration::set(
        vec![
            after.at(4).unwrap(),
            before.at(4).unwrap(),
            line.at(4).unwrap(),
            block.at(4).unwrap(),
        ],
        true,
    );
    let mut order = Vec::new();
    let mut cursor = set.iter(0);
    while let Some(value) = cursor.value() {
        order.push(value.start_side());
        cursor.next();
    }
    assert_eq!(
        order,
        [-400_000_001, -200_000_000, -100_000_001, 100_000_001]
    );

    // Deleting the line break before a line decoration drops it
    // (TrackBefore); deleting across an inline widget drops it (TrackDel).
    let set = Decoration::set(vec![line.at(4).unwrap(), after.at(6).unwrap()], false);
    let changes = ChangeSet::of(ChangeSpec::delete(3, 4), 10, None).unwrap();
    let mapped = set.map(changes.desc());
    assert_eq!(mapped.size(), 1);
    let changes = ChangeSet::of(ChangeSpec::delete(5, 7), 10, None).unwrap();
    let mapped = set.map(changes.desc());
    assert_eq!(mapped.size(), 1);
    assert_eq!(Decoration::none().size(), 0);
}

#[test]
fn finds_height_relevant_changes() {
    let tall = Decoration::set(vec![widget(Tall, 0, false).at(5).unwrap()], false);
    let flat = Decoration::set(vec![widget(Label("w"), 0, false).at(5).unwrap()], false);
    let none: RangeSet<Decoration> = Decoration::none();
    let diff = ChangeSet::empty(20);
    assert_eq!(
        height_relevant_deco_changes(
            std::slice::from_ref(&tall),
            std::slice::from_ref(&none),
            diff.desc()
        ),
        [5, 5]
    );
    assert!(height_relevant_deco_changes(&[flat], &[none], diff.desc()).is_empty());
    assert!(height_relevant_deco_changes(
        std::slice::from_ref(&tall),
        std::slice::from_ref(&tall),
        diff.desc()
    )
    .is_empty());
}

#[test]
fn adds_ranges_with_a_margin() {
    let mut ranges = Vec::new();
    add_range(5, 5, &mut ranges, 5);
    add_range(9, 12, &mut ranges, 5);
    add_range(20, 21, &mut ranges, 5);
    assert_eq!(ranges, [5, 12, 20, 21]);
}

#[test]
fn block_wrappers() {
    let wrapper = BlockWrapper::create(BlockWrapperSpec {
        tag_name: "section".into(),
        attributes: None,
        rank: Some(150.0),
    });
    assert_eq!(wrapper.rank(), 100.0);
    assert_eq!((wrapper.start_side(), wrapper.end_side()), (-1, -1));
    let same = BlockWrapper::create(BlockWrapperSpec {
        tag_name: "section".into(),
        attributes: Some(Attrs::new()),
        rank: None,
    });
    assert_eq!(same.rank(), 50.0);
    assert!(wrapper.range_eq(&same));
    let set = BlockWrapper::set(vec![wrapper.range(0, 10)], false);
    assert_eq!(set.size(), 1);
}

#[test]
fn changed_ranges() {
    let mut set = vec![
        ChangedRange::new(0, 2, 0, 2),
        ChangedRange::new(10, 12, 10, 12),
    ];
    ChangedRange::new(5, 6, 5, 6).add_to_set(&mut set);
    assert_eq!(
        set,
        [
            ChangedRange::new(0, 2, 0, 2),
            ChangedRange::new(5, 6, 5, 6),
            ChangedRange::new(10, 12, 10, 12),
        ]
    );
    ChangedRange::new(1, 11, 1, 11).add_to_set(&mut set);
    assert_eq!(set, [ChangedRange::new(0, 12, 0, 12)]);

    // A change that grows the document by 3, then a range in unchanged
    // content after it, which maps back to old positions 3 earlier.
    let diff = [ChangedRange::new(10, 12, 10, 15)];
    assert_eq!(
        ChangedRange::extend_with_ranges(&diff, &[20, 25]),
        [
            ChangedRange::new(10, 12, 10, 15),
            ChangedRange::new(17, 22, 20, 25)
        ]
    );
    // A range touching the change joins it.
    assert_eq!(
        ChangedRange::extend_with_ranges(&diff, &[15, 18]),
        [ChangedRange::new(10, 15, 10, 18)]
    );
    assert_eq!(ChangedRange::extend_with_ranges(&diff, &[]), diff);
}
