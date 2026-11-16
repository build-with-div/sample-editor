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
//! - [`ChangedRange`] (`extension.ts`) and [`Direction`] (`bidi.ts`);
//! - [`EditorView`] (`editorview.ts`): `.cm-editor`, `.cm-scroller` and
//!   `.cm-content`, updated from transactions, with the attributes the
//!   facets give and the measure cycle; [`Viewport`] and the view state
//!   behind it (`viewstate.ts`), with gap spacers for what is not drawn;
//! - themes (`theme.ts`) and [`StyleModule`], a port of `style-mod`, with
//!   the facets, [`ViewUpdate`] and [`ScrollTarget`] of `extension.ts`;
//! - view plugins ([`ViewPlugin`], `extension.ts`) and gutters
//!   ([`gutter`], [`line_numbers`], `gutter.ts`);
//! - key bindings ([`KeyBinding`], [`KEYMAP`], [`run_scope_handlers`],
//!   `keymap.ts`), with the key names of a port of `w3c-keyname`
//!   ([`keyname`]).
//!
//! - DOM event handling (`input.ts`): the handlers and observers plugins
//!   and [`EditorView::dom_event_handlers`] register run for the events on
//!   the content of a view made with [`EditorView::new_shared`], the key
//!   bindings among them.
//!
//! Focus, mouse selection, the clipboard, composition and the coordinate
//! queries come later in milestone M7.
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
mod editorview;
mod extension;
mod gutter;
mod heightmap;
mod input;
mod keymap;
pub mod keyname;
mod plugin;
mod style_mod;
mod theme;
mod tile;
mod viewstate;

pub use attributes::{attrs_eq, combine_attrs, Attrs};
pub use bidi::Direction;
pub use buildtile::BuildError;
pub use decoration::{
    BlockType, BlockWrapper, BlockWrapperSpec, Decoration, DecorationKind, DecorationSet,
    DecorationSpec, LineSpec, MarkSpec, ReplaceSpec, SpecData, Widget, WidgetSpec, WidgetType,
};
pub use dom::{DomNode, Rect, WindowSize};
pub use editorview::{EditorView, EditorViewConfig, UpdateListener};
pub use extension::{
    update_flag, ChangedRange, Margins, ScrollMarginSource, ScrollStrategy, ScrollTarget,
    ViewUpdate,
};
pub use gutter::{
    gutter, gutters, highlight_active_line_gutter, line_numbers, FormatNumber, GutterConfig,
    GutterMarker, GutterMarkerType, GutterSide, GuttersConfig, InitialSpacer, LineMarker,
    LineMarkerChange, LineNumberConfig, MarkerSource, UpdateSpacer, WidgetMarker,
    GUTTER_LINE_CLASS, GUTTER_WIDGET_CLASS, LINE_NUMBER_MARKERS, LINE_NUMBER_WIDGET_MARKER,
};
pub use heightmap::{BlockInfo, BlockKind};
pub use input::TabFocusMode;
pub use keymap::{
    command, run_scope_handlers, state_command, AnyKeyHandler, Command, KeyBinding, KeyBindings,
    Keymaps, KEYMAP,
};
pub use plugin::{
    dom_event_handler, DomEventHandler, DomEventHandlers, PluginSpec, PluginValue, Provide,
    ViewPlugin,
};
pub use style_mod::{StyleModule, StyleSpec, StyleSpecValue};
pub use viewstate::Viewport;

/// The view's internals that upstream exports for its tests (`__test`):
/// the height map and its oracle. Also the content drawing that the
/// editor view will own.
#[doc(hidden)]
pub mod __test {
    pub use crate::decoration::add_range;
    pub use crate::docview::ContentTiles;
    pub use crate::dom::set_style;
    pub use crate::heightmap::{
        clear_height_change_flag, height_change_flag, height_relevant_deco_changes, HeightMap,
        HeightOracle, MeasuredHeights, QueryType,
    };
}
