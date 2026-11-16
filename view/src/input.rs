//! A port of the core of `input.ts`: the view's [`InputState`], which
//! listens for DOM events on the content and runs the handlers and
//! observers that view plugins and the view itself register for them.
//!
//! This part ports the handler registry (`ensureHandlers`,
//! `computeHandlers`), `handleEvent` and `runHandlers`, the work done on
//! every `keydown` before the handlers (the last key, tab-focus mode), and
//! the built-in `keydown`, `scroll` and `wheel` handlers. Focus and blur,
//! mouse selection, clipboard, drag and drop, composition and
//! `beforeinput` come in later parts of milestone M7; they need Div
//! features listed in the design document (gaps 2–9).
//!
//! Div's listeners are `'static` closures, so a listener reaches the view
//! through a weak reference: only a view made with
//! [`EditorView::new_shared`] listens for events.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::{Rc, Weak};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::plugin::{dom_event_handler, DomEventHandler, DomEventHandlers, PluginSource};
use crate::tile::TileKind;
use crate::EditorView;

/// `Date.now()`: milliseconds since the Unix epoch.
pub(crate) fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64() * 1000.0)
}

/// The key codes of modifier keys (`modifierCodes`).
pub(crate) const MODIFIER_CODES: [i32; 8] = [16, 17, 18, 20, 91, 92, 224, 225];

#[derive(Clone, Default)]
struct Handlers {
    observers: Vec<DomEventHandler>,
    handlers: Vec<DomEventHandler>,
}

// What a view's listeners hold: the view, and the events that arrived
// while it was borrowed.
#[derive(Clone)]
pub(crate) struct Shared {
    pub(crate) view: Weak<RefCell<EditorView>>,
    pub(crate) pending: Rc<RefCell<VecDeque<div::Event>>>,
}

/// The view's input state (`InputState`).
pub(crate) struct InputState {
    pub(crate) last_key_code: i32,
    pub(crate) last_key_time: f64,
    pub(crate) last_scroll_top: f64,
    pub(crate) last_scroll_left: f64,
    pub(crate) last_wheel_event: f64,
    /// When non-negative, Tab presses are not given to key handlers,
    /// leaving the default behavior. Above zero, the mode expires at that
    /// time, and any other key clears it (`tabFocusMode`).
    pub(crate) tab_focus_mode: f64,
    pub(crate) last_selection_origin: Option<String>,
    pub(crate) last_selection_time: f64,
    /// -1 outside a composition, else the number of changes made during
    /// it (`composing`). Compositions come in a later part of M7.
    pub(crate) composing: i32,
    handlers: HashMap<String, Handlers>,
    // div-gap: 29 `removeEventListener` for one listener. Div removes the
    // listeners of a type all at once, the host's among them, so a type's
    // listener stays once added; a type without handlers finds none.
    registered: Vec<String>,
    pub(crate) shared: Option<Shared>,
}

impl InputState {
    pub(crate) fn new() -> InputState {
        InputState {
            last_key_code: 0,
            last_key_time: 0.0,
            last_scroll_top: 0.0,
            last_scroll_left: 0.0,
            last_wheel_event: 0.0,
            tab_focus_mode: -1.0,
            last_selection_origin: None,
            last_selection_time: 0.0,
            composing: -1,
            handlers: HashMap::new(),
            registered: Vec::new(),
            shared: None,
        }
    }

    /// `setSelectionOrigin`.
    pub(crate) fn set_selection_origin(&mut self, origin: &str) {
        self.last_selection_origin = Some(origin.to_string());
        self.last_selection_time = now();
    }

    // `ignoreDuringComposition`. Safari's Enter after a composition does
    // not apply.
    fn ignore_during_composition(&self, event: &div::Event) -> bool {
        event.event_type.starts_with("key") && self.composing > 0
    }

