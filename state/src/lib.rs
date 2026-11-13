//! A Rust port of [`@codemirror/state`](https://codemirror.net/docs/ref/#state)
//! 6.7.6, the data layer of CodeMirror 6.
//!
//! This crate has no Div dependency. It keeps CodeMirror's data structures
//! and algorithms so the editor built on it behaves like the package, and so
//! CodeMirror's own tests can check it. The port so far covers:
//!
//! - [`Text`]: the document, a balanced tree of lines (`text.ts`);
//! - [`ChangeDesc`] and [`ChangeSet`]: document changes, with mapping,
//!   composition and inversion (`change.ts`);
//! - [`EditorSelection`] and [`SelectionRange`]: multiple selection ranges
//!   (`selection.ts`).
//!
//! # Positions
//!
//! CodeMirror counts positions in UTF-16 code units, as JavaScript strings
//! do. This port counts them in UTF-8 bytes, as Rust strings do, so a
//! position is a byte offset into the document with `\n` between lines.
//! A position must fall on a character boundary; one that does not panics,
//! as slicing a `str` does. Fidelity checks against the package in Chromium
//! convert positions at the boundary. Every other behaviour follows the
//! upstream source.
//!
//! # Errors
//!
//! Where CodeMirror throws a `RangeError` for input it validates
//! (`Text.of`, `ChangeSet.of`, `EditorSelection.create`, the JSON readers),
//! this port returns [`RangeError`]. Where it throws for a broken
//! precondition between already-built values (a position outside the
//! document, change sets of different lengths), this port panics with
//! CodeMirror's message, as Rust's own slicing does.

mod change;
mod selection;
mod text;

pub use change::{
    Change, ChangeDesc, ChangeJson, ChangeSet, ChangeSpec, Insertion, MapMode, Touch,
};
pub use selection::{EditorSelection, SelectionRange};
pub use text::{Line, LineCursor, PartialTextCursor, Text, TextCursor, TextIterator};

/// Invalid input to a constructor, with CodeMirror's `RangeError` message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeError(pub String);

impl std::fmt::Display for RangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RangeError {}
