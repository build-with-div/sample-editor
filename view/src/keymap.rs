//! A port of `keymap.ts`: key bindings, the [`KEYMAP`] facet that
//! registers them, and [`run_scope_handlers`], which runs them for a
//! keyboard event.
//!
//! The facet enables a `keydown` handler, with default precedence, that
//! runs the `"editor"` scope's bindings for the content's key events.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::{Duration, Instant};

use editor_state::{Callback, Extension, Facet, FacetConfig, Prec, StateCommand};

use crate::keyname::{alt_key, base, ctrl_key, key_name, meta_key, shift, shift_key};
use crate::plugin::dom_event_handler;
use crate::EditorView;

/// A command (`Command`): a function that acts on a view and returns
/// whether it applied.
pub type Command = Callback<dyn Fn(&mut EditorView) -> bool + Send + Sync>;

/// Wrap `f` as a [`Command`].
pub fn command(f: impl Fn(&mut EditorView) -> bool + Send + Sync + 'static) -> Command {
    Callback(Arc::new(f))
}

/// A [`StateCommand`] run on a view: the view's state is its state, and
/// the transactions it dispatches update the view, as upstream passes the
/// view where a state command expects `{state, dispatch}`.
///
/// The transactions are dispatched once the command returns, as the state
/// it reads is borrowed until then. An error from the view's DOM is
/// logged, as upstream logs an exception from a key handler.
pub fn state_command(f: StateCommand) -> Command {
    command(move |view: &mut EditorView| {
        let mut transactions = Vec::new();
        let applied = f(view.state(), &mut |tr| transactions.push(tr));
        for tr in transactions {
            if let Err(error) = view.dispatch(tr) {
                // `logException`, as `plugin::log_exception` reports.
                eprintln!("CodeMirror key binding: {error}");
            }
        }
        applied
    })
}

/// A function a binding calls for every key that is not a multi-stroke
/// prefix (`KeyBinding.any`), given the view and the `keydown` event.
pub type AnyKeyHandler = Callback<dyn Fn(&mut EditorView, &div::Event) -> bool + Send + Sync>;

/// A key binding (`KeyBinding`): a key name and the command it runs.
///
/// Key names are strings like `"Shift-Ctrl-Enter"`: a key identifier, as
/// `KeyboardEvent.key` names keys, after zero or more modifiers. Lowercase
/// letters name letter keys, uppercase letters those keys with Shift, and
/// `"Space"` is an alias for `" "`. The modifiers `Shift-` (or `s-`),
/// `Alt-` (or `a-`), `Ctrl-` (or `c-` or `Control-`) and `Cmd-` (or `m-`
/// or `Meta-`) may come in any order, and `Mod-` is `Cmd-` on macOS and
/// `Ctrl-` elsewhere. Key names separated by spaces form a multi-stroke
/// binding, which fires when the keys are pressed one after another.
#[derive(Clone, Default, PartialEq)]
pub struct KeyBinding {
    /// The key name. A platform's own name (`mac`, `win` or `linux`) takes
    /// precedence on that platform; without either, the binding is
    /// ignored.
    pub key: Option<String>,
    /// The key name on macOS.
    pub mac: Option<String>,
    /// The key name on Windows.
    pub win: Option<String>,
    /// The key name on Linux.
    pub linux: Option<String>,
    /// The command to run. When it returns false, further bindings for
    /// the key are tried.
    pub run: Option<Command>,
    /// A command for the key name with `Shift-` in front.
    pub shift: Option<Command>,
    /// A function called for every key that is not a multi-stroke prefix.
    pub any: Option<AnyKeyHandler>,
    /// The scopes the binding applies in, separated by spaces; `"editor"`
    /// (focus on the content) when `None`.
    pub scope: Option<String>,
    /// Prevent further handling of the bound key even when its commands
    /// return false (`preventDefault`).
    pub prevent_default: bool,
    /// Stop the propagation of the events whose default this binding
    /// prevents (`stopPropagation`).
    pub stop_propagation: bool,
}

/// A keymap: bindings in a [`KEYMAP`] input, compared by identity, as
/// upstream compares the arrays given to `keymap.of`.
#[derive(Clone)]
pub struct KeyBindings(pub Arc<[KeyBinding]>);

impl From<Vec<KeyBinding>> for KeyBindings {
    fn from(bindings: Vec<KeyBinding>) -> Self {
        KeyBindings(bindings.into())
    }
}

