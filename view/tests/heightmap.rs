//! Port of `test/test-heightmap.ts` from @codemirror/view 6.43.15.

use std::sync::Arc;

use div::{Document, Element};

use editor_state::{Range, Text};
use editor_view::__test::{
    clear_height_change_flag, height_change_flag, HeightMap, HeightOracle, MeasuredHeights,
    QueryType,
};
use editor_view::{
    BlockInfo, BlockKind, BlockType, ChangedRange, Decoration, MarkSpec, ReplaceSpec, WidgetSpec,
    WidgetType,
};

const BY_H: QueryType = QueryType::ByHeight;
const BY_P: QueryType = QueryType::ByPos;

fn o(doc: &Text) -> HeightOracle {
    let mut oracle = HeightOracle::new(false);
    oracle.set_doc(doc.clone());
    oracle
}

fn mk(text: &Text, deco: Vec<Range<Decoration>>) -> HeightMap {
    HeightMap::empty().apply_changes(
        &[Decoration::set(deco, true)],
        &Text::empty(),
        &mut o(text),
        &[ChangedRange::new(0, 0, 0, text.len())],
    )
}

fn doc(line_len: &[usize]) -> Text {
    let lines: Vec<String> = line_len.iter().map(|&len| "x".repeat(len)).collect();
    Text::of(&lines).unwrap()
}

fn text_of(lines: &[&str]) -> Text {
    Text::of(lines).unwrap()
}

struct MyWidget(f64);

impl WidgetType for MyWidget {
    fn to_dom(&self, document: &Document) -> Result<Element, div::Error> {
        Element::create(document, "span")
    }

    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }

    fn estimated_height(&self) -> f64 {
        self.0
    }
}

struct NoHeightWidget;

impl WidgetType for NoHeightWidget {
    fn to_dom(&self, document: &Document) -> Result<Element, div::Error> {
        Element::create(document, "span")
    }
}

fn widget(height: f64, side: i32, block: bool) -> Arc<Decoration> {
    Decoration::widget(WidgetSpec {
        side,
        block,
        ..WidgetSpec::new(MyWidget(height))
    })
}

fn replace_widget(height: f64) -> Arc<Decoration> {
    Decoration::replace(ReplaceSpec {
        widget: Some(editor_view::Widget::new(MyWidget(height))),
        block: true,
        ..ReplaceSpec::default()
    })
}

fn replace() -> Arc<Decoration> {
    Decoration::replace(ReplaceSpec::default())
}

fn blocks(info: &BlockInfo) -> &[BlockInfo] {
    match info.kind() {
        BlockKind::Blocks(blocks) => blocks,
        BlockKind::Type(ty) => panic!("expected a composite line, got {ty:?}"),
    }
}

fn block_type(info: &BlockInfo) -> BlockType {
    match info.kind() {
        BlockKind::Type(ty) => ty,
        BlockKind::Blocks(_) => panic!("expected a single block"),
    }
}

#[test]
fn starts_empty() {
    let empty = HeightMap::empty();
    assert_eq!(empty.length(), 0);
    assert_eq!(empty.size(), 1);
}

#[test]
fn grows_to_match_the_document() {
    assert_eq!(mk(&doc(&[10, 10, 8]), vec![]).length(), 30);
}

#[test]
fn separates_lines_with_decorations_on_them() {
    let map = mk(
        &doc(&[10, 10, 20, 5]),
        vec![
            widget(20.0, 0, false).at(5).unwrap(),
            replace().range(25, 46).unwrap(),
        ],
    );
    assert_eq!(map.length(), 48);
    assert_eq!(map.to_string(), "line(10:20) gap(10) line(26-21)");
}

