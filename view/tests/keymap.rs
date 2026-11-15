//! Key bindings: the port of `w3c-keyname` and `keymap.ts`.
//!
//! Upstream has no tests for either file, so these are the port's own,
//! checked against the two sources. Events are Div's own keyboard events,
//! with `key_text` as `KeyboardEvent.key` and `key_code` as `keyCode`.

use std::sync::{Arc, Mutex};

use div::Document;
use editor_state::Callback;
use editor_state::{
    ChangeSpec, EditorState, EditorStateConfig, Extension, Insertion, Prec, Text, Transaction,
    TransactionSpec,
};
use editor_view::keyname::{base, key_name, shift};
use editor_view::{
    command, run_scope_handlers, state_command, AnyKeyHandler, Command, EditorView,
    EditorViewConfig, KeyBinding, KeyBindings, WindowSize, KEYMAP,
};

const SHIFT: i32 = 1;
const CTRL: i32 = 2;
const ALT: i32 = 4;
const META: i32 = 8;

// The modifier `Mod-` stands for.
const MOD: i32 = if cfg!(target_os = "macos") {
    META
} else {
    CTRL
};

fn temp_view(doc: &str, extensions: Extension) -> (EditorView, Document) {
    let document = Document::new(800, 600).unwrap();
    let doc = Text::of(&doc.split('\n').collect::<Vec<_>>()).unwrap();
    let state = EditorState::create(EditorStateConfig {
        doc: Some(Insertion::Text(doc)),
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

// A `keydown` event with `key` and `key_code`, and `modifiers` held.
fn key(key: &str, key_code: i32, modifiers: i32) -> div::Event {
    let mut event = div::Event::new("keydown", true, true);
    event.key_text = key.to_string();
    event.key_code = key_code;
    event.modifiers = modifiers;
    event
}

// A log of the commands that ran.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);

impl Log {
    // A command that logs `name` and returns `applies`.
    fn command(&self, name: &str, applies: bool) -> Command {
        let (log, name) = (self.0.clone(), name.to_string());
        command(move |_| {
            log.lock().unwrap().push(name.clone());
            applies
        })
    }

    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

fn binding(key: &str, run: Command) -> KeyBinding {
    KeyBinding {
        key: Some(key.to_string()),
        run: Some(run),
        ..KeyBinding::default()
    }
}

fn keymap(bindings: Vec<KeyBinding>) -> Extension {
    KEYMAP.of(KeyBindings::from(bindings))
}

fn run(view: &mut EditorView, event: &div::Event) -> bool {
    run_scope_handlers(view, event, "editor")
}

#[test]
fn names_keys_from_key_and_key_code() {
    assert_eq!(key_name(&key("a", 65, 0)), "a");
    assert_eq!(key_name(&key("ArrowLeft", 37, 0)), "ArrowLeft");
    // Without `key`, the code names the key, with Shift's table when it is
    // held.
    assert_eq!(key_name(&key("", 65, 0)), "a");
    assert_eq!(key_name(&key("", 65, SHIFT)), "A");
    assert_eq!(key_name(&key("", 49, SHIFT)), "!");
    assert_eq!(key_name(&key("", 112, 0)), "F1");
    assert_eq!(key_name(&key("", 101, 0)), "5");
    assert_eq!(key_name(&key("Unidentified", 13, 0)), "Enter");
    assert_eq!(key_name(&key("Unidentified", 0, 0)), "Unidentified");
    assert_eq!(key_name(&key("", 0, 0)), "Unidentified");
    // Names older browsers produced.
    assert_eq!(key_name(&key("Esc", 27, 0)), "Escape");
    assert_eq!(key_name(&key("Del", 46, 0)), "Delete");
    assert_eq!(key_name(&key("Left", 37, 0)), "ArrowLeft");
    assert_eq!(key_name(&key("Down", 40, 0)), "ArrowDown");
    // The tables.
    assert_eq!(base(219), Some("["));
    assert_eq!(shift(219), Some("{"));
    // A code without a shifted name has its base name.
    assert_eq!(shift(13), Some("Enter"));
    assert_eq!(base(0), None);
}

#[cfg(target_os = "macos")]
#[test]
fn ignores_key_with_shift_and_cmd_on_macos() {
    assert_eq!(key_name(&key("a", 65, SHIFT | META)), "A");
}

#[test]
fn runs_a_bound_command() {
    let log = Log::default();
    let (mut view, _document) = temp_view("", keymap(vec![binding("a", log.command("a", true))]));
    assert!(run(&mut view, &key("a", 65, 0)));
    assert_eq!(log.take(), ["a"]);
    assert!(!run(&mut view, &key("b", 66, 0)));
    assert!(log.take().is_empty());
}

#[test]
fn tries_bindings_until_one_applies() {
    let log = Log::default();
    let (mut view, _document) = temp_view(
        "",
        Extension::from(vec![
            keymap(vec![binding("Mod-a", log.command("first", false))]),
            keymap(vec![binding("Mod-a", log.command("second", true))]),
            keymap(vec![binding("Mod-a", log.command("third", true))]),
        ]),
    );
    assert!(run(&mut view, &key("a", 65, MOD)));
    assert_eq!(log.take(), ["first", "second"]);
}

#[test]
fn returns_false_when_no_command_applies() {
    let log = Log::default();
    let (mut view, _document) = temp_view("", keymap(vec![binding("x", log.command("x", false))]));
    assert!(!run(&mut view, &key("x", 88, 0)));
    assert_eq!(log.take(), ["x"]);
}

#[test]
fn runs_a_command_once_per_key() {
    let log = Log::default();
    let shared = log.command("shared", false);
    let (mut view, _document) = temp_view(
        "",
        keymap(vec![binding("y", shared.clone()), binding("y", shared)]),
    );
    run(&mut view, &key("y", 89, 0));
    assert_eq!(log.take(), ["shared"]);
}

#[test]
fn keymaps_with_higher_precedence_come_first() {
    let log = Log::default();
    let (mut view, _document) = temp_view(
        "",
        Extension::from(vec![
            keymap(vec![binding("q", log.command("default", true))]),
            Prec::high(keymap(vec![binding("q", log.command("high", true))])),
        ]),
    );
    run(&mut view, &key("q", 81, 0));
    assert_eq!(log.take(), ["high"]);
}

#[test]
fn normalizes_modifiers() {
    let log = Log::default();
    let (mut view, _document) = temp_view(
        "",
        keymap(vec![
            binding("Mod-Shift-x", log.command("mod-shift-x", true)),
            binding("a-c-Enter", log.command("ctrl-alt-enter", true)),
            binding("Shift-Alt-Tab", log.command("shift-alt-tab", true)),
            binding("Ctrl--", log.command("ctrl-minus", true)),
            binding("Mod-Space", log.command("mod-space", true)),
        ]),
    );
    // Ctrl-Shift-X types "X": the binding is found through the key code's
    // base name, with Shift.
    assert!(run(&mut view, &key("X", 88, MOD | SHIFT)));
    assert!(run(&mut view, &key("Enter", 13, CTRL | ALT)));
    assert!(run(&mut view, &key("Tab", 9, SHIFT | ALT)));
    assert!(run(&mut view, &key("-", 189, CTRL)));
    assert!(run(&mut view, &key(" ", 32, MOD)));
    assert_eq!(
        log.take(),
        [
            "mod-shift-x",
            "ctrl-alt-enter",
            "shift-alt-tab",
            "ctrl-minus",
            "mod-space"
        ]
    );
}

#[test]
fn names_typed_characters_with_their_case() {
    let log = Log::default();
    let (mut view, _document) = temp_view(
        "",
        keymap(vec![
            binding("A", log.command("A", true)),
            binding("?", log.command("?", true)),
        ]),
    );
    // A character's name already reflects Shift.
    assert!(run(&mut view, &key("A", 65, SHIFT)));
    assert!(run(&mut view, &key("?", 191, SHIFT)));
    assert!(!run(&mut view, &key("a", 65, 0)));
    assert_eq!(log.take(), ["A", "?"]);
}

#[test]
fn finds_shifted_names_through_the_key_code() {
    let log = Log::default();
    let (mut view, _document) = temp_view(
        "",
        keymap(vec![binding(
            "Ctrl-Shift-1",
            log.command("ctrl-shift-1", true),
        )]),
    );
    // Ctrl-Shift-1 types "!" on a US layout.
    assert!(run(&mut view, &key("!", 49, CTRL | SHIFT)));
    assert_eq!(log.take(), ["ctrl-shift-1"]);
}

#[test]
fn shift_binds_the_shifted_key() {
    let log = Log::default();
    let (mut view, _document) = temp_view(
        "",
        keymap(vec![KeyBinding {
            key: Some("Enter".into()),
            run: Some(log.command("enter", true)),
            shift: Some(log.command("shift-enter", true)),
            ..KeyBinding::default()
        }]),
    );
    run(&mut view, &key("Enter", 13, 0));
    run(&mut view, &key("Enter", 13, SHIFT));
    assert_eq!(log.take(), ["enter", "shift-enter"]);
}

#[test]
fn prevent_default_handles_the_key() {
    let log = Log::default();
    let (mut view, _document) = temp_view(
        "",
        keymap(vec![KeyBinding {
            key: Some("Mod-u".into()),
            run: Some(log.command("undo-selection", false)),
            prevent_default: true,
            ..KeyBinding::default()
        }]),
    );
    let event = key("u", 85, MOD);
    assert!(run(&mut view, &event));
    assert!(!event.propagation_stopped());
    assert_eq!(log.take(), ["undo-selection"]);
}

#[test]
fn stop_propagation_stops_handled_events() {
    let log = Log::default();
    let (mut view, _document) = temp_view(
        "",
        keymap(vec![
            KeyBinding {
                key: Some("F2".into()),
                run: Some(log.command("f2", true)),
                stop_propagation: true,
                ..KeyBinding::default()
            },
            KeyBinding {
                key: Some("F3".into()),
                run: Some(log.command("f3", false)),
                prevent_default: true,
                stop_propagation: true,
                ..KeyBinding::default()
            },
            KeyBinding {
                key: Some("F4".into()),
                run: Some(log.command("f4", false)),
                stop_propagation: true,
                ..KeyBinding::default()
            },
        ]),
    );
    let f2 = key("F2", 113, 0);
    assert!(run(&mut view, &f2));
    assert!(f2.propagation_stopped());
    let f3 = key("F3", 114, 0);
    assert!(run(&mut view, &f3));
    assert!(f3.propagation_stopped());
    // Not handled, so not stopped.
    let f4 = key("F4", 115, 0);
    assert!(!run(&mut view, &f4));
    assert!(!f4.propagation_stopped());
}

#[test]
fn any_sees_every_key() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let any: AnyKeyHandler = {
        let seen = seen.clone();
        Callback(Arc::new(move |_: &mut EditorView, event: &div::Event| {
            seen.lock().unwrap().push(key_name(event));
            false
        }))
    };
    let log = Log::default();
    let (mut view, _document) = temp_view(
        "",
        keymap(vec![
            binding("a", log.command("a", false)),
            KeyBinding {
                any: Some(any),
                ..KeyBinding::default()
            },
            binding("b", log.command("b", false)),
        ]),
    );
    assert!(!run(&mut view, &key("a", 65, 0)));
    assert!(!run(&mut view, &key("b", 66, 0)));
    assert!(!run(&mut view, &key("c", 67, 0)));
    assert_eq!(log.take(), ["a", "b"]);
    // `a` was bound before the handler: its binding and the catch-all each
    // carry a closure of their own around it, so it runs twice. `b` was
    // bound after, and shares the catch-all's closure.
    assert_eq!(*seen.lock().unwrap(), ["a", "a", "b", "c"]);
}

#[test]
fn any_can_handle_a_key() {
    let any: AnyKeyHandler = Callback(Arc::new(|_: &mut EditorView, event: &div::Event| {
        event.key_text == "z"
    }));
    let (mut view, _document) = temp_view(
        "",
        keymap(vec![KeyBinding {
            any: Some(any),
            ..KeyBinding::default()
        }]),
    );
    assert!(run(&mut view, &key("z", 90, 0)));
    assert!(!run(&mut view, &key("y", 89, 0)));
}

#[test]
fn runs_multi_stroke_bindings() {
    let log = Log::default();
    let (mut view, _document) = temp_view(
        "",
        keymap(vec![
            binding("Mod-k Mod-c", log.command("comment", true)),
            binding("Mod-c", log.command("copy", true)),
        ]),
    );
    // The prefix is handled and waits for the next key.
    assert!(run(&mut view, &key("k", 75, MOD)));
    assert!(log.take().is_empty());
    // A modifier key alone keeps it waiting.
    assert!(!run(&mut view, &key("Control", 17, CTRL)));
    assert!(run(&mut view, &key("c", 67, MOD)));
    assert_eq!(log.take(), ["comment"]);
    // Without the prefix, the key has its own binding.
    assert!(run(&mut view, &key("c", 67, MOD)));
    assert_eq!(log.take(), ["copy"]);
}

#[test]
fn an_unbound_second_stroke_is_swallowed() {
    let log = Log::default();
    let (mut view, _document) = temp_view(
        "",
        keymap(vec![
            binding("Mod-k Mod-c", log.command("comment", true)),
            binding("x", log.command("x", true)),
        ]),
    );
    assert!(run(&mut view, &key("k", 75, MOD)));
    // `x` after the prefix names `Mod-k x`, which has no binding; the key
    // is still handled, and the prefix is gone.
    assert!(run(&mut view, &key("x", 88, 0)));
    assert!(log.take().is_empty());
    assert!(run(&mut view, &key("x", 88, 0)));
    assert_eq!(log.take(), ["x"]);
}

#[test]
fn a_prefix_belongs_to_its_view_and_scope() {
    let log = Log::default();
    let bindings = keymap(vec![
        binding("Mod-k Mod-c", log.command("comment", true)),
        KeyBinding {
            key: Some("Mod-k Mod-c".into()),
            run: Some(log.command("panel-comment", true)),
            scope: Some("panel".into()),
            ..KeyBinding::default()
        },
    ]);
    let (mut first, _first_document) = temp_view("", bindings.clone());
    let (mut second, _second_document) = temp_view("", bindings);
    assert!(run(&mut first, &key("k", 75, MOD)));
    // The other view, and the other scope, have no prefix.
    assert!(!run(&mut second, &key("c", 67, MOD)));
    assert!(!run_scope_handlers(&mut first, &key("c", 67, MOD), "panel"));
    assert!(log.take().is_empty());
    assert!(run(&mut first, &key("c", 67, MOD)));
    assert_eq!(log.take(), ["comment"]);
}

#[test]
fn bindings_apply_in_their_scopes() {
    let log = Log::default();
    let (mut view, _document) = temp_view(
        "",
        keymap(vec![
            KeyBinding {
                key: Some("Escape".into()),
                run: Some(log.command("panel", true)),
                scope: Some("panel".into()),
                ..KeyBinding::default()
            },
            KeyBinding {
                key: Some("Enter".into()),
                run: Some(log.command("both", true)),
                scope: Some("editor panel".into()),
                ..KeyBinding::default()
            },
        ]),
    );
    assert!(!run(&mut view, &key("Escape", 27, 0)));
    assert!(run_scope_handlers(
        &mut view,
        &key("Escape", 27, 0),
        "panel"
    ));
    assert!(run(&mut view, &key("Enter", 13, 0)));
    assert!(run_scope_handlers(&mut view, &key("Enter", 13, 0), "panel"));
    assert!(!run_scope_handlers(
        &mut view,
        &key("Enter", 13, 0),
        "search"
    ));
    assert_eq!(log.take(), ["panel", "both", "both"]);
}

#[test]
fn a_platform_key_takes_precedence() {
    let log = Log::default();
    let (mut view, _document) = temp_view(
        "",
        keymap(vec![
            KeyBinding {
                key: Some("F5".into()),
                mac: Some("F6".into()),
                win: Some("F6".into()),
                linux: Some("F6".into()),
                run: Some(log.command("platform", true)),
                ..KeyBinding::default()
            },
            // Without a key name, a binding is ignored.
            KeyBinding {
                run: Some(log.command("none", true)),
                ..KeyBinding::default()
            },
        ]),
    );
    assert!(!run(&mut view, &key("F5", 116, 0)));
    assert!(run(&mut view, &key("F6", 117, 0)));
    assert_eq!(log.take(), ["platform"]);
}

#[test]
fn ctrl_alt_characters_are_typed_on_windows() {
    let log = Log::default();
    let (mut view, _document) = temp_view(
        "",
        keymap(vec![binding("Ctrl-Alt-2", log.command("ctrl-alt-2", true))]),
    );
    // AltGr-2 types "@" on some layouts, and arrives as Ctrl-Alt.
    let handled = run(&mut view, &key("@", 50, CTRL | ALT));
    assert_eq!(handled, !cfg!(windows));
}

#[test]
#[should_panic(expected = "Unrecognized modifier name: Hyper")]
fn rejects_unknown_modifiers() {
    let log = Log::default();
    let (mut view, _document) =
        temp_view("", keymap(vec![binding("Hyper-a", log.command("a", true))]));
    run(&mut view, &key("a", 65, 0));
}

#[test]
#[should_panic(expected = "is used both as a regular binding and as a multi-stroke prefix")]
fn rejects_a_binding_that_is_also_a_prefix() {
    let log = Log::default();
    let (mut view, _document) = temp_view(
        "",
        keymap(vec![
            binding("Mod-k", log.command("k", true)),
            binding("Mod-k Mod-c", log.command("comment", true)),
        ]),
    );
    run(&mut view, &key("k", 75, MOD));
}

// A state command that inserts "!" at the start of the document.
fn insert_bang(state: &EditorState, dispatch: &mut dyn FnMut(Transaction)) -> bool {
    let tr = state
        .update(TransactionSpec {
            changes: Some(ChangeSpec::insert(0, "!")),
            ..TransactionSpec::default()
        })
        .unwrap();
    dispatch(tr);
    true
}

#[test]
fn runs_state_commands_on_the_view() {
    let (mut view, _document) = temp_view(
        "text",
        keymap(vec![binding("Mod-1", state_command(insert_bang))]),
    );
    assert!(run(&mut view, &key("1", 49, MOD)));
    assert_eq!(view.state().doc().to_string(), "!text");
}
