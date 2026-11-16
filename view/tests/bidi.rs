//! Bidirectional text: the port of `bidi.ts`, against upstream's
//! `webtest-bidi.ts`.
//!
//! Upstream compares `computeOrder` with the order the browser it runs in
//! draws each case in. Here that order comes from pinned Chromium,
//! recorded by `fidelity/bidi_order.py` with upstream's own query in
//! `fidelity/fixtures/bidi-order-v1/order.json`. Upstream's positions are
//! UTF-16 code units and the port's UTF-8 bytes, so the tests convert at
//! their edges and keep upstream's numbers.

use std::collections::HashSet;

use div::Document;
use editor_state::{EditorSelection, EditorState, EditorStateConfig, Extension, Insertion};
use editor_state::{Range, RangeSet, SelectionRange, Text};
use editor_view::__test::{compute_order, move_visually};
use editor_view::{
    Attrs, BidiSpan, Decoration, Direction, EditorView, EditorViewConfig, MarkSpec, StyleSpec,
    WindowSize,
};

const ORDER: &str = include_str!("../../fidelity/fixtures/bidi-order-v1/order.json");

// The UTF-8 offset of UTF-16 offset `index` in `text`.
fn byte_at(text: &str, index: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units >= index {
            return byte;
        }
        units += ch.len_utf16();
    }
    text.len()
}

// The UTF-16 offset of UTF-8 offset `byte` in `text`.
fn units_at(text: &str, byte: usize) -> usize {
    text[..byte].encode_utf16().count()
}

// The integer arrays of `key` in the recorded order, one per case.
fn recorded(key: &str) -> Vec<Vec<usize>> {
    let start = ORDER.find(&format!("\"{key}\": [")).expect("the key") + key.len() + 4;
    let mut result = Vec::new();
    let mut current: Option<Vec<usize>> = None;
    let mut number = String::new();
    for ch in ORDER[start..].chars() {
        match ch {
            '[' => current = Some(Vec::new()),
            '0'..='9' => number.push(ch),
            ',' | ']' | ' ' | '\n' => {
                if !number.is_empty() {
                    current
                        .as_mut()
                        .expect("an array")
                        .push(number.parse().unwrap());
                    number.clear();
                }
                if ch == ']' {
                    match current.take() {
                        Some(array) => result.push(array),
                        None => break,
                    }
                }
            }
            _ => {}
        }
    }
    result
}

fn cases() -> Vec<String> {
    let start = ORDER.find("\"cases\": [").expect("the cases");
    let end = start + ORDER[start..].find("\n ]").expect("the end of the cases");
    ORDER[start..end]
        .lines()
        .skip(1)
        .map(|line| {
            let line = line.trim().trim_end_matches(',');
            line[1..line.len() - 1].to_string()
        })
        .collect()
}

// `ourOrder`: the code point indices of `line` in visual order, left to
// right.
fn our_order(line: &str, order: &[BidiSpan], dir: Direction) -> Vec<usize> {
    let index = |byte: usize| line[..byte].chars().count();
    let mut spans: Vec<&BidiSpan> = order.iter().collect();
    if dir == Direction::Rtl {
        spans.reverse();
    }
    let mut result = Vec::new();
    for span in spans {
        let (from, to) = (index(span.from), index(span.to));
        if span.level % 2 == 1 {
            result.extend((from..to).rev());
        } else {
            result.extend(from..to);
        }
    }
    result
}

fn join(order: &[usize]) -> String {
    order
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join("-")
}

