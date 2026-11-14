//! A Rust port of [`@codemirror/view`](https://codemirror.net/docs/ref/#view)
//! 6.43.15, the view layer of CodeMirror 6, on Div's DOM.
//!
//! The port so far covers the parts of the view that need no DOM:
//!
//! - [`Decoration`], [`WidgetType`] and [`BlockWrapper`]: how content is
//!   styled, replaced and wrapped (`decoration.ts`), with the attribute
//!   sets they carry ([`Attrs`], `attributes.ts`);
//! - the height map, which estimates and records the heights of lines and
//!   block widgets so the view can place content it has not drawn, find
//!   the viewport and size the gap spacers (`heightmap.ts`): [`BlockInfo`]
//!   is public as upstream, and the map itself is in [`__test`], as
//!   upstream exports it for its tests;
//! - [`ChangedRange`] (`extension.ts`) and [`Direction`] (`bidi.ts`).
//!
//! The `EditorView` itself, its DOM (`.cm-editor`, `.cm-scroller`,
//! `.cm-content`, `.cm-line`), the viewport, gutters and the theme come in
//! the next part of milestone M6, with the widget methods that build DOM
//! (`toDOM`, `updateDOM`, `ignoreEvent`, `coordsAt`, `destroy`).
//!
//! Positions are UTF-8 byte offsets, as in `editor-state`.

mod attributes;
mod bidi;
mod decoration;
mod extension;
mod heightmap;

pub use attributes::{attrs_eq, combine_attrs, Attrs};
pub use bidi::Direction;
pub use decoration::{
    BlockType, BlockWrapper, BlockWrapperSpec, Decoration, DecorationKind, DecorationSet,
    DecorationSpec, LineSpec, MarkSpec, ReplaceSpec, SpecData, Widget, WidgetSpec, WidgetType,
};
pub use extension::ChangedRange;
pub use heightmap::{BlockInfo, BlockKind};

/// The view's internals that upstream exports for its tests (`__test`):
/// the height map and its oracle.
#[doc(hidden)]
pub mod __test {
    pub use crate::decoration::add_range;
    pub use crate::heightmap::{
        clear_height_change_flag, height_change_flag, height_relevant_deco_changes, HeightMap,
        HeightOracle, MeasuredHeights, QueryType,
    };
}