    /// The part of `update` that does not need the DOM observer or mouse
    /// selection, which come later.
    pub(crate) fn update(&mut self, transactions: usize) {
        if transactions > 0 {
            self.last_key_code = 0;
            self.last_selection_time = 0.0;
        }
    }
}

// The view's own handlers and observers (`handlers`, `observers`).
static BUILTIN_HANDLERS: LazyLock<DomEventHandlers> = LazyLock::new(|| {
    vec![(
        "keydown".to_string(),
        dom_event_handler(|view, event| {
            let input = view.input_mut();
            input.set_selection_origin("select");
            if event.key_code == 27 && input.tab_focus_mode != 0.0 {
                input.tab_focus_mode = now() + 2000.0;
            }
            false
        }),
    )]
});

static BUILTIN_OBSERVERS: LazyLock<DomEventHandlers> = LazyLock::new(|| {
    let wheel = dom_event_handler(|view, _| {
        view.input_mut().last_wheel_event = now();
        false
    });
    vec![
        (
            "scroll".to_string(),
            dom_event_handler(|view, _| {
                let top = view.scroll_dom().scroll_top().unwrap_or(0.0);
                let left = view.scroll_dom().scroll_left().unwrap_or(0.0);
                let input = view.input_mut();
                input.last_scroll_top = top;
                input.last_scroll_left = left;
                false
            }),
        ),
        ("wheel".to_string(), wheel.clone()),
        ("mousewheel".to_string(), wheel),
    ]
});

// `computeHandlers`: the plugins' handlers and observers, then the view's.
fn compute_handlers(plugins: &[PluginSource]) -> HashMap<String, Handlers> {
    let mut result: HashMap<String, Handlers> = HashMap::new();
    for plugin in plugins {
        for (event_type, handler) in plugin.event_handlers() {
            result
                .entry(event_type.clone())
                .or_default()
                .handlers
                .push(handler.clone());
        }
        for (event_type, observer) in plugin.event_observers() {
            result
                .entry(event_type.clone())
                .or_default()
                .observers
                .push(observer.clone());
        }
    }
    for (event_type, handler) in BUILTIN_HANDLERS.iter() {
        result
            .entry(event_type.clone())
            .or_default()
            .handlers
            .push(handler.clone());
    }
    for (event_type, observer) in BUILTIN_OBSERVERS.iter() {
        result
            .entry(event_type.clone())
            .or_default()
            .observers
            .push(observer.clone());
    }
    result
}

impl EditorView {
    /// `ensureHandlers`: compute the handlers of the view's plugins, and
    /// listen on the content for the event types they handle. Scroll
    /// events come from the scroller, through the DOM observer, which
    /// comes later.
    pub(crate) fn ensure_handlers(&mut self) -> Result<(), div::Error> {
        let plugins = self.plugin_sources();
        let handlers = compute_handlers(&plugins);
        let content = self.content_dom().clone();
        let input = self.input_mut();
        if let Some(shared) = input.shared.clone() {
            let mut types: Vec<&String> = handlers.keys().collect();
            types.sort();
            for event_type in types {
                if event_type == "scroll" || input.registered.contains(event_type) {
                    continue;
                }
                let shared = shared.clone();
                // div-gap: 29 listener options: Div has no `passive`, which
                // upstream sets for types with observers only.
                content.on(event_type, move |event| {
                    let Some(view) = shared.view.upgrade() else {
                        return;
                    };
                    let Ok(mut view) = view.try_borrow_mut() else {
                        // The view is updating, or its host holds it: run
                        // the event after, as upstream does during updates.
                        shared.pending.borrow_mut().push_back(event.clone());
                        return;
                    };
                    view.handle_event(event);
                })?;
                input.registered.push(event_type.clone());
            }
        }
        input.handlers = handlers;
        Ok(())
    }

    /// `handleEvent`: run the handlers for an event on the content.
    pub(crate) fn handle_event(&mut self, event: &div::Event) {
        if !self.event_belongs_to_editor(event) || self.input().ignore_during_composition(event) {
            return;
        }
        if event.event_type == "keydown" && self.input_keydown(event) {
            return;
        }
        self.run_handlers(&event.event_type.clone(), event);
    }

