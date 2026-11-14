//! A Rust port of [`@codemirror/view`](https://codemirror.net/docs/ref/#view)
//! 6.43.15, the view layer of CodeMirror 6, on Div's DOM.
//!
//! The port so far covers:
//!
//! - [`Decoration`], [`WidgetType`] and [`BlockWrapper`]: how content is
//!   styled, replaced and wrapped (`decoration.ts`), with the attribute
//!   sets they carry ([`Attrs`], `attributes.ts`);
//! - the tile tree that draws the content on Div's DOM (`tile.ts`,
//!   `buildtile.ts`): `.cm-line` elements with their text, marks, widgets
//!   and block wrappers, redrawn only where a change or a decoration
//!   change touches them. It is driven by the part of `DocView` that
//!   draws, in [`__test`] until the editor view that owns it is ported;
//! - the height map, which estimates and records the heights of lines and
//!   block widgets so the view can place content it has not drawn, find
//!   the viewport and size the gap spacers (`heightmap.ts`): [`BlockInfo`]
//!   is public as upstream, and the map itself is in [`__test`], as
//!   upstream exports it for its tests;
//! - [`ChangedRange`] (`extension.ts`) and [`Direction`] (`bidi.ts`).
//!
//! The `EditorView` itself (`.cm-editor`, `.cm-scroller`, `.cm-content`),
//! the viewport, gutters and the theme come in the next parts of milestone
//! M6, with the widget methods that need them (`ignoreEvent`, `coordsAt`).
//!
//! Div lacks some DOM features the content needs; the code that stands in
//! for them is marked `div-gap` with the gap's number in the sample's
//! design document.
//!
//! Positions are UTF-8 byte offsets, as in `editor-state`.

mod attributes;
mod bidi;
mod buildtile;
mod decoration;
mod docview;
mod dom;
mod extension;
mod heightmap;
mod tile;

pub use attributes::{attrs_eq, combine_attrs, Attrs};
pub use bidi::Direction;
pub use decoration::{
    BlockType, BlockWrapper, BlockWrapperSpec, Decoration, DecorationKind, DecorationSet,
    DecorationSpec, LineSpec, MarkSpec, ReplaceSpec, SpecData, Widget, WidgetSpec, WidgetType,
};
pub use extension::ChangedRange;
pub use heightmap::{BlockInfo, BlockKind};

/// The view's internals that upstream exports for its tests (`__test`):
/// the height map and its oracle. Also the content drawing that the
/// editor view will own.
#[doc(hidden)]
pub mod __test {
    pub use crate::buildtile::BuildError;
    pub use crate::decoration::add_range;
    pub use crate::docview::ContentTiles;
    pub use crate::heightmap::{
        clear_height_change_flag, height_change_flag, height_relevant_deco_changes, HeightMap,
        HeightOracle, MeasuredHeights, QueryType,
    };
}