impl PartialEq for KeyBindings {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// The keymaps of a state, with the bindings built from them once, as
/// upstream keeps them in a `WeakMap` keyed by the facet's value.
pub struct Keymaps {
    bindings: Vec<KeyBindings>,
    built: OnceLock<Arc<Keymap>>,
}

impl PartialEq for Keymaps {
    fn eq(&self, other: &Self) -> bool {
        self.bindings == other.bindings
    }
}

/// The facet that registers keymaps (`keymap`). Keymaps given earlier, or
/// with higher precedence, are tried first; once a command returns true,
/// no further handler runs.
pub static KEYMAP: LazyLock<Facet<KeyBindings, Keymaps>> = LazyLock::new(|| {
    Facet::define_with(
        FacetConfig::new(|bindings: &[KeyBindings]| Keymaps {
            bindings: bindings.to_vec(),
            built: OnceLock::new(),
        })
        .enables_with(|_| handle_key_events()),
    )
});

// `handleKeyEvents`.
fn handle_key_events() -> Extension {
    Prec::default(EditorView::dom_event_handlers(vec![(
        "keydown".to_string(),
        dom_event_handler(|view, event| {
            let map = get_keymap(view);
            run_handlers(&map, event, view, "editor")
        }),
    )]))
}

// A function a binding runs. Commands are compared by identity, as
// upstream's `Set` of the functions it ran does.
#[derive(Clone)]
enum Runner {
    Command(Command),
    // The closure upstream makes, per binding, around an `any` handler.
    Any(Arc<AnyKeyHandler>),
    // The closure that stores a multi-stroke prefix.
    Prefix { prefix: String, scope: String },
}

impl Runner {
    fn id(&self) -> *const () {
        match self {
            Runner::Command(command) => Arc::as_ptr(&command.0) as *const (),
            Runner::Any(handler) => Arc::as_ptr(handler) as *const (),
            Runner::Prefix { .. } => self as *const Runner as *const (),
        }
    }
}

#[derive(Clone)]
struct Binding {
    prevent_default: bool,
    stop_propagation: bool,
    run: Vec<Runner>,
}

// In each scope, the `_any` key holds the bindings that apply to all keys.
struct Keymap(HashMap<String, HashMap<String, Binding>>);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Platform {
    Mac,
    Win,
    Linux,
    Key,
}

const CURRENT_PLATFORM: Platform = if cfg!(target_os = "macos") {
    Platform::Mac
} else if cfg!(windows) {
    Platform::Win
} else if cfg!(target_os = "linux") {
    Platform::Linux
} else {
    Platform::Key
};

// Split `name` at each `separator` that does not end it (`/-(?!$)/`).
fn split_not_last(name: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    for (i, c) in name.char_indices() {
        if c == separator && i + c.len_utf8() < name.len() {
            parts.push(&name[start..i]);
            start = i + c.len_utf8();
        }
    }
    parts.push(&name[start..]);
    parts
}

fn normalize_key_name(name: &str, platform: Platform) -> String {
    let parts = split_not_last(name, '-');
    let mut result = parts[parts.len() - 1].to_string();
    if result == "Space" {
        result = " ".to_string();
    }
    let (mut alt, mut ctrl, mut shift, mut meta) = (false, false, false, false);
    for modifier in &parts[..parts.len() - 1] {
        match modifier.to_ascii_lowercase().as_str() {
            "cmd" | "meta" | "m" => meta = true,
            "a" | "alt" => alt = true,
            "c" | "ctrl" | "control" => ctrl = true,
            "s" | "shift" => shift = true,
            "mod" => {
                if platform == Platform::Mac {
                    meta = true;
                } else {
                    ctrl = true;
                }
            }
            _ => panic!("Unrecognized modifier name: {modifier}"),
        }
    }
    if alt {
        result = format!("Alt-{result}");
    }
    if ctrl {
        result = format!("Ctrl-{result}");
    }
    if meta {
        result = format!("Meta-{result}");
    }
    if shift {
        result = format!("Shift-{result}");
    }
    result
}

fn modifiers(name: &str, event: &div::Event, shift: bool) -> String {
    let mut name = name.to_string();
    if alt_key(event) {
        name = format!("Alt-{name}");
    }
    if ctrl_key(event) {
        name = format!("Ctrl-{name}");
    }
    if meta_key(event) {
        name = format!("Meta-{name}");
    }
    if shift && shift_key(event) {
        name = format!("Shift-{name}");
    }
    name
}

fn get_keymap(view: &EditorView) -> Arc<Keymap> {
    let keymaps = view.state().facet(&KEYMAP);
    keymaps
        .built
        .get_or_init(|| {
            let bindings: Vec<KeyBinding> = keymaps
                .bindings
                .iter()
                .flat_map(|bindings| bindings.0.iter().cloned())
                .collect();
            Arc::new(build_keymap(&bindings, CURRENT_PLATFORM))
        })
        .clone()
}

/// Run the key bindings registered for `scope` for `event`, a `keydown`
/// event (`runScopeHandlers`). Returns true when a binding handled it.
///
/// Panics, as upstream throws, when a binding's key name has an unknown
/// modifier, or a key name is used both as a binding and as the prefix
/// of a multi-stroke binding.
pub fn run_scope_handlers(view: &mut EditorView, event: &div::Event, scope: &str) -> bool {
    let map = get_keymap(view);
    run_handlers(&map, event, view, scope)
}

// A pressed prefix of a multi-stroke binding, waiting for the next key.
struct StoredPrefix {
    view: u64,
    prefix: String,
    scope: String,
    stored: Instant,
}

thread_local! {
    static STORED_PREFIX: RefCell<Option<StoredPrefix>> = const { RefCell::new(None) };
}

// Upstream forgets a prefix after this long, with a timeout; here the
// prefix is ignored once it is this old.
const PREFIX_TIMEOUT: Duration = Duration::from_millis(4000);

// The views, numbered for the stored prefix, which upstream compares by
// identity.
static NEXT_VIEW_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_view_id() -> u64 {
    NEXT_VIEW_ID.fetch_add(1, Ordering::Relaxed)
}

fn build_keymap(bindings: &[KeyBinding], platform: Platform) -> Keymap {
    let mut bound: HashMap<String, HashMap<String, Binding>> = HashMap::new();
    let mut is_prefix: HashMap<String, bool> = HashMap::new();

    let mut check_prefix = |name: &str, is: bool| match is_prefix.get(name) {
        None => {
            is_prefix.insert(name.to_string(), is);
        }
        Some(&current) if current != is => panic!(
            "Key binding {name} is used both as a regular binding and as a multi-stroke prefix"
        ),
        Some(_) => {}
    };

    let mut add = |bound: &mut HashMap<String, HashMap<String, Binding>>,
                   scope: &str,
                   key: &str,
                   command: Option<&Command>,
                   prevent_default: bool,
                   stop_propagation: bool| {
        let scope_obj = bound.entry(scope.to_string()).or_default();
        let parts: Vec<String> = split_not_last(key, ' ')
            .into_iter()
            .map(|part| normalize_key_name(part, platform))
            .collect();
        for i in 1..parts.len() {
            let prefix = parts[..i].join(" ");
            check_prefix(&prefix, true);
            if !scope_obj.contains_key(&prefix) {
                scope_obj.insert(
                    prefix.clone(),
                    Binding {
                        prevent_default: true,
                        stop_propagation: false,
                        run: vec![Runner::Prefix {
                            prefix,
                            scope: scope.to_string(),
                        }],
                    },
                );
            }
        }
        let full = parts.join(" ");
        check_prefix(&full, false);
        if !scope_obj.contains_key(&full) {
            let run = scope_obj
                .get("_any")
                .map(|any| any.run.clone())
                .unwrap_or_default();
            scope_obj.insert(
                full.clone(),
                Binding {
                    prevent_default: false,
                    stop_propagation: false,
                    run,
                },
            );
        }
        let binding = scope_obj.get_mut(&full).expect("the binding");
        if let Some(command) = command {
            binding.run.push(Runner::Command(command.clone()));
        }
        if prevent_default {
            binding.prevent_default = true;
        }
        if stop_propagation {
            binding.stop_propagation = true;
        }
    };

    for b in bindings {
        let scopes: Vec<&str> = match b.scope.as_deref() {
            Some(scope) if !scope.is_empty() => scope.split(' ').collect(),
            _ => vec!["editor"],
        };
        if let Some(any) = &b.any {
            for scope in &scopes {
                let scope_obj = bound.entry(scope.to_string()).or_default();
                scope_obj.entry("_any".to_string()).or_insert(Binding {
                    prevent_default: false,
                    stop_propagation: false,
                    run: Vec::new(),
                });
                // Upstream makes a closure per key, so each is a function of
                // its own.
                for binding in scope_obj.values_mut() {
                    binding.run.push(Runner::Any(Arc::new(any.clone())));
                }
            }
        }
        let platform_name = match platform {
            Platform::Mac => b.mac.as_deref(),
            Platform::Win => b.win.as_deref(),
            Platform::Linux => b.linux.as_deref(),
            Platform::Key => None,
        };
        let Some(name) = platform_name
            .filter(|name| !name.is_empty())
            .or(b.key.as_deref())
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        for scope in &scopes {
            add(
                &mut bound,
                scope,
                name,
                b.run.as_ref(),
                b.prevent_default,
                b.stop_propagation,
            );
            if b.shift.is_some() {
                add(
                    &mut bound,
                    scope,
                    &format!("Shift-{name}"),
                    b.shift.as_ref(),
                    b.prevent_default,
                    b.stop_propagation,
                );
            }
        }
    }
    Keymap(bound)
}

use crate::input::MODIFIER_CODES;

fn run_handlers(map: &Keymap, event: &div::Event, view: &mut EditorView, scope: &str) -> bool {
    let name = key_name(event);
    let is_char = name.chars().count() == 1 && name != " ";
    let mut prefix = String::new();
    let mut handled = false;
    let mut prevented = false;
    let mut stop_propagation = false;
    STORED_PREFIX.with(|stored| {
        let mut stored = stored.borrow_mut();
        if stored
            .as_ref()
            .is_some_and(|s| s.stored.elapsed() >= PREFIX_TIMEOUT)
        {
            *stored = None;
        }
        if let Some(s) = stored.as_ref() {
            if s.view == view.id() && s.scope == scope {
                prefix = format!("{} ", s.prefix);
                if !MODIFIER_CODES.contains(&event.key_code) {
                    prevented = true;
                    *stored = None;
                }
            }
        }
    });

    let mut ran: Vec<*const ()> = Vec::new();
    let mut run_for = |binding: Option<&Binding>,
                       view: &mut EditorView,
                       prevented: &mut bool,
                       stop_propagation: &mut bool|
     -> bool {
        let Some(binding) = binding else {
            return false;
        };
        for runner in &binding.run {
            let id = runner.id();
            if ran.contains(&id) {
                continue;
            }
            ran.push(id);
            let applied = match runner {
                Runner::Command(command) => command(view),
                Runner::Any(handler) => handler(view, event),
                Runner::Prefix { prefix, scope } => {
                    let stored = StoredPrefix {
                        view: view.id(),
                        prefix: prefix.clone(),
                        scope: scope.clone(),
                        stored: Instant::now(),
                    };
                    STORED_PREFIX.with(|s| *s.borrow_mut() = Some(stored));
                    true
                }
            };
            if applied {
                if binding.stop_propagation {
                    *stop_propagation = true;
                }
                return true;
            }
        }
        if binding.prevent_default {
            if binding.stop_propagation {
                *stop_propagation = true;
            }
            *prevented = true;
        }
        false
    };

    if let Some(scope_obj) = map.0.get(scope) {
        let lookup = |name: &str| scope_obj.get(&format!("{prefix}{name}"));
        if run_for(
            lookup(&modifiers(&name, event, !is_char)),
            view,
            &mut prevented,
            &mut stop_propagation,
        ) {
            handled = true;
        } else if is_char
            && (alt_key(event) || meta_key(event) || ctrl_key(event))
            // Ctrl-Alt may be used for AltGr on Windows.
            && !(cfg!(windows) && ctrl_key(event) && alt_key(event))
            // Alt-combinations on macOS tend to be typed characters.
            && !(cfg!(target_os = "macos")
                && alt_key(event)
                && !(ctrl_key(event) || meta_key(event)))
            && base(event.key_code).is_some_and(|base_name| base_name != name)
        {
            let base_name = base(event.key_code).expect("a base name");
            if run_for(
                lookup(&modifiers(base_name, event, true)),
                view,
                &mut prevented,
                &mut stop_propagation,
            ) {
                handled = true;
            } else if shift_key(event) {
                let shift_name = shift(event.key_code);
                if shift_name != Some(name.as_str())
                    && shift_name != Some(base_name)
                    && run_for(
                        lookup(&modifiers(shift_name.unwrap_or("undefined"), event, false)),
                        view,
                        &mut prevented,
                        &mut stop_propagation,
                    )
                {
                    handled = true;
                }
            }
        } else if is_char
            && shift_key(event)
            && run_for(
                lookup(&modifiers(&name, event, true)),
                view,
                &mut prevented,
                &mut stop_propagation,
            )
        {
            handled = true;
        }
        if !handled
            && run_for(
                scope_obj.get("_any"),
                view,
                &mut prevented,
                &mut stop_propagation,
            )
        {
            handled = true;
        }
    }

    if prevented {
        handled = true;
    }
    if handled && stop_propagation {
        event.stop_propagation();
    }
    handled
}