#[test]
fn ignores_irrelevant_decorations() {
    let map = mk(
        &doc(&[10, 10, 20, 5]),
        vec![
            Decoration::widget(WidgetSpec::new(NoHeightWidget))
                .at(5)
                .unwrap(),
            Decoration::mark(MarkSpec {
                class: Some("ahah".into()),
                ..MarkSpec::default()
            })
            .range(25, 46)
            .unwrap(),
        ],
    );
    assert_eq!(map.length(), 48);
    assert_eq!(map.to_string(), "gap(48)");
}

#[test]
fn drops_decorations_from_the_tree_when_they_are_deleted() {
    let text = doc(&[20]);
    let mut map = mk(&text, vec![widget(20.0, 0, false).at(5).unwrap()]);
    assert_eq!(map.to_string(), "line(20:20)");
    map = map.apply_changes(&[], &text, &mut o(&text), &[ChangedRange::new(5, 5, 5, 5)]);
    assert_eq!(map.to_string(), "line(20)");
}

#[test]
fn updates_the_length_of_replaced_decorations_for_changes() {
    let text = doc(&[20]);
    let mut map = mk(&text, vec![replace().range(5, 15).unwrap()]);
    map = map.apply_changes(
        &[Decoration::set(
            vec![replace().range(5, 10).unwrap()],
            false,
        )],
        &text,
        &mut o(&text.replace(7, 12, &Text::empty())),
        &[ChangedRange::new(7, 12, 7, 7)],
    );
    assert_eq!(map.to_string(), "line(15-5)");
}

#[test]
fn stores_information_about_block_widgets() {
    let text = doc(&[3, 3, 3]);
    let oracle = o(&text);
    let mut map = mk(
        &text,
        vec![
            widget(10.0, -1, true).at(0).unwrap(),
            widget(13.0, -1, true).at(0).unwrap(),
            widget(5.0, 1, true).at(3).unwrap(),
        ],
    );
    assert_eq!(map.to_string(), "block(0)-block(0)-line(3)-block(0) gap(7)");
    assert_eq!(map.height(), 28.0 + 3.0 * oracle.line_height);
    let line = map.line_at(0.0, BY_P, &oracle, 0.0, 0);
    let heights: Vec<f64> = blocks(&line).iter().map(|b| b.height).collect();
    assert_eq!(heights, [10.0, 13.0, oracle.line_height, 5.0]);
    assert_eq!(
        map.line_at(4.0, BY_P, &oracle, 0.0, 0).top,
        28.0 + oracle.line_height
    );
    map = map.update_height(
        &oracle,
        0,
        false,
        Some(&mut MeasuredHeights::new(
            0,
            vec![8.0, 12.0, 10.0, 20.0, 40.0, 20.0],
        )),
    );
    assert_eq!(
        map.to_string(),
        "block(0)-block(0)-line(3)-block(0) line(3) line(3)"
    );
    assert_eq!(map.height(), 110.0);
}

#[test]
fn stores_information_about_block_ranges() {
    let text = doc(&[3, 3, 3, 3, 3, 3]);
    let mut map = mk(
        &text,
        vec![
            widget(10.0, -1, true).at(4).unwrap(),
            replace_widget(40.0).range(4, 11).unwrap(),
            widget(15.0, 1, true).at(11).unwrap(),
            // Widgets after the start or before the end of an inclusive
            // range get covered.
            widget(20.0, 1, true).at(16).unwrap(),
            replace_widget(50.0).range(16, 19).unwrap(),
            widget(10.0, -1, true).at(19).unwrap(),
        ],
    );
    assert_eq!(
        map.to_string(),
        "gap(3) block(0)-block(7)-block(0) gap(3) block(3) gap(3)"
    );
    map = map.update_height(
        &o(&text),
        0,
        false,
        Some(&mut MeasuredHeights::new(
            4,
            vec![5.0, 5.0, 5.0, 5.0, 10.0, 5.0],
        )),
    );
    assert_eq!(map.height(), o(&text).line_height + 35.0);
}

