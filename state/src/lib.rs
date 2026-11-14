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
//!   (`selection.ts`);
//! - [`Facet`], [`StateField`], [`Extension`], [`Prec`] and
//!   [`Compartment`]: the extension system (`facet.ts`);
//! - [`EditorState`] and its built-in [`facets`] (`state.ts`,
//!   `extension.ts`);
//! - [`Transaction`], [`Annotation`] and [`StateEffect`], with change
//!   filters, transaction filters and extenders (`transaction.ts`);
//! - [`find_cluster_break`] and the other character helpers (`char.ts`,
//!   with a port of its dependency `@marijn/find-cluster-break` 1.0.4),
//!   [`count_column`] and [`find_column`] (`column.ts`), and
//!   [`CharCategory`] (`charcategory.ts`).
//!
//! Not yet ported: `RangeSet` (`rangeset.ts`), `EditorState.toJSON` and
//! `fromJSON` with the field serializers, and `combineConfig`, which
//! merges plain JavaScript config objects; the packages that use it are
//! ported with typed config structs.
//!
//! # Values and identity
//!
//! Facet inputs and outputs and field values are typed at their handles
//! ([`Facet`], [`StateField`]) and stored as [`AnyValue`]s. Where
//! CodeMirror compares them with `===`, the port uses `PartialEq`;
//! functions are wrapped in [`Callback`], which compares by identity as
//! JavaScript does. Value equality can treat two equal objects as the same
//! where JavaScript would not, which only skips recomputation. Extension
//! values are deduplicated by identity, as upstream: reuse the same
//! [`Extension`] (it is cheap to clone) to include it once.
//!
//! A field's `provide` extension and a facet's `enables` extension usually
//! refer back to the field or facet, which makes a reference cycle; field
//! and facet definitions are meant to live for the program, as upstream.
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
//! (`Text.of`, `ChangeSet.of`, `EditorSelection.create`, the JSON readers,
//! `EditorState.create`, `EditorState.update`), this port returns
//! [`RangeError`]. Where it throws for a broken
//! precondition between already-built values (a position outside the
//! document, change sets of different lengths, a missing state field, a
//! cyclic dependency between fields and facets), this port panics with
//! CodeMirror's message, as Rust's own slicing does. So does a
//! reconfiguration that uses a compartment twice, which CodeMirror reports
//! when the transaction's new state is computed lazily.

mod change;
mod char;
mod charcategory;
mod cluster;
mod column;
mod facet;
mod selection;
mod state;
mod text;
mod transaction;

pub use change::{
    Change, ChangeDesc, ChangeJson, ChangeSet, ChangeSpec, Insertion, MapMode, Touch,
};
pub use char::{code_point_at, code_point_size, find_cluster_break, from_code_point};
pub use charcategory::CharCategory;
pub use column::{count_column, find_column};
pub use facet::{
    AnyValue, Callback, Compartment, Dep, Extension, Facet, FacetConfig, FacetReader, Prec,
    StateField, StateFieldSpec,
};
pub use selection::{EditorSelection, SelectionRange};
pub use state::{
    facets, EditorState, EditorStateConfig, LanguageData, LanguageDataProvider, RangeChange,
    StateCommand,
};
pub use text::{Line, LineCursor, PartialTextCursor, Text, TextCursor, TextIterator};
pub use transaction::{
    effects, Annotation, AnnotationType, ChangeFilter, ChangeFilterResult, SpecItem, StateEffect,
    StateEffectType, Transaction, TransactionExtender, TransactionExtension, TransactionFilter,
    TransactionSpec,
};

/// Invalid input to a constructor, with CodeMirror's `RangeError` message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeError(pub String);

impl std::fmt::Display for RangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RangeError {}