fn check_context(dir: Direction) {
    let expected = recorded(if dir == Direction::Ltr { "ltr" } else { "rtl" });
    let cases = cases();
    assert_eq!(cases.len(), 20);
    let mut failures = Vec::new();
    for (case, browser) in cases.iter().zip(&expected) {
        let ours = join(&our_order(case, &compute_order(case, dir, &[]), dir));
        if ours != join(browser) {
            failures.push(format!(
                "{case}: {ours} against Chromium's {}",
                join(browser)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{dir:?} context:\n{}",
        failures.join("\n")
    );
}

#[test]
fn orders_ltr_context_as_chromium_draws_it() {
    check_context(Direction::Ltr);
}

#[test]
fn orders_rtl_context_as_chromium_draws_it() {
    check_context(Direction::Rtl);
}

fn check_motion(dir: Direction) {
    for case in cases() {
        let text = Text::of(&[case.as_str()]).unwrap();
        let line = text.line(1);
        let order = compute_order(&case, dir, &[]);
        for forward in [true, false] {
            let mut seen = HashSet::new();
            let mut pos = if forward {
                EditorSelection::cursor_with(0, -1, None, None)
            } else {
                EditorSelection::cursor_with(line.len(), 1, None, None)
            };
            loop {
                let id = pos.head() as isize * if pos.assoc() < 0 { -1 } else { 1 };
                assert!(seen.insert(id), "{case} ({forward}): {id} visited twice");
                let Some(next) = move_visually(&line, &order, dir, &pos, forward) else {
                    assert_eq!(pos.head(), if forward { line.len() } else { 0 }, "{case}");
                    assert_eq!(pos.assoc(), if forward { 1 } else { -1 }, "{case}");
                    break;
                };
                pos = next;
            }
            assert_eq!(seen.len(), case.chars().count() + 1, "{case} ({forward})");
        }
    }
}

#[test]
fn moves_through_every_position_in_ltr_context() {
    check_motion(Direction::Ltr);
}

#[test]
fn moves_through_every_position_in_rtl_context() {
    check_motion(Direction::Rtl);
}

#[test]
fn handles_extending_characters() {
    // Upstream's string, with its combining marks spelled out.
    let s = "ae\u{301}\u{320}o\u{30b}x \u{1f60e}\u{1f649} \
             \u{1f468}\u{200d}\u{1f3a4}\u{1f4aa}\u{1f3fd}\
             \u{1f469}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466} \
             \u{1f1e9}\u{1f1ea}\u{1f1eb}\u{1f1f7}";
    assert_eq!(s.encode_utf16().count(), 42);
    let points = [0, 1, 4, 6, 7, 8, 10, 12, 13, 18, 22, 33, 34, 38, 42];
    let text = Text::of(&[s]).unwrap();
    let line = text.line(1);
    let order = compute_order(s, Direction::Ltr, &[]);
    let cursor = |units: usize| EditorSelection::cursor_with(byte_at(s, units), 0, Some(0), None);
    for pair in points.windows(2) {
        let next = move_visually(&line, &order, Direction::Ltr, &cursor(pair[0]), true).unwrap();
        assert_eq!(units_at(s, next.from()), pair[1]);
        let prev = move_visually(&line, &order, Direction::Ltr, &cursor(pair[1]), false).unwrap();
        assert_eq!(units_at(s, prev.from()), pair[0]);
    }
}

// Upstream's loop condition, `count++`, is 0 when first checked, so its
// body never runs. The port runs the loop the test describes, bounded by
// one step per position of the line and the step that finds no further
// position; upstream's bound, fewer than 6 steps, would fail without a
// loop.
#[test]
fn handles_a_misplaced_non_joiner_without_going_in_a_loop() {
    let doc = "ءAB\u{200c}ء";
    let text = Text::of(&[doc]).unwrap();
    let line = text.line(1);
    let order = compute_order(doc, Direction::Rtl, &[]);
    for (start, forward) in [(0, true), (doc.len(), false)] {
        let mut pos: Option<SelectionRange> = Some(EditorSelection::cursor(start));
        let mut count = 0;
        while let Some(current) = pos {
            count += 1;
            assert!(count <= doc.chars().count() + 1, "no loop from {start}");
            pos = move_visually(&line, &order, Direction::Rtl, &current, forward);
        }
    }
}

fn temp_view(doc: &str, extensions: Extension) -> (EditorView, Document) {
    let document = Document::new(800, 600).unwrap();
    let state = EditorState::create(EditorStateConfig {
        doc: Some(Insertion::Text(Text::of(&[doc]).unwrap())),
        selection: None,
        extensions,
    })
    .unwrap();
    let view = EditorView::new(
        &document,
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
    .unwrap();
    (view, document)
}

fn rtl_theme() -> Extension {
    EditorView::theme(
        &StyleSpec::new().nest("&", StyleSpec::new().set("direction", "rtl")),
        false,
    )
    .unwrap()
}

fn iso(attrs: &[(&str, &str)], bidi_isolate: Option<Direction>) -> std::sync::Arc<Decoration> {
    let mut attributes = Attrs::new();
    for (name, value) in attrs {
        attributes.set(*name, *value);
    }
    Decoration::mark(MarkSpec {
        attributes: Some(attributes),
        bidi_isolate,
        ..MarkSpec::default()
    })
}

// A decoration over UTF-16 range `from..to` of `doc`.
type IsoRange = (
    &'static [(&'static str, &'static str)],
    Option<Direction>,
    usize,
    usize,
);

const RTL_ISO: &[(&str, &str)] = &[("style", "direction: rtl; unicode-bidi: isolate")];
const LTR_ISO: &[(&str, &str)] = &[("style", "direction: ltr; unicode-bidi: isolate")];
const AUTO_ISO: &[(&str, &str)] = &[("dir", "auto")];

fn rtl(from: usize, to: usize) -> IsoRange {
    (RTL_ISO, Some(Direction::Rtl), from, to)
}

fn ltr(from: usize, to: usize) -> IsoRange {
    (LTR_ISO, Some(Direction::Ltr), from, to)
}

fn auto(from: usize, to: usize) -> IsoRange {
    (AUTO_ISO, None, from, to)
}

// `deco`: decorations that are also isolated ranges.
fn deco(doc: &str, ranges: &[IsoRange]) -> Extension {
    let ranges: Vec<Range<Decoration>> = ranges
        .iter()
        .map(|&(attrs, dir, from, to)| {
            iso(attrs, dir)
                .range(byte_at(doc, from), byte_at(doc, to))
                .unwrap()
        })
        .collect();
    let set = RangeSet::of(ranges, false);
    Extension::from(vec![
        EditorView::decorations(set.clone()),
        EditorView::bidi_isolated_ranges(set),
    ])
}

fn test_isolates(doc: &str, extensions: Extension, expected: &str) {
    let (mut view, _document) = temp_view(doc, extensions);
    view.measure().unwrap();
    let line = view.state().doc().line(1);
    let spans = view.bidi_spans(&line);
    let got: Vec<String> = spans
        .iter()
        .map(|s| {
            format!(
                "{}-{}:{}",
                units_at(doc, s.from),
                units_at(doc, s.to),
                s.level
            )
        })
        .collect();
    assert_eq!(got.join(" "), expected);
}

#[test]
fn properly_handles_isolates_in_rtl() {
    let doc = "אחת -hello- שתיים";
    test_isolates(
        doc,
        Extension::from(vec![rtl_theme(), deco(doc, &[ltr(4, 11)])]),
        "0-4:1 4-11:2 11-17:1",
    );
}

#[test]
fn properly_handles_isolates_in_ltr() {
    let doc = "one -שלום- two";
    test_isolates(doc, deco(doc, &[rtl(4, 10)]), "0-4:0 4-10:1 10-14:0");
}

#[test]
fn properly_handles_isolates_in_rtl_text_in_ltr_context() {
    let doc = "אחת -hello- שתיים";
    test_isolates(doc, deco(doc, &[ltr(4, 11)]), "11-17:1 4-11:2 0-4:1");
}

#[test]
fn handles_ltr_isolates_in_nested_numerals() {
    let doc = "كود12ab34المرآة";
    test_isolates(
        doc,
        Extension::from(vec![rtl_theme(), deco(doc, &[ltr(5, 7)])]),
        "0-3:1 3-5:2 5-7:2 7-9:2 9-15:1",
    );
}

#[test]
fn handles_rtl_isolates_in_nested_numerals() {
    let doc = "كود12مر34المرآة";
    test_isolates(
        doc,
        Extension::from(vec![rtl_theme(), deco(doc, &[rtl(5, 7)])]),
        "0-3:1 3-5:2 5-7:1 7-9:2 9-15:1",
    );
}

#[test]
fn works_for_multiple_isolates() {
    let doc = "one -שלום- two .אחת. three";
    test_isolates(
        doc,
        deco(doc, &[rtl(4, 10), rtl(15, 20)]),
        "0-4:0 4-10:1 10-15:0 15-20:1 20-26:0",
    );
}

#[test]
fn handles_multiple_isolates_in_a_row() {
    let doc = "one -שלום- two";
    test_isolates(
        doc,
        deco(doc, &[rtl(4, 7), rtl(7, 10)]),
        "0-4:0 4-7:1 7-10:1 10-14:0",
    );
}

#[test]
fn supports_nested_isolates() {
    let doc = "one -אחת .two. שתיים- three";
    test_isolates(
        doc,
        Extension::from(vec![deco(doc, &[ltr(9, 14)]), deco(doc, &[rtl(4, 21)])]),
        "0-4:0 14-21:1 9-14:2 4-9:1 21-27:0",
    );
}

#[test]
fn includes_isolates_at_the_end_of_spans_in_the_base_direction() {
    let doc = "oneאחת -";
    test_isolates(
        doc,
        deco(doc, &[ltr(6, 7), ltr(7, 8)]),
        "0-3:0 3-6:1 6-7:0 7-8:0",
    );
}

#[test]
fn normalizes_neutrals_between_isolates() {
    let doc = "שלa-bום";
    test_isolates(
        doc,
        deco(doc, &[ltr(2, 3), ltr(4, 5)]),
        "5-7:1 4-5:2 3-4:1 2-3:2 0-2:1",
    );
}

#[test]
fn matches_brackets_across_isolates() {
    let doc = "one(אחת)שתיים";
    test_isolates(
        doc,
        deco(doc, &[rtl(4, 5)]),
        "0-4:0 4-5:1 5-7:1 7-8:0 8-13:1",
    );
}

#[test]
fn resolves_auto_isolates_as_a_whole() {
    let doc = " abc,אחת.";
    test_isolates(
        doc,
        Extension::from(vec![deco(doc, &[ltr(1, 4)]), deco(doc, &[auto(0, 9)])]),
        "4-9:1 1-4:2 0-1:1",
    );
}

#[test]
fn reads_the_direction_per_line_when_asked() {
    let doc = "one";
    let (view, _document) = temp_view(doc, rtl_theme());
    assert_eq!(view.text_direction(), Direction::Rtl);
    assert_eq!(view.text_direction_at(0), Direction::Rtl);
    let (view, _document) = temp_view(
        doc,
        Extension::from(vec![EditorView::per_line_text_direction(true)]),
    );
    assert_eq!(view.text_direction_at(0), Direction::Ltr);
}

#[test]
fn orders_a_changed_line_again() {
    let doc = "one -שלום- two";
    let (mut view, _document) = temp_view(doc, Extension::from(vec![]));
    let first = view.bidi_spans(&view.state().doc().line(1));
    assert_eq!(first.len(), 3);
    let tr = view
        .state()
        .update(editor_state::TransactionSpec {
            changes: Some(editor_state::ChangeSpec::insert(0, "x")),
            ..Default::default()
        })
        .unwrap();
    view.dispatch(tr).unwrap();
    let line = view.state().doc().line(1);
    let spans = view.bidi_spans(&line);
    assert_eq!(
        spans,
        compute_order(line.text, Direction::Ltr, &[]),
        "a changed line is ordered again"
    );
}