#[test]
fn handles_empty_lines_correctly() {
    let text = doc(&[0, 0, 0, 0, 0]);
    let mut map = mk(
        &text,
        vec![
            widget(10.0, -1, true).at(1).unwrap(),
            replace_widget(20.0).range(2, 2).unwrap(),
            widget(30.0, 1, true).at(3).unwrap(),
        ],
    );
    assert_eq!(
        map.to_string(),
        "gap(0) block(0)-line(0) block(0) line(0)-block(0) gap(0)"
    );
    map = map.apply_changes(
        &[],
        &text,
        &mut o(&text.replace(1, 3, &text_of(&["y"]))),
        &[ChangedRange::new(1, 3, 1, 2)],
    );
    assert_eq!(map.to_string(), "gap(3)");
}

#[test]
fn joins_ranges() {
    let text = doc(&[10, 10, 10, 10]);
    let mut map = mk(&text, vec![replace().range(16, 27).unwrap()]);
    assert_eq!(map.to_string(), "gap(10) line(21-11) gap(10)");
    map = map.apply_changes(
        &[],
        &text,
        &mut o(&text.replace(5, 38, &text_of(&["yyy"]))),
        &[ChangedRange::new(5, 38, 5, 8)],
    );
    assert_eq!(map.to_string(), "gap(13)");
}

#[test]
fn joins_lines() {
    let text = doc(&[10, 10, 10]);
    let mut map = mk(
        &text,
        vec![
            replace().range(2, 5).unwrap(),
            widget(20.0, 0, false).at(24).unwrap(),
        ],
    );
    assert_eq!(map.to_string(), "line(10-3) gap(10) line(10:20)");
    map = map.apply_changes(
        &[Decoration::set(
            vec![
                replace().range(2, 5).unwrap(),
                widget(20.0, 0, false).at(12).unwrap(),
            ],
            false,
        )],
        &text,
        &mut o(&text.replace(10, 22, &Text::empty())),
        &[ChangedRange::new(10, 22, 10, 10)],
    );
    assert_eq!(map.to_string(), "line(20-3:20)");
}

#[test]
fn materializes_lines_for_measured_heights() {
    let text = doc(&[10, 10, 10, 10]);
    let oracle = o(&text);
    let map = mk(&text, vec![]).update_height(
        &oracle,
        0,
        false,
        Some(&mut MeasuredHeights::new(11, vec![28.0, 14.0, 5.0])),
    );
    assert_eq!(map.to_string(), "gap(10) line(10) line(10) line(10)");
    assert_eq!(map.height(), 61.0);
}

#[test]
fn does_not_set_the_height_changed_flag_for_small_within_line_replacements() {
    let text = doc(&[10, 10, 10]);
    let mut oracle = o(&text);
    let mut map = mk(&text, vec![]).update_height(
        &oracle,
        0,
        false,
        Some(&mut MeasuredHeights::new(0, vec![12.0, 12.0, 12.0])),
    );
    let new_text = text.replace(21, 21, &text_of(&["!"]));
    oracle.set_doc(new_text);
    map = map.apply_changes(
        &[],
        &text,
        &mut oracle,
        &[ChangedRange::new(21, 21, 21, 22)],
    );
    clear_height_change_flag();
    let _map = map.update_height(
        &oracle,
        0,
        false,
        Some(&mut MeasuredHeights::new(0, vec![12.0, 12.0, 12.0])),
    );
    assert!(!height_change_flag());
}

#[test]
fn can_update_lines_across_the_tree() {
    let text = doc(&[10; 100]);
    let oracle = o(&text);
    let mut map = mk(&text, vec![]).update_height(
        &oracle,
        0,
        false,
        Some(&mut MeasuredHeights::new(0, vec![12.0; 100])),
    );
    assert_eq!(map.height(), 1200.0);
    assert_eq!(map.size(), 100);
    map = map.update_height(
        &oracle,
        0,
        false,
        Some(&mut MeasuredHeights::new(55, vec![10.0; 90])),
    );
    assert_eq!(map.height(), 1020.0);
    assert_eq!(map.size(), 100);
}

