//! Probe: one element per line and one span per token, as CodeMirror and
//! Monaco build their DOM, with Div's current engine.
//!
//! Variants:
//! - `all`: every line of the document is an element in a block scroller.
//! - `window`: only the lines in view (plus a margin) exist, positioned
//!   absolutely in a tall spacer; scrolling re-renders the window.
//! - `flex3`: `all` inside three nested flex columns, as an app shell would
//!   place the editor (div-gap: 16, Div's layout cost).
//!
//! `cargo test -p editor-probes --test line_elements -- --nocapture`
//! (`PROBE_SCALE=full` for 2,000 and 10,000 lines).

use div::prelude::*;
use div::testing::TestApp;
use editor_probes::{ms, size, source_line, timed, tokens};
use std::time::Duration;

const LINE_HEIGHT: f32 = 18.0;
const VIEW_HEIGHT: f32 = 600.0;

fn token_color(class: &str) -> Color {
    match class {
        "kw" => Color::from_rgba8(175, 0, 219, 255),
        "ty" => Color::from_rgba8(38, 127, 153, 255),
        "num" => Color::from_rgba8(9, 134, 88, 255),
        "com" => Color::from_rgba8(0, 128, 0, 255),
        _ => Color::from_rgba8(30, 30, 30, 255),
    }
}

