//! Port of `@codemirror/state` 6.7.6 `test/test-selection.ts`.

use editor_state::{ChangeSet, ChangeSpec, EditorSelection, SelectionRange};

fn describe(sel: &EditorSelection) -> String {
    sel.ranges()
        .iter()
        .map(|r| format!("{}/{}", r.anchor(), r.head()))
        .collect::<Vec<_>>()
        .join(",")
}

fn create(ranges: Vec<SelectionRange>, main: usize) -> EditorSelection {
    EditorSelection::create(ranges, main).unwrap()
}

#[test]
fn stores_ranges_with_a_primary_range() {
    let sel = create(
        vec![
            EditorSelection::range(0, 1),
            EditorSelection::range(3, 2),
            EditorSelection::range(4, 5),
        ],
        1,
    );
    assert_eq!(sel.main().from(), 2);
    assert_eq!(sel.main().to(), 3);
    assert_eq!(sel.main().anchor(), 3);
    assert_eq!(sel.main().head(), 2);
    assert_eq!(describe(&sel), "0/1,3/2,4/5");
}

#[test]
fn merges_and_sorts_ranges_when_normalizing() {
    let sel = create(
        vec![
            EditorSelection::range(10, 12),
            EditorSelection::range(6, 7),
            EditorSelection::range(4, 5),
            EditorSelection::range(3, 4),
            EditorSelection::range(0, 6),
            EditorSelection::range(7, 8),
            EditorSelection::range(9, 13),
            EditorSelection::range(13, 14),
        ],
        0,
    );
    assert_eq!(describe(&sel), "0/6,6/7,7/8,9/13,13/14");
}

#[test]
fn merges_adjacent_point_ranges_when_normalizing() {
    let sel = create(
        vec![
            EditorSelection::range(10, 12),
            EditorSelection::range(12, 12),
            EditorSelection::range(12, 12),
            EditorSelection::range(10, 10),
            EditorSelection::range(8, 10),
        ],
        0,
    );
    assert_eq!(describe(&sel), "8/10,10/12");
}

#[test]
fn preserves_the_direction_of_the_last_range_when_merging_ranges() {
    let sel = create(
        vec![EditorSelection::range(0, 2), EditorSelection::range(10, 1)],
        0,
    );
    assert_eq!(describe(&sel), "10/0");
}

// The cases below are not upstream tests; they check the port's API
// against upstream's documented behaviour.

#[test]
fn rejects_an_empty_selection() {
    assert!(EditorSelection::create(Vec::new(), 0).is_err());
}

#[test]
fn maps_ranges_through_changes() {
    let changes = ChangeSet::of(ChangeSpec::insert(2, "abc"), 10, None).unwrap();
    let sel = create(
        vec![
            EditorSelection::cursor(2),
            EditorSelection::range(4, 1),
            EditorSelection::range(6, 8),
        ],
        0,
    );
    // The cursor stays before the insertion with assoc -1 and then merges
    // into the range that grew over it, taking the later range's
    // direction.
    let mapped = sel.map(changes.desc(), -1);
    assert_eq!(describe(&mapped), "1/7,9/11");
    let single = EditorSelection::single(2, 2).map(changes.desc(), 1);
    assert_eq!(describe(&single), "5/5");
}

#[test]
fn associates_ranges_away_from_the_head() {
    assert_eq!(EditorSelection::range(1, 4).assoc(), -1);
    assert_eq!(EditorSelection::range(4, 1).assoc(), 1);
    assert_eq!(EditorSelection::cursor(3).assoc(), 0);
    assert_eq!(EditorSelection::cursor(3).bidi_level(), None);
    assert_eq!(
        EditorSelection::cursor_with(3, -1, Some(9), Some(12.5)).bidi_level(),
        Some(6)
    );
}

#[test]
fn extends_ranges() {
    let range = EditorSelection::range(5, 8);
    let extended = range.extend(2, 2, 0);
    assert_eq!((extended.anchor(), extended.head()), (5, 2));
    let covering = range.extend(3, 10, 0);
    assert_eq!((covering.anchor(), covering.head()), (3, 10));
}

#[test]
fn adds_and_replaces_ranges() {
    let sel = EditorSelection::single(1, 1).add_range(EditorSelection::cursor(5), true);
    assert_eq!(describe(&sel), "1/1,5/5");
    assert_eq!(sel.main().head(), 5);
    let replaced = sel.replace_range(EditorSelection::range(0, 3), None);
    // The new range swallows the cursor at 1 when the ranges are merged.
    assert_eq!(describe(&replaced), "0/3");
    assert_eq!(sel.as_single().ranges().len(), 1);
}

#[test]
fn round_trips_json() {
    let sel = create(
        vec![EditorSelection::range(0, 1), EditorSelection::range(5, 3)],
        1,
    );
    let (ranges, main) = sel.to_json();
    let read = EditorSelection::from_json(&ranges, main).unwrap();
    assert!(read.eq_with(&sel, false));
    assert!(EditorSelection::from_json(&ranges, 2).is_err());
}