fn depth(map: &HeightMap) -> usize {
    match map.children() {
        Some((left, right)) => depth(left).max(depth(right)) + 1,
        None => 1,
    }
}

#[test]
fn balances_a_big_tree() {
    let text = doc(&[30; 100]);
    let mut oracle = o(&text);
    let mut map = mk(&text, vec![]).update_height(
        &oracle,
        0,
        false,
        Some(&mut MeasuredHeights::new(0, vec![15.0; 100])),
    );
    assert_eq!(map.height(), 1500.0);
    assert_eq!(map.size(), 100);
    assert!(depth(&map) < 9);
    let text2 = text.replace(0, 31 * 80, &Text::empty());
    map = map.apply_changes(
        &[],
        &text,
        &mut o(&text2),
        &[ChangedRange::new(0, 31 * 80, 0, 0)],
    );
    assert_eq!(map.size(), 20);
    assert!(depth(&map) < 7);
    let len = text2.len();
    let inserted = "\nfoo".repeat(200);
    let lines: Vec<&str> = inserted.split('\n').collect();
    let text3 = text2.replace(len, len, &text_of(&lines));
    map = map.apply_changes(
        &[],
        &text2,
        &mut o(&text3),
        &[ChangedRange::new(len, len, len, len + 800)],
    );
    oracle.set_doc(text3);
    map = map.update_height(
        &oracle,
        0,
        false,
        Some(&mut MeasuredHeights::new(len + 1, vec![10.0; 200])),
    );
    assert_eq!(map.size(), 220);
    assert!(depth(&map) < 12);
}

#[test]
fn can_handle_inserting_a_line_break() {
    let text = doc(&[3, 3, 3]);
    let mut oracle = o(&text);
    let mut map = mk(&text, vec![]).update_height(
        &oracle,
        0,
        false,
        Some(&mut MeasuredHeights::new(0, vec![10.0, 10.0, 10.0])),
    );
    assert_eq!(map.size(), 3);
    let text2 = text.replace(3, 3, &text_of(&["", ""]));
    oracle.set_doc(text2);
    map = map
        .apply_changes(&[], &text, &mut oracle, &[ChangedRange::new(3, 3, 3, 4)])
        .update_height(
            &oracle,
            0,
            false,
            Some(&mut MeasuredHeights::new(0, vec![10.0; 4])),
        );
    assert_eq!(map.size(), 4);
    assert_eq!(map.height(), 40.0);
}

#[test]
fn can_handle_insertion_in_the_middle_of_a_line() {
    let text = doc(&[3, 3, 3]);
    let oracle = o(&text);
    let mut map = mk(&text, vec![]).update_height(
        &oracle,
        0,
        false,
        Some(&mut MeasuredHeights::new(0, vec![10.0, 10.0, 10.0])),
    );
    let text2 = text.replace(5, 5, &text_of(&["foo", "bar", "baz", "bug"]));
    map = map
        .apply_changes(
            &[],
            &text,
            &mut o(&text2),
            &[ChangedRange::new(5, 5, 5, 20)],
        )
        .update_height(
            &o(&text2),
            0,
            false,
            Some(&mut MeasuredHeights::new(0, vec![10.0; 6])),
        );
    assert_eq!(map.size(), 6);
    assert_eq!(map.height(), 60.0);
}

#[test]
fn properly_shrinks_gaps_when_partially_replaced() {
    let text = doc(&[9; 7]);
    let mut map = mk(&text, vec![]).update_height(
        &o(&text),
        0,
        false,
        Some(&mut MeasuredHeights::new(20, vec![10.0, 10.0, 10.0])),
    );
    assert_eq!(map.to_string(), "gap(19) line(9) line(9) line(9) gap(19)");
    map = map.apply_changes(
        &[Decoration::set(
            vec![replace().range(15, 55).unwrap()],
            false,
        )],
        &text,
        &mut o(&text),
        &[ChangedRange::new(15, 55, 15, 55)],
    );
    assert_eq!(map.to_string(), "gap(9) line(49-40) gap(9)");
}