/// A line element: a gutter span with the line number and a span per token.
fn line_element(document: &Document, n: usize) -> Element {
    let line = Element::create(document, "div").unwrap();
    line.apply_style(&style! { height: 18px; white-space: pre })
        .unwrap();
    let gutter = Element::create(document, "span").unwrap();
    gutter
        .apply_style(&style! { display: inline-block; width: 48px; color: #888888 })
        .unwrap();
    gutter.set_text(&(n + 1).to_string()).unwrap();
    line.append_child(&gutter).unwrap();
    for (token, class) in tokens(&source_line(n)) {
        let span = Element::create(document, "span").unwrap();
        span.set_color_value(token_color(class).into()).unwrap();
        span.set_text(&token).unwrap();
        line.append_child(&span).unwrap();
    }
    line
}

fn shell(depth: usize, scroller_style: Style) -> (TestApp, Element) {
    let host = create_signal(None::<Element>);
    let app = TestApp::new(LogicalSize::new(800.0, VIEW_HEIGHT as f64)).mount(move || {
        let document = current_document();
        let outer = Element::create(&document, "div").unwrap();
        outer
            .apply_style(&style! {
                display: flex; flex-direction: column; height: 600px;
                font-family: monospace; font-size: 13px; line-height: 18px
            })
            .unwrap();
        let mut parent = outer.clone();
        for _ in 0..depth {
            let level = Element::create(&document, "div").unwrap();
            level
                .apply_style(&style! {
                    display: flex; flex-direction: column; flex-grow: 1; min-height: 0
                })
                .unwrap();
            parent.append_child(&level).unwrap();
            parent = level;
        }
        let scroller = Element::create(&document, "div").unwrap();
        scroller.apply_style(&scroller_style).unwrap();
        parent.append_child(&scroller).unwrap();
        host.set(Some(scroller));
        ViewNode::Element(outer)
    });
    (app, host.get().unwrap())
}

fn scroller_style() -> Style {
    style! { overflow: auto; flex-grow: 1; min-height: 0; position: relative }
}

struct Timings {
    build: Duration,
    layout: Duration,
    paint: Duration,
    edit: Duration,
    insert_top: Duration,
    scroll: Duration,
    hit_test: Duration,
}

fn update_and_paint(app: &mut TestApp) -> Duration {
    let (_, t) = timed(|| {
        app.document().update_all().unwrap();
        app.headless().render_at(0.0).unwrap();
    });
    t
}

fn all_lines(lines: usize, depth: usize) -> Timings {
    let (mut app, scroller) = shell(depth, scroller_style());
    let document = app.document().clone();
    let (_, build) = timed(|| {
        for n in 0..lines {
            scroller.append_child(&line_element(&document, n)).unwrap();
        }
    });
    let (_, layout) = timed(|| document.update_all().unwrap());
    let (_, paint) = timed(|| app.headless().render_at(0.0).unwrap());

    // Edit one line in the middle: replace its token spans.
    let mut target = scroller.first_child().unwrap().unwrap();
    for _ in 0..lines / 2 {
        target = target.next_sibling().unwrap().unwrap();
    }
    let (_, edit) = timed(|| {
        let replacement = line_element(&document, lines / 2 + 1);
        scroller.insert_before(&replacement, &target).unwrap();
        target.remove().unwrap();
    });
    let edit = edit + update_and_paint(&mut app);

    // Insert a line at the top (every later line moves).
    let first = scroller.first_child().unwrap().unwrap();
    let (_, insert_top) = timed(|| {
        scroller
            .insert_before(&line_element(&document, 0), &first)
            .unwrap()
    });
    let insert_top = insert_top + update_and_paint(&mut app);

    let (_, scroll) = timed(|| {
        scroller
            .scroll_to(0.0, (lines as f64 * 18.0) / 2.0)
            .unwrap()
    });
    let scroll = scroll + update_and_paint(&mut app);

    let (_, hit_test) = timed(|| document.hit_test(200.0, 300.0).unwrap());
    Timings {
        build,
        layout,
        paint,
        edit,
        insert_top,
        scroll,
        hit_test,
    }
}

/// Only the visible lines exist; the window is rebuilt on scroll.
fn windowed(lines: usize) -> Timings {
    let (mut app, scroller) = shell(0, scroller_style());
    let document = app.document().clone();
    let spacer = Element::create(&document, "div").unwrap();
    let height = lines as f32 * LINE_HEIGHT;
    spacer
        .set_property(
            StyleProperty::Height,
            StyleValue::Length(LengthValue::px(height)),
        )
        .unwrap();
    scroller.append_child(&spacer).unwrap();
    let visible = (VIEW_HEIGHT / LINE_HEIGHT) as usize + 10;

    let render_window = |first: usize| {
        spacer.remove_all_children().unwrap();
        for n in first..(first + visible).min(lines) {
            let line = line_element(&document, n);
            line.apply_style(&style! { position: absolute; left: 0px; right: 0px })
                .unwrap();
            line.set_property(
                StyleProperty::Top,
                StyleValue::Length(LengthValue::px(n as f32 * LINE_HEIGHT)),
            )
            .unwrap();
            spacer.append_child(&line).unwrap();
        }
    };

    let (_, build) = timed(|| render_window(0));
    let (_, layout) = timed(|| document.update_all().unwrap());
    let (_, paint) = timed(|| app.headless().render_at(0.0).unwrap());

    let (_, edit) = timed(|| {
        let line = spacer
            .first_child()
            .unwrap()
            .unwrap()
            .next_sibling()
            .unwrap()
            .unwrap();
        let replacement = line_element(&document, 7);
        replacement
            .apply_style(&style! { position: absolute; left: 0px; right: 0px; top: 18px })
            .unwrap();
        spacer.insert_before(&replacement, &line).unwrap();
        line.remove().unwrap();
    });
    let edit = edit + update_and_paint(&mut app);
    let (_, insert_top) = timed(|| render_window(0));
    let insert_top = insert_top + update_and_paint(&mut app);
    let middle = lines / 2;
    let (_, scroll) = timed(|| {
        scroller
            .scroll_to(0.0, (middle as f32 * LINE_HEIGHT) as f64)
            .unwrap();
        render_window(middle);
    });
    let scroll = scroll + update_and_paint(&mut app);
    let (_, hit_test) = timed(|| document.hit_test(200.0, 300.0).unwrap());
    Timings {
        build,
        layout,
        paint,
        edit,
        insert_top,
        scroll,
        hit_test,
    }
}

fn report(name: &str, lines: usize, t: Timings) {
    println!(
        "{name:<7} {lines:>6} lines: build {}, layout {}, paint {}, edit one line {}, \
         insert at top {}, scroll to middle {}, hit test {}",
        ms(t.build),
        ms(t.layout),
        ms(t.paint),
        ms(t.edit),
        ms(t.insert_top),
        ms(t.scroll),
        ms(t.hit_test),
    );
}

#[test]
fn line_element_cost() {
    // Every line as an element: 2,000 lines took 31.6 s per layout at
    // 565db46a, so the full run stops there.
    for lines in [size(100, 500), size(200, 2_000)] {
        report("all", lines, all_lines(lines, 0));
    }
    // Only the visible lines exist, so the document size does not matter.
    for lines in [size(1_000, 10_000), size(1_000, 100_000)] {
        report("window", lines, windowed(lines));
    }
    let lines = size(50, 200);
    report("all", lines, all_lines(lines, 0));
    report("flex3", lines, all_lines(lines, 3));
}

/// Caret geometry from spans: with no text-offset API, an app can measure
/// a monospace cell from a span's box. Records what the box API returns.
#[test]
fn caret_geometry_from_spans() {
    let (mut app, scroller) = shell(0, scroller_style());
    let document = app.document().clone();
    let line = line_element(&document, 1);
    scroller.append_child(&line).unwrap();
    document.update_all().unwrap();
    app.run_until_idle();

    let mut span = line.first_child().unwrap();
    let mut cells = Vec::new();
    while let Some(s) = span {
        let text = s.text_content().unwrap();
        let rect = s.bounding_rect().unwrap().unwrap();
        cells.push((text.chars().count(), rect.width));
        span = s.next_sibling().unwrap();
    }
    // Skip the inline-block gutter; every token span should be n * cell.
    let (n, w) = cells[1];
    let cell = w / n as f32;
    println!("monospace cell width from a {n}-char span: {cell} px");
    for (n, w) in &cells[1..] {
        assert!(
            (w - cell * *n as f32).abs() < 0.01,
            "span of {n} chars is {w}px, cell {cell}px"
        );
    }

    // Point to element works; point to text offset has no API.
    let rect = line.bounding_rect().unwrap().unwrap();
    let hit = document
        .hit_test(rect.x + 48.0 + cell * 6.5, rect.y + 9.0)
        .unwrap()
        .unwrap();
    println!(
        "hit test 6.5 cells into the code: <{:?}> {:?}",
        hit.kind().unwrap(),
        hit.text_content().unwrap()
    );
}

/// How a windowed line is built.
#[derive(Clone, Copy, Debug)]
enum LineContent {
    /// One text node: no highlighting.
    Plain,
    /// Adjacent tokens of the same colour merged into one span.
    Grouped,
    /// One span per token, as `line_element` builds.
    Tokens,
}

fn flow_line(document: &Document, n: usize, content: LineContent) -> Element {
    let line = Element::create(document, "div").unwrap();
    line.apply_style(&style! { height: 18px; white-space: pre })
        .unwrap();
    match content {
        LineContent::Plain => line.set_text(&source_line(n)).unwrap(),
        LineContent::Grouped | LineContent::Tokens => {
            let mut groups: Vec<(String, &str)> = Vec::new();
            for (token, class) in tokens(&source_line(n)) {
                let class = if class == "ws" || class == "punct" {
                    "id"
                } else {
                    class
                };
                match groups.last_mut() {
                    Some((text, last))
                        if matches!(content, LineContent::Grouped) && *last == class =>
                    {
                        text.push_str(&token)
                    }
                    _ => groups.push((token, class)),
                }
            }
            for (text, class) in groups {
                let span = Element::create(document, "span").unwrap();
                span.set_color_value(token_color(class).into()).unwrap();
                span.set_text(&text).unwrap();
                line.append_child(&span).unwrap();
            }
        }
    }
    line
}

/// The configuration a prototype can use today: block containers only, the
/// visible lines in normal flow between two spacers, rebuilt on scroll.
fn window_flow(lines: usize, content: LineContent, margin_px: f32) {
    let host = create_signal(None::<Element>);
    let mut app = TestApp::new(LogicalSize::new(800.0, VIEW_HEIGHT as f64)).mount(move || {
        let document = current_document();
        let scroller = Element::create(&document, "div").unwrap();
        scroller
            .apply_style(&style! {
                height: 600px; overflow: auto;
                font-family: monospace; font-size: 13px; line-height: 18px
            })
            .unwrap();
        host.set(Some(scroller.clone()));
        ViewNode::Element(scroller)
    });
    let scroller = host.get().unwrap();
    let document = app.document().clone();
    let visible = ((VIEW_HEIGHT + 2.0 * margin_px) / LINE_HEIGHT) as usize + 10;
    let render_window = |first: usize| {
        scroller.remove_all_children().unwrap();
        let last = (first + visible).min(lines);
        for (n, height) in [(0, first), (1, lines - last)] {
            let spacer = Element::create(&document, "div").unwrap();
            spacer
                .set_property(
                    StyleProperty::Height,
                    StyleValue::Length(LengthValue::px(height as f32 * LINE_HEIGHT)),
                )
                .unwrap();
            if n == 0 {
                scroller.append_child(&spacer).unwrap();
                for i in first..last {
                    scroller
                        .append_child(&flow_line(&document, i, content))
                        .unwrap();
                }
            } else {
                scroller.append_child(&spacer).unwrap();
            }
        }
    };
    let (_, build) = timed(|| render_window(0));
    let (_, layout) = timed(|| document.update_all().unwrap());
    let (_, paint) = timed(|| app.headless().render_at(0.0).unwrap());
    // A keystroke: replace one visible line's content.
    let (_, edit) = timed(|| {
        let spacer = scroller.first_child().unwrap().unwrap();
        let line = spacer
            .next_sibling()
            .unwrap()
            .unwrap()
            .next_sibling()
            .unwrap()
            .unwrap();
        let replacement = flow_line(&document, 6, content);
        scroller.insert_before(&replacement, &line).unwrap();
        line.remove().unwrap();
    });
    let edit = edit + update_and_paint(&mut app);
    let middle = lines / 2;
    let (_, scroll) = timed(|| {
        render_window(middle);
        scroller
            .scroll_to(0.0, (middle as f32 * LINE_HEIGHT) as f64)
            .unwrap();
    });
    let scroll = scroll + update_and_paint(&mut app);
    let spans: usize = (0..visible)
        .map(|n| match content {
            LineContent::Plain => 1,
            _ => {
                let line = flow_line(&document, n, content);
                let mut count = 0;
                let mut child = line.first_child().unwrap();
                while let Some(c) = child {
                    count += 1;
                    child = c.next_sibling().unwrap();
                }
                count
            }
        })
        .sum();
    println!(
        "flow window {lines:>6} lines, margin {margin_px}px, {content:?} ({spans} inline boxes): build {}, \
         layout {}, paint {}, edit one line + layout + paint {}, scroll to middle {}",
        ms(build),
        ms(layout),
        ms(paint),
        ms(edit),
        ms(scroll),
    );
}

#[test]
fn window_flow_cost() {
    let lines = size(1_000, 100_000);
    for content in [
        LineContent::Plain,
        LineContent::Grouped,
        LineContent::Tokens,
    ] {
        window_flow(lines, content, 0.0);
    }
    // CodeMirror 6 renders the visible lines plus a margin of about 1000px
    // above and below (its viewport margin), each line a span per token.
    window_flow(lines, LineContent::Tokens, 1000.0);
}
