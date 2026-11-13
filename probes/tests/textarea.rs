//! Probe: a `<textarea>` holding a large document.
//!
//! Measures mount, layout, paint and per-keystroke cost with the built-in
//! editing model, and records which editor behaviours the control has.
//!
//! `cargo test -p editor-probes --test textarea -- --nocapture`
//! (`PROBE_SCALE=full` for 10,000 and 100,000 lines).

use div::prelude::*;
use div::testing::TestApp;
use editor_probes::{ms, size, source_text, timed};

struct Probe {
    app: TestApp,
    area: Element,
}

fn mount(lines: usize) -> (Probe, std::time::Duration) {
    let text = source_text(lines);
    let host = create_signal(None::<Element>);
    let ((app, area), mount) = timed(|| {
        let app = TestApp::new(LogicalSize::new(800.0, 600.0)).mount(move || {
            let document = current_document();
            let area = Element::create(&document, "textarea").unwrap();
            area.apply_style(&style! {
                width: 780px; height: 580px; font-family: monospace; font-size: 13px;
                white-space: pre
            })
            .unwrap();
            host.set(Some(area.clone()));
            ViewNode::Element(area)
        });
        let area = host.get().unwrap();
        area.set_control_value(&text).unwrap();
        app.document().update_all().unwrap();
        (app, area)
    });
    (Probe { app, area }, mount)
}

fn layout_and_render(probe: &mut Probe) -> (std::time::Duration, std::time::Duration) {
    let (_, layout) = timed(|| probe.app.document().update_all().unwrap());
    let (_, paint) = timed(|| probe.app.headless().render_at(0.0).unwrap());
    (layout, paint)
}

fn measure(lines: usize) {
    let (mut probe, mount) = mount(lines);
    let (_, first_paint) = timed(|| probe.app.headless().render_at(0.0).unwrap());
    let len = probe.area.control_value().unwrap().unwrap().len();
    probe.area.focus().unwrap();
    probe.area.set_selection(len / 2, len / 2).unwrap();

    let (_, key) = timed(|| probe.app.type_text("x"));
    let (key_layout, key_paint) = layout_and_render(&mut probe);
    let (_, enter) = timed(|| probe.app.press(Key::Enter));
    let (_, backspace) = timed(|| probe.app.press(Key::Backspace));
    let (_, undo) = timed(|| probe.app.press_with(Modifiers::CTRL, Key::Z));
    let (_, arrow) = timed(|| probe.app.press(Key::ArrowRight));

    println!(
        "textarea {lines:>6} lines ({len} bytes): mount+layout {}, first paint {}, \
         key event {}, then layout {} + paint {}, Enter {}, Backspace {}, Ctrl+Z {}, ArrowRight {}",
        ms(mount),
        ms(first_paint),
        ms(key),
        ms(key_layout),
        ms(key_paint),
        ms(enter),
        ms(backspace),
        ms(undo),
        ms(arrow),
    );
}

#[test]
fn large_textarea_cost() {
    for lines in [size(500, 1_000), size(1_000, 10_000), size(2_000, 100_000)] {
        measure(lines);
    }
}

/// What the built-in control does with the keys and geometry an editor
/// needs. Each assertion records today's behaviour; a change in Div that
/// fixes a gap makes the assertion fail, which is the signal to update
/// `docs/design.md`.
#[test]
fn textarea_editor_behaviour() {
    let (mut probe, _) = mount(200);
    let area = probe.area.clone();
    area.focus().unwrap();

    // Typing at a caret in the middle inserts there.
    area.set_selection(0, 0).unwrap();
    probe.app.type_text("ab");
    assert_eq!(area.selection().unwrap(), Some((2, 2)));
    assert!(area
        .control_value()
        .unwrap()
        .unwrap()
        .starts_with("abfn item_0"));

    // div-gap: 21. ArrowDown does not move the caret to the next line.
    probe.app.press(Key::ArrowDown);
    let after_down = area.selection().unwrap();
    println!("selection after ArrowDown from (2, 2): {after_down:?}");
    assert_eq!(after_down, Some((2, 2)));

    // Home and End go to the logical line's ends.
    probe.app.press(Key::End);
    let first_line = editor_probes::source_line(0).len() + 2;
    assert_eq!(area.selection().unwrap(), Some((first_line, first_line)));

    // Tab does not insert a tab character (as in Chromium, where Tab moves
    // focus); an editor must prevent keydown's default to keep Tab.
    probe.app.press(Key::Tab);
    let value = area.control_value().unwrap().unwrap();
    println!(
        "after Tab: focus still in textarea = {}, value contains a tab = {}",
        area.has_focus().unwrap(),
        value.contains('\t')
    );
    assert!(!value.contains('\t'));

    // div-gap: 21. Undo restores the value but puts the caret at the end.
    area.focus().unwrap();
    probe.app.press_with(Modifiers::CTRL, Key::Z);
    let after_undo = area.selection().unwrap();
    println!("selection after Ctrl+Z: {after_undo:?}");
    let end = area.control_value().unwrap().unwrap().len();
    assert_eq!(after_undo, Some((end, end)));

    // Scrolling: a 200-line textarea in a 580px box.
    // div-gap: 21. The textarea's content does not overflow into a scroll range.
    let metrics = area.scroll_metrics().unwrap().unwrap();
    println!("textarea scroll metrics with 200 lines: {metrics:?}");
    assert_eq!(metrics.scroll_height, metrics.client_height);
    area.scroll_to(0.0, 400.0).unwrap();
    probe.app.document().update_all().unwrap();
    let top = area.scroll_top().unwrap();
    println!("textarea scroll_top after scroll_to(0, 400): {top}");
    assert_eq!(top, 0.0);
}