mod block_at {
    use super::*;

    #[test]
    fn finds_blocks_in_a_gap() {
        let text = doc(&[3, 3, 3, 3, 3]);
        let map = mk(&text, vec![]);
        let block1 = map.block_at(0.0, &o(&text), 0.0, 0);
        assert_eq!((block1.from, block1.to()), (0, 3));
        assert_eq!(block1.top, 0.0);
        assert!(block1.bottom() > 0.0);
        assert_eq!(block_type(&block1), BlockType::Text);
        let block2 = map.block_at(block1.bottom() + 1.0, &o(&text), 0.0, 0);
        assert_eq!((block2.from, block2.to()), (4, 7));
        assert_eq!(block2.top, block1.bottom());
        assert!(block2.bottom() > block1.bottom());
        let block3 = map.block_at(1e9, &o(&text), 0.0, 0);
        assert_eq!((block3.from, block3.to()), (16, 19));
        assert_eq!(block3.bottom(), map.height());
    }

    #[test]
    fn finds_blocks_in_lines() {
        let text = doc(&[3, 3, 3, 3]);
        let map = mk(&text, vec![]).update_height(
            &o(&text),
            0,
            false,
            Some(&mut MeasuredHeights::new(0, vec![10.0, 20.0, 10.0, 30.0])),
        );
        let oracle = o(&text);
        let block1 = map.block_at(-100.0, &oracle, 0.0, 0);
        assert_eq!((block1.from, block1.to()), (0, 3));
        assert_eq!((block1.top, block1.bottom()), (0.0, 10.0));
        assert_eq!(block_type(&block1), BlockType::Text);
        let block2 = map.block_at(39.0, &oracle, 0.0, 0);
        assert_eq!((block2.from, block2.to()), (8, 11));
        assert_eq!((block2.top, block2.bottom()), (30.0, 40.0));
        let block3 = map.block_at(77.0, &oracle, 0.0, 0);
        assert_eq!((block3.from, block3.to()), (12, 15));
        assert_eq!((block3.top, block3.bottom()), (40.0, 70.0));
    }

    #[test]
    fn finds_widget_blocks() {
        let text = doc(&[3, 3, 3, 3]);
        let map = mk(
            &text,
            vec![
                widget(100.0, -1, true).at(4).unwrap(),
                replace_widget(30.0).range(8, 11).unwrap(),
                widget(0.0, 1, true).at(15).unwrap(),
            ],
        );
        let oracle = o(&text);
        let block1 = map.block_at(0.0, &oracle, 0.0, 0);
        assert_eq!((block1.from, block1.to()), (0, 3));
        let block2 = map.block_at(block1.height + 1.0, &oracle, 0.0, 0);
        assert_eq!((block2.from, block2.to()), (4, 4));
        assert_eq!((block2.top, block2.height), (block1.height, 100.0));
        assert_eq!(block_type(&block2), BlockType::WidgetBefore);
        let top3 = block2.bottom() + block1.height;
        let block3 = map.block_at(top3 + 10.0, &oracle, 0.0, 0);
        assert_eq!((block3.from, block3.to()), (8, 11));
        assert_eq!((block3.top, block3.height), (top3, 30.0));
        assert_eq!(block_type(&block3), BlockType::WidgetRange);
        let block4 = map.block_at(block3.bottom() + block1.height, &oracle, 0.0, 0);
        assert_eq!(block_type(&block4), BlockType::WidgetAfter);
    }
}

fn assert_eq_block(a: &BlockInfo, b: &BlockInfo) {
    assert_eq!(
        (a.from, a.to(), a.top, a.bottom()),
        (b.from, b.to(), b.top, b.bottom()),
        "blocks differ"
    );
}

mod line_at {
    use super::*;

