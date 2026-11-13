//! Probe: where the layout time of a code view goes.
//!
//! `line_elements` found a few dozen highlighted lines taking about a second
//! to lay out. This isolates the variables: lines, spans per line, the
//! container (block or flex column), `white-space`, and the gutter's
//! `inline-block`. Each case lays out a fresh document once.
//!
//! `cargo test -p editor-probes --test inline_cost -- --nocapture --test-threads=1`

use div::prelude::*;
use div::testing::TestApp;
use editor_probes::{ms, size, timed};
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
struct Case {
    lines: usize,
    spans: usize,
    flex: bool,
    pre: bool,
    gutter: bool,
}

fn layout_time(case: Case) -> Duration {
    let host = create_signal(None::<Element>);
    let mut app = TestApp::new(LogicalSize::new(800.0, 600.0)).mount(move || {
        let document = current_document();
        let outer = Element::create(&document, "div").unwrap();
        if case.flex {
            outer
                .apply_style(&style! { display: flex; flex-direction: column; height: 600px })
                .unwrap();
        } else {
            outer.apply_style(&style! { height: 600px }).unwrap();
        }
        outer
            .apply_style(&style! { font-family: monospace; font-size: 13px; line-height: 18px })
            .unwrap();
        let scroller = Element::create(&document, "div").unwrap();
        scroller
            .apply_style(&style! { overflow: auto; height: 600px })
            .unwrap();
        outer.append_child(&scroller).unwrap();
        host.set(Some(scroller));
        ViewNode::Element(outer)
    });
    let scroller = host.get().unwrap();
    let document = app.document().clone();
    for n in 0..case.lines {
        let line = Element::create(&document, "div").unwrap();
        if case.pre {
            line.apply_style(&style! { height: 18px; white-space: pre })
                .unwrap();
        } else {
            line.apply_style(&style! { height: 18px }).unwrap();
        }
        if case.gutter {
            let gutter = Element::create(&document, "span").unwrap();
            gutter
                .apply_style(&style! { display: inline-block; width: 48px })
                .unwrap();
            gutter.set_text(&n.to_string()).unwrap();
            line.append_child(&gutter).unwrap();
        }
        for s in 0..case.spans {
            let span = Element::create(&document, "span").unwrap();
            span.set_text(if s % 2 == 0 { "token" } else { " " })
                .unwrap();
            line.append_child(&span).unwrap();
        }
        scroller.append_child(&line).unwrap();
    }
    let (_, t) = timed(|| document.update_all().unwrap());
    app.run_until_idle();
    t
}

#[test]
fn layout_by_spans_per_line() {
    let lines = size(10, 40);
    let max_spans = size(8, 32);
    let mut spans = 1;
    while spans <= max_spans {
        let base = Case {
            lines,
            spans,
            flex: false,
            pre: true,
            gutter: false,
        };
        println!(
            "{lines} lines x {spans:>2} spans: block {}, flex column {}, block+gutter {}, \
             block without pre {}",
            ms(layout_time(base)),
            ms(layout_time(Case { flex: true, ..base })),
            ms(layout_time(Case {
                gutter: true,
                ..base
            })),
            ms(layout_time(Case { pre: false, ..base })),
        );
        spans *= 2;
    }
}

#[test]
fn layout_by_line_count() {
    let max_lines = size(40, 640);
    let mut lines = 10;
    while lines <= max_lines {
        let base = Case {
            lines,
            spans: 8,
            flex: false,
            pre: true,
            gutter: false,
        };
        println!(
            "{lines:>4} lines x 8 spans: block {}, flex column {}",
            ms(layout_time(base)),
            ms(layout_time(Case { flex: true, ..base })),
        );
        lines *= 4;
    }
}
