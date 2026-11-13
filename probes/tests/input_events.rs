//! Probe: which keyboard, text, IME, mouse, clipboard and accessibility
//! signals reach app code, for (a) a custom editor surface (a focusable
//! `div`) and (b) the hidden-textarea pattern CodeMirror 5 and Monaco use.
//!
//! Each assertion records today's behaviour at the pinned Div commit. When
//! Div closes a gap, the assertion fails, which is the signal to update
//! `docs/design.md`.
//!
//! `cargo test -p editor-probes --test input_events -- --nocapture`

use div::prelude::*;
use div::testing::TestApp;
use std::cell::RefCell;
use std::rc::Rc;

type Log = Rc<RefCell<Vec<String>>>;

const EVENTS: &[&str] = &[
    "keydown",
    "keyup",
    "beforeinput",
    "input",
    "compositionstart",
    "compositionupdate",
    "compositionend",
    "focus",
    "blur",
    "mousedown",
    "mouseup",
    "click",
    "dblclick",
    "pointerdown",
    "pointermove",
    "contextmenu",
];

fn record(element: &Element, log: &Log) {
    for name in EVENTS {
        let log = log.clone();
        element
            .on(name, move |event: &Event| {
                let mut entry = event.event_type.clone();
                if let Some(key) = event.key() {
                    entry.push_str(&format!(" key={key:?}"));
                }
                if event.repeat {
                    entry.push_str(" repeat");
                }
                if !event.key_text.is_empty() && event.key().is_none() {
                    entry.push_str(&format!(" text={:?}", event.key_text));
                }
                if let Some(info) = event.input_info() {
                    entry.push_str(&format!(" inputType={}", info.input_type().as_str()));
                }
                if event.event_type.starts_with("mouse") || event.event_type == "click" {
                    entry.push_str(&format!(
                        " button={} at=({},{})",
                        event.mouse_button, event.mouse_x, event.mouse_y
                    ));
                }
                log.borrow_mut().push(entry);
            })
            .unwrap();
    }
}

/// A focusable `div` editor surface with a child span, and a textarea.
fn mount() -> (TestApp, Element, Element, Element) {
    let ids = create_signal(None::<(Element, Element, Element)>);
    let app = TestApp::new(LogicalSize::new(400.0, 300.0)).mount(move || {
        let document = current_document();
        let root = Element::create(&document, "div").unwrap();
        let surface = Element::create(&document, "div").unwrap();
        surface.set_attribute("tabindex", "0").unwrap();
        surface
            .apply_style(&style! { width: 300px; height: 100px; font-family: monospace })
            .unwrap();
        let span = Element::create(&document, "span").unwrap();
        span.set_text("let value = 1;").unwrap();
        surface.append_child(&span).unwrap();
        let area = Element::create(&document, "textarea").unwrap();
        area.apply_style(&style! { width: 300px; height: 60px })
            .unwrap();
        root.append_child(&surface).unwrap();
        root.append_child(&area).unwrap();
        ids.set(Some((surface, span, area)));
        ViewNode::Element(root)
    });
    let (surface, span, area) = ids.get().unwrap();
    (app, surface, span, area)
}

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

#[test]
fn custom_surface_keyboard() {
    let (mut app, surface, _, _) = mount();
    let log: Log = Rc::default();
    record(&surface, &log);
    surface.focus().unwrap();
    take(&log);

    app.type_text("a");
    let typed = take(&log);
    println!("surface, type 'a': {typed:?}");
    // div-gap: 2. No beforeinput/input on a non-control element (no editing host).
    assert!(typed.iter().any(|e| e.starts_with("keydown key=Char('a')")));
    assert!(!typed
        .iter()
        .any(|e| e.starts_with("beforeinput") || e.starts_with("input")));

    app.press_with(Modifiers::CTRL | Modifiers::SHIFT, Key::ArrowLeft);
    println!("surface, Ctrl+Shift+ArrowLeft: {:?}", take(&log));

    app.document()
        .dispatch_key(KeyEventType::Down, Key::ArrowDown, Modifiers::NONE, true)
        .unwrap();
    let repeated = take(&log);
    println!("surface, repeated ArrowDown: {repeated:?}");
    assert!(repeated.iter().any(|e| e == "keydown key=ArrowDown repeat"));

    // Tab: the app keeps focus by preventing keydown's default.
    surface
        .on("keydown", |event: &Event| {
            if event.key() == Some(Key::Tab) {
                event.prevent_default();
            }
        })
        .unwrap();
    app.press(Key::Tab);
    assert!(surface.has_focus().unwrap(), "prevented Tab keeps focus");
    take(&log);
}

#[test]
fn custom_surface_ime() {
    let (app, surface, _, _) = mount();
    let log: Log = Rc::default();
    record(&surface, &log);
    surface.focus().unwrap();
    take(&log);
    let document = app.document().clone();
    document.dispatch_composition_start().unwrap();
    document.dispatch_composition_update("ni").unwrap();
    document.dispatch_composition_end("你").unwrap();
    document.dispatch_text_input("x").unwrap();
    let events = take(&log);
    println!("surface, IME start/update/end + text input: {events:?}");
    // div-gap: 7. Composition and text input never reach a custom surface.
    assert!(events.is_empty());
}