    /// Run the events that arrived while the view was borrowed.
    pub(crate) fn run_pending_events(&mut self) {
        let Some(shared) = self.input().shared.clone() else {
            return;
        };
        loop {
            let Some(event) = shared.pending.borrow_mut().pop_front() else {
                break;
            };
            if self.event_belongs_to_editor(&event)
                && !self.input().ignore_during_composition(&event)
            {
                self.run_handlers(&event.event_type.clone(), &event);
            }
        }
    }

    /// `runHandlers`: run the observers of `event_type`, then its handlers
    /// until one handles the event, whose default is then prevented.
    pub(crate) fn run_handlers(&mut self, event_type: &str, event: &div::Event) {
        let Some(handlers) = self.input().handlers.get(event_type).cloned() else {
            return;
        };
        for observer in &handlers.observers {
            observer(self, event);
        }
        for handler in &handlers.handlers {
            if event.default_prevented() {
                break;
            }
            if handler(self, event) {
                event.prevent_default();
                break;
            }
        }
    }

    // `InputState.keydown`: what always runs for a key, even when a handler
    // handles it. Android's and iOS's key handling does not apply.
    fn input_keydown(&mut self, event: &div::Event) -> bool {
        let input = self.input_mut();
        input.last_key_code = event.key_code;
        input.last_key_time = now();
        if event.key_code == 9
            && input.tab_focus_mode > -1.0
            && (input.tab_focus_mode == 0.0 || now() <= input.tab_focus_mode)
        {
            return true;
        }
        if input.tab_focus_mode > 0.0
            && event.key_code != 27
            && !MODIFIER_CODES.contains(&event.key_code)
        {
            input.tab_focus_mode = -1.0;
        }
        // div-gap: 10 `MutationObserver`. Upstream flushes the DOM
        // observer's pending changes here (unless the key code is 229).
        false
    }

    // `eventBelongsToEditor`.
    fn event_belongs_to_editor(&self, event: &div::Event) -> bool {
        if !event.bubbles() {
            return true;
        }
        if event.default_prevented() {
            return false;
        }
        let mut node = event.target();
        loop {
            let Some(current) = node else {
                return false;
            };
            if current.is_same_node(self.content_dom()).unwrap_or(false) {
                return true;
            }
            // Div has no document fragments or shadow roots to stop at.
            if let Some(tile) = self.doc_view_tiles().get(&current) {
                let tile = &self.doc_view_tiles()[tile];
                if let TileKind::Widget(widget) = &tile.kind {
                    if !widget.is_hidden() && widget.ignore_event(event) {
                        return false;
                    }
                }
            }
            node = current.parent().ok().flatten();
        }
    }

    /// Enable or disable tab-focus mode, in which Tab and Shift-Tab do not
    /// run key bindings, so focus can leave the editor
    /// (`setTabFocusMode`).
    pub fn set_tab_focus_mode(&mut self, to: TabFocusMode) {
        let input = self.input_mut();
        input.tab_focus_mode = match to {
            TabFocusMode::Toggle if input.tab_focus_mode < 0.0 => 0.0,
            TabFocusMode::Toggle => -1.0,
            TabFocusMode::Set(true) => 0.0,
            TabFocusMode::Set(false) => -1.0,
            TabFocusMode::For(_) if input.tab_focus_mode == 0.0 => 0.0,
            TabFocusMode::For(ms) => now() + ms,
        };
    }
}

/// The argument of [`EditorView::set_tab_focus_mode`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TabFocusMode {
    /// Toggle the mode (no argument).
    Toggle,
    /// Enable or disable it (a boolean).
    Set(bool),
    /// Enable it for this many milliseconds, or until a key other than
    /// Tab is pressed (a number). Leaves an enabled mode enabled.
    For(f64),
}