    #[test]
    fn finds_lines_in_gaps() {
        let text = doc(&[3, 3, 3, 3]);
        let map = mk(&text, vec![]);
        let oracle = o(&text);
        let line1 = map.line_at(0.0, BY_P, &oracle, 0.0, 0);
        assert_eq!((line1.from, line1.to()), (0, 3));
        assert_eq!(line1.top, 0.0);
        assert_eq_block(&map.line_at(0.0, BY_H, &oracle, 0.0, 0), &line1);
        let line2 = map.line_at((line1.to() + 1) as f64, BY_P, &oracle, 0.0, 0);
        assert_eq!((line2.from, line2.to()), (4, 7));
        assert_eq!(line2.top, line1.bottom());
        assert_eq_block(
            &map.line_at(line1.bottom() + 1.0, BY_H, &oracle, 0.0, 0),
            &line2,
        );
        let line3 = map.line_at(15.0, BY_P, &oracle, 0.0, 0);
        assert_eq!((line3.from, line3.to()), (12, 15));
        assert_eq!(line3.bottom(), map.height());
        assert_eq_block(&map.line_at(1e9, BY_H, &oracle, 0.0, 0), &line3);
    }

    #[test]
    fn finds_lines_in_lines() {
        let text = doc(&[3, 3, 3, 3]);
        let oracle = o(&text);
        let map = mk(&text, vec![]).update_height(
            &oracle,
            0,
            false,
            Some(&mut MeasuredHeights::new(0, vec![10.0, 10.0, 20.0, 10.0])),
        );
        let line1 = map.line_at(0.0, BY_P, &oracle, 0.0, 0);
        assert_eq!((line1.from, line1.to()), (0, 3));
        assert_eq!((line1.top, line1.bottom()), (0.0, 10.0));
        assert_eq_block(&map.line_at(9.0, BY_H, &oracle, 0.0, 0), &line1);
        let line2 = map.line_at(9.0, BY_P, &oracle, 0.0, 0);
        assert_eq!((line2.from, line2.to()), (8, 11));
        assert_eq!((line2.top, line2.bottom()), (20.0, 40.0));
        assert_eq_block(&map.line_at(39.0, BY_H, &oracle, 0.0, 0), &line2);
    }

    #[test]
    fn includes_adjacent_widgets_in_lines() {
        let text = doc(&[3, 3, 3, 3]);
        let oracle = o(&text);
        let map = mk(
            &text,
            vec![
                widget(100.0, -1, true).at(4).unwrap(),
                replace_widget(30.0).range(7, 8).unwrap(),
                widget(0.0, 1, true).at(15).unwrap(),
            ],
        );
        let line1 = map.line_at(4.0, BY_P, &oracle, 0.0, 0);
        assert_eq!((line1.from, line1.to()), (4, 11));
        assert_eq!(blocks(&line1).len(), 4);
        assert_eq_block(&map.line_at(line1.top + 1.0, BY_H, &oracle, 0.0, 0), &line1);
        assert_eq_block(
            &map.line_at(line1.bottom() - 1.0, BY_H, &oracle, 0.0, 0),
            &line1,
        );
        assert_eq_block(
            &map.line_at(line1.top + line1.height / 2.0, BY_H, &oracle, 0.0, 0),
            &line1,
        );
        assert_eq_block(&map.line_at(5.0, BY_P, &oracle, 0.0, 0), &line1);
        assert_eq_block(&map.line_at(7.0, BY_P, &oracle, 0.0, 0), &line1);
        assert_eq_block(&map.line_at(11.0, BY_P, &oracle, 0.0, 0), &line1);
        let line2 = map.line_at(map.height(), BY_H, &oracle, 0.0, 0);
        assert_eq!((line2.from, line2.to()), (12, 15));
        assert_eq!(blocks(&line2).len(), 2);
        assert_eq_block(&map.line_at(line2.top + 1.0, BY_H, &oracle, 0.0, 0), &line2);
    }
}
