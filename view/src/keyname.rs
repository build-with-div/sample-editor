//! A port of [`w3c-keyname`](https://github.com/marijnh/w3c-keyname) 2.2.8
//! (MIT), which names the key of a keyboard event as the DOM's
//! `KeyboardEvent.key` does, falling back on its `keyCode` where the key is
//! missing or unreliable.
//!
//! Div's [`div::Event`] carries what the package reads: `key_text` is
//! `key`, `key_code` is `keyCode`, and `modifiers` holds the four modifier
//! flags.

use std::collections::HashMap;
use std::sync::LazyLock;

/// The modifier flags of a [`div::Event`]'s `modifiers`.
const SHIFT: i32 = 1;
const CTRL: i32 = 2;
const ALT: i32 = 4;
const META: i32 = 8;

pub(crate) fn shift_key(event: &div::Event) -> bool {
    event.modifiers & SHIFT != 0
}

pub(crate) fn ctrl_key(event: &div::Event) -> bool {
    event.modifiers & CTRL != 0
}

pub(crate) fn alt_key(event: &div::Event) -> bool {
    event.modifiers & ALT != 0
}

pub(crate) fn meta_key(event: &div::Event) -> bool {
    event.modifiers & META != 0
}

static BASE: LazyLock<HashMap<i32, String>> = LazyLock::new(|| {
    let mut base: HashMap<i32, String> = [
        (8, "Backspace"),
        (9, "Tab"),
        (10, "Enter"),
        (12, "NumLock"),
        (13, "Enter"),
        (16, "Shift"),
        (17, "Control"),
        (18, "Alt"),
        (20, "CapsLock"),
        (27, "Escape"),
        (32, " "),
        (33, "PageUp"),
        (34, "PageDown"),
        (35, "End"),
        (36, "Home"),
        (37, "ArrowLeft"),
        (38, "ArrowUp"),
        (39, "ArrowRight"),
        (40, "ArrowDown"),
        (44, "PrintScreen"),
        (45, "Insert"),
        (46, "Delete"),
        (59, ";"),
        (61, "="),
        (91, "Meta"),
        (92, "Meta"),
        (106, "*"),
        (107, "+"),
        (108, ","),
        (109, "-"),
        (110, "."),
        (111, "/"),
        (144, "NumLock"),
        (145, "ScrollLock"),
        (160, "Shift"),
        (161, "Shift"),
        (162, "Control"),
        (163, "Control"),
        (164, "Alt"),
        (165, "Alt"),
        (173, "-"),
        (186, ";"),
        (187, "="),
        (188, ","),
        (189, "-"),
        (190, "."),
        (191, "/"),
        (192, "`"),
        (219, "["),
        (220, "\\"),
        (221, "]"),
        (222, "'"),
    ]
    .into_iter()
    .map(|(code, name)| (code, name.to_string()))
    .collect();
    // The digit keys.
    for i in 0..10 {
        base.insert(48 + i, i.to_string());
        base.insert(96 + i, i.to_string());
    }
    // The function keys.
    for i in 1..=24 {
        base.insert(i + 111, format!("F{i}"));
    }
    // The alphabetic keys.
    for i in 65..=90u8 {
        base.insert(i32::from(i), char::from(i + 32).to_string());
    }
    base
});

static SHIFT_NAMES: LazyLock<HashMap<i32, String>> = LazyLock::new(|| {
    let mut shift: HashMap<i32, String> = [
        (48, ")"),
        (49, "!"),
        (50, "@"),
        (51, "#"),
        (52, "$"),
        (53, "%"),
        (54, "^"),
        (55, "&"),
        (56, "*"),
        (57, "("),
        (59, ":"),
        (61, "+"),
        (173, "_"),
        (186, ":"),
        (187, "+"),
        (188, "<"),
        (189, "_"),
        (190, ">"),
        (191, "?"),
        (192, "~"),
        (219, "{"),
        (220, "|"),
        (221, "}"),
        (222, "\""),
    ]
    .into_iter()
    .map(|(code, name)| (code, name.to_string()))
    .collect();
    for i in 65..=90u8 {
        shift.insert(i32::from(i), char::from(i).to_string());
    }
    // Each code without a shifted name has its base name.
    for (code, name) in BASE.iter() {
        shift.entry(*code).or_insert_with(|| name.clone());
    }
    shift
});

/// The name of the key with code `key_code` (`base`), as a US keyboard
/// produces it without Shift.
pub fn base(key_code: i32) -> Option<&'static str> {
    BASE.get(&key_code).map(String::as_str)
}

/// The name of the key with code `key_code` with Shift held (`shift`), as
/// a US keyboard produces it.
pub fn shift(key_code: i32) -> Option<&'static str> {
    SHIFT_NAMES.get(&key_code).map(String::as_str)
}

// `navigator.platform` names a Mac: on macOS, Div runs as Chromium's Mac
// build does.
const MAC: bool = cfg!(target_os = "macos");

/// The name of the key of `event` (`keyName`): its `key`, unless that is
/// missing or unreliable, else the name of its key code.
pub fn key_name(event: &div::Event) -> String {
    // On macOS, keys held with Shift and Cmd don't reflect the effect of
    // Shift in `key`. The package's Internet Explorer case does not apply.
    let ignore_key =
        MAC && meta_key(event) && shift_key(event) && !ctrl_key(event) && !alt_key(event)
            || event.key_text == "Unidentified";
    let key = event.key_text.as_str();
    let table = if shift_key(event) {
        &*SHIFT_NAMES
    } else {
        &*BASE
    };
    let name = if !ignore_key && !key.is_empty() {
        key.to_string()
    } else if let Some(name) = table.get(&event.key_code) {
        name.clone()
    } else if !key.is_empty() {
        key.to_string()
    } else {
        "Unidentified".to_string()
    };
    // Names some browsers produced in the past.
    match name.as_str() {
        "Esc" => "Escape".to_string(),
        "Del" => "Delete".to_string(),
        "Left" => "ArrowLeft".to_string(),
        "Up" => "ArrowUp".to_string(),
        "Right" => "ArrowRight".to_string(),
        "Down" => "ArrowDown".to_string(),
        _ => name,
    }
}