#[test]
fn hidden_textarea_ime() {
    let (app, _, _, area) = mount();
    let log: Log = Rc::default();
    record(&area, &log);
    area.focus().unwrap();
    take(&log);
    let document = app.document().clone();
    document.dispatch_composition_start().unwrap();
    document.dispatch_composition_update("ni").unwrap();
    let preedit_value = area.control_value().unwrap();
    document.dispatch_composition_end("你").unwrap();
    let events = take(&log);
    println!("textarea, IME: {events:?}; value during preedit {preedit_value:?}");
    assert!(events.iter().any(|e| e == "compositionupdate text=\"ni\""));
    assert!(events.iter().any(|e| e == "compositionend text=\"你\""));
    // div-gap: 3. The input event after a commit has no inputType.
    let commit_input = events.iter().find(|e| e.starts_with("input")).unwrap();
    println!("input after commit: {commit_input}");
    assert_eq!(area.control_value().unwrap().as_deref(), Some("你"));

    // The pattern: read what the control received, then clear it.
    area.set_control_value("").unwrap();
    document.dispatch_text_input("x").unwrap();
    let events = take(&log);
    println!("textarea, text input: {events:?}");
    assert!(events
        .iter()
        .any(|e| e == "input text=\"x\" inputType=insertText"));
}

#[test]
fn custom_surface_mouse() {
    let (mut app, surface, span, _) = mount();
    let log: Log = Rc::default();
    record(&surface, &log);
    let rect = span.bounding_rect().unwrap().unwrap();
    let (x, y) = (rect.x + 20.0, rect.y + rect.height / 2.0);
    let document = app.document().clone();
    for kind in [MouseEventType::Down, MouseEventType::Up] {
        document
            .dispatch_mouse_event(kind, x, y, MouseButton::Left, Modifiers::NONE)
            .unwrap();
    }
    app.run_until_idle();
    let first = take(&log);
    for kind in [MouseEventType::Down, MouseEventType::Up] {
        document
            .dispatch_mouse_event(kind, x, y, MouseButton::Left, Modifiers::NONE)
            .unwrap();
    }
    app.run_until_idle();
    let second = take(&log);
    println!("surface, click on child span: {first:?}");
    println!("surface, second click at the same point: {second:?}");
    println!(
        "focus after clicking the child span of a tabindex div: {}",
        surface.has_focus().unwrap()
    );
    // div-gap: 15. No dblclick and no click count; no pointer* from headless mouse.
    assert!(!second.iter().any(|e| e.starts_with("dblclick")));

    for kind in [MouseEventType::Down, MouseEventType::Up] {
        document
            .dispatch_mouse_event(kind, x, y, MouseButton::Right, Modifiers::NONE)
            .unwrap();
    }
    let right = take(&log);
    println!("surface, right click: {right:?}");
    // div-gap: 15. A right-button press fires `click` and no `contextmenu`
    // (Chromium fires `auxclick` and `contextmenu`, no `click`).
    assert!(right.iter().any(|e| e.starts_with("click")));
    assert!(!right.iter().any(|e| e.starts_with("contextmenu")));
    // div-gap: 15. Pressing on a child does not focus the focusable ancestor.
    assert!(!surface.has_focus().unwrap());
}

#[test]
fn clipboard_is_text() {
    let (app, _, _, _) = mount();
    let document = app.document();
    document.set_clipboard_text("line one\nline two").unwrap();
    assert_eq!(document.clipboard_text().unwrap(), "line one\nline two");
}

#[test]
fn textarea_accessibility_text() {
    let (mut app, _, _, area) = mount();
    area.set_control_value("first line\nsecond").unwrap();
    area.focus().unwrap();
    area.set_selection(3, 3).unwrap();
    app.run_until_idle();
    let update = app.document().accessibility_update().unwrap();
    let mut saw_run = false;
    for (id, node) in &update.nodes {
        let role = node.role();
        if role == AccessibilityRole::MultilineTextInput {
            println!(
                "textarea node {id:?}: value {:?}, text_selection on control {:?}",
                node.value(),
                node.text_selection().is_some()
            );
            // div-gap: 17. AccessKit reads the selection from the control.
            assert!(node.text_selection().is_none());
        }
        if role == AccessibilityRole::TextRun {
            saw_run = true;
            println!(
                "text run {id:?}: {} character lengths, positions {}, widths {}, word starts {}",
                node.character_lengths().len(),
                node.character_positions().is_some(),
                node.character_widths().is_some(),
                !node.word_starts().is_empty(),
            );
            assert!(node.character_positions().is_none());
        }
    }
    assert!(saw_run, "the textarea exposes a text run");
}

/// `contenteditable`, the editing host CodeMirror 6, ProseMirror and Lexical
/// are built on: today the attribute does not make an element editable.
#[test]
fn contenteditable_host() {
    let ids = create_signal(None::<Element>);
    let mut app = TestApp::new(LogicalSize::new(400.0, 200.0)).mount(move || {
        let document = current_document();
        let host = Element::create(&document, "div").unwrap();
        host.set_attribute("contenteditable", "true").unwrap();
        host.apply_style(&style! { width: 300px; height: 100px; white-space: pre-wrap })
            .unwrap();
        let line = Element::create(&document, "div").unwrap();
        line.set_text("hello").unwrap();
        host.append_child(&line).unwrap();
        ids.set(Some(host.clone()));
        ViewNode::Element(host)
    });
    let host = ids.get().unwrap();
    let log: Log = Rc::default();
    record(&host, &log);
    // div-gap: 2. Chromium makes an editing host focusable; Div refuses to
    // focus it without a tabindex.
    let focus_result = host.focus();
    println!("contenteditable without tabindex: focus() -> {focus_result:?}");
    assert!(focus_result.is_err() || !host.has_focus().unwrap());
    host.set_attribute("tabindex", "0").unwrap();
    host.focus().unwrap();
    app.type_text("x");
    let events = take(&log);
    println!(
        "contenteditable with tabindex: events on typing {events:?}, text after typing {:?}",
        host.text_content().unwrap()
    );
    // div-gap: 2. No editing host: no beforeinput, no edit.
    assert!(!events.iter().any(|e| e.starts_with("beforeinput")));
    assert_eq!(host.text_content().unwrap(), "hello");
}
