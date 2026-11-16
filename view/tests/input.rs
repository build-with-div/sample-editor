//! DOM event handling: the port of the core of `input.ts`.
//!
//! Upstream tests this through a browser (`webtest-domevents.ts` and the
//! key handling in `webtest-keymap.ts`). These tests dispatch Div's own
//! events at the content of a view made with [`EditorView::new_shared`],
//! through the same listener route the host's input takes.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};

use div::{Document, Element};
use editor_state::{Compartment, EditorState, EditorStateConfig, Extension, Insertion, RangeSet};
use editor_state::{Text, TransactionSpec};
use editor_view::{
    command, dom_event_handler, Decoration, DomEventHandlers, EditorView, EditorViewConfig,
    KeyBinding, KeyBindings, PluginSpec, PluginValue, TabFocusMode, ViewPlugin, WidgetSpec,
    WidgetType, WindowSize, KEYMAP,
};

type View = Rc<RefCell<EditorView>>;

fn shared_view(doc: &str, extensions: Extension) -> (View, Document) {
    let document = Document::new(800, 600).unwrap();
    let doc = Text::of(&doc.split('\n').collect::<Vec<_>>()).unwrap();
    let state = EditorState::create(EditorStateConfig {
        doc: Some(Insertion::Text(doc)),
        selection: None,
        extensions,
    })
    .unwrap();
    let view = EditorView::new_shared(
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

// A `keydown` event with `key` and `key_code`.
fn key(key: &str, key_code: i32) -> div::Event {
    let mut event = div::Event::new("keydown", true, true);
    event.key_text = key.to_string();
    event.key_code = key_code;
    event
}

// Dispatch `event` at the view's content, or at `target`, and return
// whether its default was prevented.
fn fire(document: &Document, view: &View, event: &div::Event, target: Option<&Element>) -> bool {
    let content = view.borrow().content_dom().clone();
    let allowed = document
        .dispatch_event(target.unwrap_or(&content), event)
        .unwrap();
    assert_eq!(allowed, !event.default_prevented());
    event.default_prevented()
}

// A log of what ran.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);

impl Log {
    fn push(&self, entry: &str) {
        self.0.lock().unwrap().push(entry.to_string());
    }

    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }

    // A handler that logs `name` and returns `handles`.
    fn handlers(&self, event_type: &str, name: &str, handles: bool) -> DomEventHandlers {
        let (log, name) = (self.clone(), name.to_string());
        vec![(
            event_type.to_string(),
            dom_event_handler(move |_, _| {
                log.push(&name);
                handles
            }),
        )]
    }

    fn keymap(&self, key: &str, name: &str) -> Extension {
        let (log, name) = (self.clone(), name.to_string());
        KEYMAP.of(KeyBindings::from(vec![KeyBinding {
            key: Some(key.to_string()),
            run: Some(command(move |_| {
                log.push(&name);
                true
            })),
            ..Default::default()
        }]))
    }
}

#[test]
fn runs_key_bindings_for_keydown_on_the_content() {
    let log = Log::default();
    let (view, document) = shared_view("one", log.keymap("a", "a"));
    let event = key("a", 65);
    assert!(fire(&document, &view, &event, None));
    assert_eq!(log.take(), ["a"]);
    let event = key("b", 66);
    assert!(!fire(&document, &view, &event, None));
    assert!(log.take().is_empty());
}

#[test]
fn a_view_made_with_new_does_not_listen() {
    let log = Log::default();
    let document = Document::new(800, 600).unwrap();
    let state = EditorState::create(EditorStateConfig {
        doc: None,
        selection: None,
        extensions: log.keymap("a", "a"),
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
    let event = key("a", 65);
    assert!(document.dispatch_event(view.content_dom(), &event).unwrap());
    assert!(log.take().is_empty());
}

#[test]
fn observers_run_before_handlers_and_handlers_until_one_handles() {
    let log = Log::default();
    let (view, document) = shared_view(
        "one",
        Extension::from(vec![
            EditorView::dom_event_handlers(log.handlers("keydown", "first", false)),
            EditorView::dom_event_handlers(log.handlers("keydown", "second", true)),
            EditorView::dom_event_handlers(log.handlers("keydown", "third", true)),
            EditorView::dom_event_observers(log.handlers("keydown", "observer", true)),
        ]),
    );
    let event = key("x", 88);
    assert!(fire(&document, &view, &event, None));
    assert_eq!(log.take(), ["observer", "first", "second"]);
}

#[test]
fn an_observer_does_not_handle_the_event() {
    let log = Log::default();
    let (view, document) = shared_view(
        "one",
        EditorView::dom_event_observers(log.handlers("keyup", "observer", true)),
    );
    let event = div::Event::new("keyup", true, true);
    assert!(!fire(&document, &view, &event, None));
    assert_eq!(log.take(), ["observer"]);
}

#[test]
fn handlers_skip_a_prevented_event() {
    let log = Log::default();
    let (view, document) = shared_view(
        "one",
        Extension::from(vec![
            EditorView::dom_event_handlers(vec![(
                "keyup".to_string(),
                dom_event_handler(|_, event| {
                    event.prevent_default();
                    false
                }),
            )]),
            EditorView::dom_event_handlers(log.handlers("keyup", "after", true)),
        ]),
    );
    let event = div::Event::new("keyup", true, true);
    assert!(fire(&document, &view, &event, None));
    assert!(log.take().is_empty());
}

struct Counter(usize);

impl PluginValue for Counter {}

#[test]
fn plugins_handle_events_and_reach_their_value() {
    // A handler reaches its plugin's value through the view, as upstream's
    // reach `this`.
    let handle: Arc<OnceLock<ViewPlugin<Counter>>> = Arc::default();
    let log = Log::default();
    let (in_handler, in_log) = (handle.clone(), log.clone());
    let plugin = ViewPlugin::define(
        |_| Ok(Counter(7)),
        PluginSpec {
            event_handlers: vec![(
                "mousedown".to_string(),
                dom_event_handler(move |view, _| {
                    let count = view.plugin(in_handler.get().unwrap()).unwrap().0;
                    in_log.push(&format!("handler {count}"));
                    true
                }),
            )],
            event_observers: log.handlers("mousedown", "observer", false),
            ..PluginSpec::default()
        },
    );
    handle.set(plugin.clone()).ok().unwrap();
    let (view, document) = shared_view(
        "one",
        Extension::from(vec![
            EditorView::dom_event_handlers(log.handlers("mousedown", "view", false)),
            plugin.extension(),
        ]),
    );
    let event = div::Event::new("mousedown", true, true);
    assert!(fire(&document, &view, &event, None));
    assert_eq!(log.take(), ["observer", "view", "handler 7"]);
}

#[test]
fn tab_focus_mode_leaves_tab_to_the_default() {
    let log = Log::default();
    let (view, document) = shared_view("one", log.keymap("Tab", "tab"));
    let tab = || key("Tab", 9);
    assert!(fire(&document, &view, &tab(), None));
    assert_eq!(log.take(), ["tab"]);

    view.borrow_mut()
        .set_tab_focus_mode(TabFocusMode::Set(true));
    assert!(!fire(&document, &view, &tab(), None));
    assert!(log.take().is_empty());
    // An enabled mode stays across other keys.
    fire(&document, &view, &key("a", 65), None);
    assert!(!fire(&document, &view, &tab(), None));

    view.borrow_mut().set_tab_focus_mode(TabFocusMode::Toggle);
    assert!(fire(&document, &view, &tab(), None));
    assert_eq!(log.take(), ["tab"]);
}

#[test]
fn escape_enables_tab_focus_mode_until_another_key() {
    let log = Log::default();
    let (view, document) = shared_view("one", log.keymap("Tab", "tab"));
    fire(&document, &view, &key("Escape", 27), None);
    assert!(!fire(&document, &view, &key("Tab", 9), None));
    // Modifier keys leave the timed mode alone; others end it.
    fire(&document, &view, &key("Shift", 16), None);
    assert!(!fire(&document, &view, &key("Tab", 9), None));
    fire(&document, &view, &key("a", 65), None);
    assert!(fire(&document, &view, &key("Tab", 9), None));
    assert_eq!(log.take(), ["tab"]);
}

#[test]
fn a_timed_mode_does_not_override_an_enabled_one() {
    let log = Log::default();
    let (view, document) = shared_view("one", log.keymap("Tab", "tab"));
    view.borrow_mut()
        .set_tab_focus_mode(TabFocusMode::For(60_000.0));
    assert!(!fire(&document, &view, &key("Tab", 9), None));
    fire(&document, &view, &key("a", 65), None);
    assert!(fire(&document, &view, &key("Tab", 9), None));

    view.borrow_mut()
        .set_tab_focus_mode(TabFocusMode::Set(true));
    view.borrow_mut()
        .set_tab_focus_mode(TabFocusMode::For(60_000.0));
    fire(&document, &view, &key("a", 65), None);
    assert!(!fire(&document, &view, &key("Tab", 9), None));
}

struct Button {
    ignore: bool,
}

impl WidgetType for Button {
    fn to_dom(&self, document: &Document) -> Result<Element, div::Error> {
        let dom = Element::create(document, "span")?;
        dom.set_attribute("class", "button")?;
        let inner = Element::create(document, "strong")?;
        inner.set_text("x")?;
        dom.append_child(&inner)?;
        Ok(dom)
    }

    fn eq(&self, other: &Self) -> bool {
        self.ignore == other.ignore
    }

    fn ignore_event(&self, _event: &div::Event) -> bool {
        self.ignore
    }
}

// The first element in `root`'s subtree with class `class`.
fn find_class(root: &Element, class: &str) -> Option<Element> {
    let mut child = root.first_child().unwrap();
    while let Some(node) = child {
        let found = node
            .get_attribute("class")
            .unwrap()
            .is_some_and(|list| list.split_whitespace().any(|c| c == class));
        if found {
            return Some(node);
        }
        if let Some(found) = find_class(&node, class) {
            return Some(found);
        }
        child = node.next_sibling().unwrap();
    }
    None
}

fn widget_view(log: &Log, ignore: bool) -> (View, Document) {
    let widget = Decoration::widget(WidgetSpec::new(Button { ignore }))
        .range(1, 1)
        .unwrap();
    shared_view(
        "one",
        Extension::from(vec![
            EditorView::decorations(RangeSet::of(vec![widget], false)),
            EditorView::dom_event_handlers(log.handlers("mousedown", "handler", true)),
        ]),
    )
}

#[test]
fn events_in_widgets_are_ignored_by_default() {
    let log = Log::default();
    let (view, document) = widget_view(&log, true);
    let button = find_class(view.borrow().content_dom(), "button").unwrap();
    let inner = button.first_child().unwrap().unwrap();
    let event = div::Event::new("mousedown", true, true);
    assert!(!fire(&document, &view, &event, Some(&inner)));
    assert!(log.take().is_empty());
    // Outside the widget, the event is the editor's.
    let event = div::Event::new("mousedown", true, true);
    assert!(fire(&document, &view, &event, None));
    assert_eq!(log.take(), ["handler"]);
}

#[test]
fn a_widget_can_let_the_editor_handle_its_events() {
    let log = Log::default();
    let (view, document) = widget_view(&log, false);
    let button = find_class(view.borrow().content_dom(), "button").unwrap();
    let inner = button.first_child().unwrap().unwrap();
    let event = div::Event::new("mousedown", true, true);
    assert!(fire(&document, &view, &event, Some(&inner)));
    assert_eq!(log.take(), ["handler"]);
}

#[test]
fn events_while_the_view_is_borrowed_run_after_its_update() {
    let log = Log::default();
    let (view, document) = shared_view("one", log.keymap("a", "a"));
    let content = view.borrow().content_dom().clone();
    {
        let mut borrowed = view.borrow_mut();
        let event = key("a", 65);
        assert!(document.dispatch_event(&content, &event).unwrap());
        assert!(log.take().is_empty());
        let tr = borrowed.state().update(TransactionSpec::default()).unwrap();
        borrowed.dispatch(tr).unwrap();
    }
    assert_eq!(log.take(), ["a"]);
}

#[test]
fn reconfiguring_changes_the_handlers() {
    let log = Log::default();
    let handlers = Compartment::new();
    let (view, document) = shared_view("one", handlers.of(Extension::from(vec![])));
    let event = div::Event::new("keyup", true, true);
    assert!(!fire(&document, &view, &event, None));

    let tr = view
        .borrow()
        .state()
        .update(TransactionSpec {
            effects: vec![handlers.reconfigure(EditorView::dom_event_handlers(
                log.handlers("keyup", "added", true),
            ))],
            ..Default::default()
        })
        .unwrap();
    view.borrow_mut().dispatch(tr).unwrap();
    let event = div::Event::new("keyup", true, true);
    assert!(fire(&document, &view, &event, None));
    assert_eq!(log.take(), ["added"]);

    let tr = view
        .borrow()
        .state()
        .update(TransactionSpec {
            effects: vec![handlers.reconfigure(Extension::from(vec![]))],
            ..Default::default()
        })
        .unwrap();
    view.borrow_mut().dispatch(tr).unwrap();
    let event = div::Event::new("keyup", true, true);
    assert!(!fire(&document, &view, &event, None));
    assert!(log.take().is_empty());
}

#[test]
fn a_dropped_view_ignores_its_events() {
    let log = Log::default();
    let (view, document) = shared_view("one", log.keymap("a", "a"));
    let content = view.borrow().content_dom().clone();
    drop(view);
    let event = key("a", 65);
    assert!(document.dispatch_event(&content, &event).unwrap());
    assert!(log.take().is_empty());
}
