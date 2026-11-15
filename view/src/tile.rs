//! Port of `tile.ts`: the tree of tiles that describes the drawn content.
//!
//! The view keeps a tree of tiles, each of which stands for a DOM node
//! with a meaning in the content: the document (`.cm-content`), block
//! wrappers, lines, marks, text and widgets. It maps between document and
//! DOM positions and lets updates redraw only what changed.
//!
//! Upstream's tiles are objects that point at each other and at their DOM
//! nodes. Here they live in an arena, [`Tiles`], and refer to each other
//! by [`TileId`]; an update moves the tiles it keeps from the old tree to
//! the new one, as upstream does. The DOM-to-tile lookup (`Tile.get`), the
//! position and coordinate queries that need the DOM (`domPosFor`,
//! `coordsIn`, `resolveInline`, `domIn`) and `markDirty` come with the
//! view's DOM reading and measurement.

use std::sync::Arc;

use div::{Document, Element};

use crate::attributes::Attrs;
use crate::buildtile::BuildError;
use crate::decoration::{BlockWrapper, Decoration, DecorationKind, Widget};
use crate::dom::{self, DomNode};

/// Tile flags (`TileFlag`).
pub(crate) mod flag {
    /// There is a line break, one position long, after the tile.
    pub const BREAK_AFTER: u32 = 1;
    /// The tile's DOM is up to date.
    pub const SYNCED: u32 = 2;
    /// The tile's DOM attributes must be written.
    pub const ATTRS_DIRTY: u32 = 4;
    /// A text tile of an active composition, which is not merged.
    pub const COMPOSITION: u32 = 8;
    /// A point widget before the cursor position.
    pub const BEFORE: u32 = 16;
    /// A point widget after the cursor position.
    pub const AFTER: u32 = 32;
    pub const POINT_WIDGET: u32 = BEFORE | AFTER;
    /// A replace widget that includes its start.
    pub const INC_START: u32 = 64;
    /// A replace widget that includes its end.
    pub const INC_END: u32 = 128;
    /// A block widget.
    pub const BLOCK: u32 = 256;
    pub const WIDGET: u32 = BEFORE | AFTER | INC_START | INC_END | BLOCK;
}

use flag::*;

/// A tile in a [`Tiles`] arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TileId(usize);

/// What a tile stands for: upstream's tile classes.
pub(crate) enum TileKind {
    /// The document (`DocTile`), drawn into the content element.
    Doc,
    /// A block wrapper (`BlockWrapperTile`).
    BlockWrapper(Arc<BlockWrapper>),
    /// A line (`LineTile`) and its attributes.
    Line(Attrs),
    /// A mark decoration's element (`MarkTile`).
    Mark(Arc<Decoration>),
    /// Text (`TextTile`).
    Text(String),
    /// A widget (`WidgetTile`).
    Widget(Widget),
    /// An image drawn beside an uneditable widget (`WidgetBufferTile`).
    WidgetBuffer,
}

pub(crate) struct Tile {
    pub(crate) kind: TileKind,
    pub(crate) dom: DomNode,
    pub(crate) length: usize,
    pub(crate) flags: u32,
    pub(crate) parent: Option<TileId>,
    pub(crate) children: Vec<TileId>,
    /// The attributes `set_attrs` last gave `dom` (div-gap 23).
    pub(crate) written_attrs: Attrs,
}

/// A DOM node taken over from another tile, with the attributes written
/// to it.
pub(crate) struct Adopted {
    pub(crate) dom: DomNode,
    pub(crate) written_attrs: Attrs,
}

/// The tile arena.
#[derive(Default)]
pub(crate) struct Tiles {
    slots: Vec<Option<Tile>>,
    free: Vec<usize>,
}

impl std::ops::Index<TileId> for Tiles {
    type Output = Tile;

    fn index(&self, id: TileId) -> &Tile {
        self.slots[id.0].as_ref().expect("released tile")
    }
}

impl std::ops::IndexMut<TileId> for Tiles {
    fn index_mut(&mut self, id: TileId) -> &mut Tile {
        self.slots[id.0].as_mut().expect("released tile")
    }
}

fn element_of(dom: Option<Adopted>) -> (Option<Element>, Attrs) {
    match dom {
        Some(Adopted {
            dom: DomNode::Element(element),
            written_attrs,
        }) => (Some(element), written_attrs),
        _ => (None, Attrs::new()),
    }
}

impl Tiles {
    fn alloc(&mut self, kind: TileKind, dom: DomNode, length: usize, flags: u32) -> TileId {
        self.alloc_with(kind, dom, length, flags, Attrs::new())
    }

    fn alloc_with(
        &mut self,
        kind: TileKind,
        dom: DomNode,
        length: usize,
        flags: u32,
        written_attrs: Attrs,
    ) -> TileId {
        let tile = Tile {
            kind,
            dom,
            length,
            flags,
            parent: None,
            children: Vec::new(),
            written_attrs,
        };
        match self.free.pop() {
            Some(index) => {
                self.slots[index] = Some(tile);
                TileId(index)
            }
            None => {
                self.slots.push(Some(tile));
                TileId(self.slots.len() - 1)
            }
        }
    }

    /// Free a tile's slot. Its DOM is left alone.
    pub(crate) fn release(&mut self, id: TileId) {
        self.slots[id.0] = None;
        self.free.push(id.0);
    }

    /// The number of live tiles.
    pub(crate) fn live(&self) -> usize {
        self.slots.iter().filter(|slot| slot.is_some()).count()
    }

    /// The DOM and written attributes of `id`, for another tile to take.
    pub(crate) fn adopt(&self, id: TileId) -> Adopted {
        let tile = &self[id];
        Adopted {
            dom: tile.dom.clone(),
            written_attrs: tile.written_attrs.clone(),
        }
    }

    /// `new DocTile(view, dom)`.
    pub(crate) fn new_doc(&mut self, dom: Element) -> TileId {
        self.alloc(TileKind::Doc, DomNode::Element(dom), 0, 0)
    }

    /// `BlockWrapperTile.of`.
    pub(crate) fn block_wrapper_of(
        &mut self,
        document: &Document,
        wrapper: &Arc<BlockWrapper>,
        dom: Option<Adopted>,
    ) -> Result<TileId, div::Error> {
        let (dom, written) = element_of(dom);
        let flags = if dom.is_none() { ATTRS_DIRTY } else { 0 };
        let dom = match dom {
            Some(dom) => dom,
            None => dom::create_element(document, wrapper.tag_name())?,
        };
        Ok(self.alloc_with(
            TileKind::BlockWrapper(wrapper.clone()),
            DomNode::Element(dom),
            0,
            flags,
            written,
        ))
    }

    /// `LineTile.start`.
    pub(crate) fn line_start(
        &mut self,
        document: &Document,
        attrs: Attrs,
        dom: Option<Adopted>,
        keep_attrs: bool,
    ) -> Result<TileId, div::Error> {
        let (dom, written) = element_of(dom);
        let flags = if dom.is_none() || !keep_attrs {
            ATTRS_DIRTY
        } else {
            0
        };
        let dom = match dom {
            Some(dom) => dom,
            None => dom::create_element(document, "div")?,
        };
        Ok(self.alloc_with(
            TileKind::Line(attrs),
            DomNode::Element(dom),
            0,
            flags,
            written,
        ))
    }

    /// `MarkTile.of`.
    pub(crate) fn mark_of(
        &mut self,
        document: &Document,
        mark: &Arc<Decoration>,
        dom: Option<Adopted>,
    ) -> Result<TileId, div::Error> {
        let (dom, written) = element_of(dom);
        let flags = if dom.is_none() { ATTRS_DIRTY } else { 0 };
        let dom = match dom {
            Some(dom) => dom,
            None => {
                let DecorationKind::Mark { tag_name, .. } = mark.kind() else {
                    panic!("MarkTile of a decoration that is not a mark");
                };
                dom::create_element(document, tag_name)?
            }
        };
        Ok(self.alloc_with(
            TileKind::Mark(mark.clone()),
            DomNode::Element(dom),
            0,
            flags,
            written,
        ))
    }

    /// `TextTile.of`.
    pub(crate) fn text_of(
        &mut self,
        document: &Document,
        text: &str,
        dom: Option<Adopted>,
    ) -> Result<TileId, div::Error> {
        let (dom, flags) = match dom {
            Some(Adopted {
                dom: dom @ DomNode::Text(_),
                ..
            }) => (dom, 0),
            _ => (DomNode::Text(dom::create_text(document, text)?), SYNCED),
        };
        Ok(self.alloc(TileKind::Text(text.to_string()), dom, text.len(), flags))
    }

    /// `new WidgetTile(dom, length, widget, flags)`.
    pub(crate) fn widget_tile(
        &mut self,
        dom: Element,
        length: usize,
        widget: Widget,
        flags: u32,
    ) -> TileId {
        self.alloc(
            TileKind::Widget(widget),
            DomNode::Element(dom),
            length,
            flags,
        )
    }

    /// `WidgetTile.of`.
    pub(crate) fn widget_of(
        &mut self,
        document: &Document,
        widget: &Widget,
        length: usize,
        flags: u32,
        dom: Option<Adopted>,
    ) -> Result<TileId, div::Error> {
        let dom = match element_of(dom).0 {
            Some(dom) => dom,
            None => {
                let dom = widget.to_dom(document)?;
                if !widget.editable() {
                    dom::set_attribute(&dom, "contenteditable", "false")?;
                }
                dom
            }
        };
        Ok(self.widget_tile(dom, length, widget.clone(), flags))
    }

    /// `new WidgetBufferTile(flags)`.
    pub(crate) fn widget_buffer(
        &mut self,
        document: &Document,
        flags: u32,
    ) -> Result<TileId, div::Error> {
        let img = dom::create_element(document, "img")?;
        img.set_attribute("class", "cm-widgetBuffer")?;
        img.set_attribute("aria-hidden", "true")?;
        Ok(self.alloc(TileKind::WidgetBuffer, DomNode::Element(img), 0, flags))
    }

    /// `isComposite`.
    pub(crate) fn is_composite(&self, id: TileId) -> bool {
        matches!(
            self[id].kind,
            TileKind::Doc | TileKind::BlockWrapper(_) | TileKind::Line(_) | TileKind::Mark(_)
        )
    }

    /// `isWidget`.
    pub(crate) fn is_widget(&self, id: TileId) -> bool {
        matches!(self[id].kind, TileKind::Widget(_))
    }

    /// `isLine`.
    pub(crate) fn is_line(&self, id: TileId) -> bool {
        matches!(self[id].kind, TileKind::Line(_))
    }

    /// `isText`.
    pub(crate) fn is_text(&self, id: TileId) -> bool {
        matches!(self[id].kind, TileKind::Text(_))
    }

    /// `breakAfter`: 1 when a line break follows the tile.
    pub(crate) fn break_after(&self, id: TileId) -> usize {
        (self[id].flags & BREAK_AFTER) as usize
    }

    /// `lastChild`.
    pub(crate) fn last_child(&self, id: TileId) -> Option<TileId> {
        self[id].children.last().copied()
    }

    /// `append`.
    pub(crate) fn append(&mut self, parent: TileId, child: TileId) {
        self[parent].children.push(child);
        self[child].parent = Some(parent);
    }

    /// The mark of a mark tile.
    pub(crate) fn mark(&self, id: TileId) -> Option<&Arc<Decoration>> {
        match &self[id].kind {
            TileKind::Mark(mark) => Some(mark),
            _ => None,
        }
    }

    /// The wrapper of a block wrapper tile.
    pub(crate) fn wrapper(&self, id: TileId) -> Option<&Arc<BlockWrapper>> {
        match &self[id].kind {
            TileKind::BlockWrapper(wrapper) => Some(wrapper),
            _ => None,
        }
    }

    /// The widget of a widget tile.
    pub(crate) fn widget(&self, id: TileId) -> Option<&Widget> {
        match &self[id].kind {
            TileKind::Widget(widget) => Some(widget),
            _ => None,
        }
    }

    /// `domAttrs`.
    fn dom_attrs(&self, id: TileId) -> Option<&Attrs> {
        match &self[id].kind {
            TileKind::BlockWrapper(wrapper) => Some(wrapper.attributes()),
            TileKind::Line(attrs) => Some(attrs),
            TileKind::Mark(mark) => match mark.kind() {
                DecorationKind::Mark { attrs, .. } => Some(attrs),
                _ => None,
            },
            _ => None,
        }
    }

    /// `covers`: whether the tile includes its start (`side < 0`) or end.
    pub(crate) fn covers(&self, id: TileId, side: i8) -> bool {
        let tile = &self[id];
        match &tile.kind {
            TileKind::BlockWrapper(_) => match (tile.children.first(), tile.children.last()) {
                (Some(&first), Some(&last)) => {
                    if side < 0 {
                        self.covers(first, -1)
                    } else {
                        self.covers(last, 1)
                    }
                }
                _ => false,
            },
            TileKind::Widget(_) => {
                if tile.flags & POINT_WIDGET != 0 {
                    return false;
                }
                tile.flags & (if side < 0 { INC_START } else { INC_END }) > 0
            }
            _ => true,
        }
    }

    /// Bring the DOM of `id` and its descendants up to date (`sync`).
    pub(crate) fn sync(&mut self, id: TileId) -> Result<(), div::Error> {
        if self.is_composite(id) {
            if self[id].flags & SYNCED != 0 {
                return Ok(());
            }
            self.sync_attrs(id)?;
            let children = self[id].children.clone();
            let mut length = 0;
            let mut doms = Vec::with_capacity(children.len());
            for &child in &children {
                self.sync(child)?;
                length += self[child].length + self.break_after(child);
                doms.push(self[child].dom.clone());
            }
            let parent = self[id]
                .dom
                .element()
                .expect("a composite tile draws an element")
                .clone();
            dom::sync_children(&parent, &doms)?;
            self[id].length = length;
        } else if let TileKind::Text(text) = &self[id].kind {
            if self[id].flags & SYNCED != 0 {
                return Ok(());
            }
            let node = self[id].dom.text_node()?;
            if node.data()? != *text {
                node.set_data(text)?;
            }
            self.sync_attrs(id)?;
        } else {
            self.sync_attrs(id)?;
        }
        Ok(())
    }

    // `Tile.sync`: write the attributes when they are dirty.
    fn sync_attrs(&mut self, id: TileId) -> Result<(), div::Error> {
        self[id].flags |= SYNCED;
        if self[id].flags & ATTRS_DIRTY != 0 {
            self[id].flags &= !ATTRS_DIRTY;
            if let Some(attrs) = self.dom_attrs(id).cloned() {
                let tile = &self[id];
                let element = tile.dom.element().expect("attributes go on an element");
                dom::set_attrs(element, &attrs, &tile.written_attrs)?;
                self[id].written_attrs = attrs;
            }
        }
        Ok(())
    }

    /// `destroy`: let a widget clean up its DOM.
    pub(crate) fn destroy(&mut self, id: TileId) {
        self[id].parent = None;
        if let (TileKind::Widget(widget), Some(dom)) = (&self[id].kind, self[id].dom.element()) {
            widget.destroy(dom);
        }
    }

    /// Call `f` for each line and block widget below `root`, with its
    /// position, until it returns a value (`DocTile.blockTiles`).
    pub(crate) fn block_tiles<T>(
        &self,
        root: TileId,
        mut f: impl FnMut(TileId, usize) -> Option<T>,
    ) -> Option<T> {
        let mut stack = Vec::new();
        let (mut cur, mut i, mut pos) = (root, 0, 0);
        loop {
            if i == self[cur].children.len() {
                let index = stack.pop()?;
                cur = self[cur].parent.expect("a nested tile has a parent");
                // As upstream, this looks at the parent the walk returns to.
                pos += self.break_after(cur);
                i = index;
            } else {
                let next = self[cur].children[i];
                i += 1;
                if matches!(self[next].kind, TileKind::BlockWrapper(_)) {
                    stack.push(i);
                    cur = next;
                    i = 0;
                } else {
                    let end = pos + self[next].length;
                    if let Some(result) = f(next, pos) {
                        return Some(result);
                    }
                    pos = end + self.break_after(next);
                }
            }
        }
    }

    /// The block at `pos` and the offset in it (`DocTile.resolveBlock`).
    /// A `side` below -1 stays before block widgets at `pos`, above 1
    /// after them.
    ///
    /// # Panics
    ///
    /// When no block covers `pos`.
    pub(crate) fn resolve_block(&self, root: TileId, pos: usize, side: i32) -> (TileId, usize) {
        let mut before: Option<(TileId, usize)> = None;
        let mut after: Option<(TileId, usize)> = None;
        self.block_tiles(root, |tile, off| {
            let length = self[tile].length;
            let end = off + length;
            if pos >= off && pos <= end {
                if self.is_widget(tile) && (-1..=1).contains(&side) {
                    if self[tile].flags & AFTER != 0 {
                        return Some(());
                    }
                    if self[tile].flags & BEFORE != 0 {
                        before = None;
                    }
                }
                let better = |current: Option<(TileId, usize)>| {
                    current
                        .is_none_or(|(current, _)| !self.is_widget(tile) && self.is_widget(current))
                };
                if (off < pos
                    || pos == end
                        && (if side < -1 {
                            length > 0
                        } else {
                            self.covers(tile, 1)
                        }))
                    && better(before)
                {
                    before = Some((tile, pos - off));
                }
                if (end > pos
                    || pos == off
                        && (if side > 1 {
                            length > 0
                        } else {
                            self.covers(tile, -1)
                        }))
                    && better(after)
                {
                    after = Some((tile, pos - off));
                }
            }
            None
        });
        match (before, after) {
            (Some(before), _) if side < 0 => before,
            (Some(before), None) => before,
            (_, Some(after)) => after,
            (None, None) => panic!("No tile at position {pos}"),
        }
    }

    /// Upstream's `toString`: the tile classes, with children in
    /// parentheses, `#` for a line break after, and text in quotes.
    pub(crate) fn describe(&self, id: TileId, out: &mut String) {
        let tile = &self[id];
        let name = match &tile.kind {
            TileKind::Text(text) => {
                json_string(text, out);
                return;
            }
            TileKind::Doc => "DocTile",
            TileKind::BlockWrapper(_) => "BlockWrapperTile",
            TileKind::Line(_) => "LineTile",
            TileKind::Mark(_) => "MarkTile",
            TileKind::Widget(_) => "WidgetTile",
            TileKind::WidgetBuffer => "WidgetBufferTile",
        };
        out.push_str(name);
        if !tile.children.is_empty() {
            out.push('(');
            for (i, &child) in tile.children.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                self.describe(child, out);
            }
            out.push(')');
        }
        if tile.flags & BREAK_AFTER != 0 {
            out.push('#');
        }
    }
}

// `JSON.stringify` for a string.
fn json_string(text: &str, out: &mut String) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
}

/// Visits tiles as a [`TilePointer`] moves (`TileWalker`).
pub(crate) trait TileWalker {
    /// The pointer enters a composite tile.
    fn enter(&mut self, tiles: &mut Tiles, tile: TileId) -> Result<(), BuildError>;
    /// The pointer leaves a composite tile.
    fn leave(&mut self, tiles: &mut Tiles, tile: TileId);
    /// The pointer moves over `from..to` of a tile. Returning `false` for a
    /// composite tile makes the pointer enter it instead.
    fn skip(
        &mut self,
        tiles: &mut Tiles,
        tile: TileId,
        from: usize,
        to: usize,
    ) -> Result<bool, BuildError>;
    /// The pointer moves over a line break.
    fn line_break(&mut self, tiles: &mut Tiles) -> Result<(), BuildError>;
}

/// A position in a tile tree (`TilePointer`).
pub(crate) struct TilePointer {
    pub(crate) tile: TileId,
    pub(crate) index: usize,
    pub(crate) before_break: bool,
    pub(crate) parents: Vec<(TileId, usize)>,
}

impl TilePointer {
    pub(crate) fn new(top: TileId) -> TilePointer {
        TilePointer {
            tile: top,
            index: 0,
            before_break: false,
            parents: Vec::new(),
        }
    }

    /// Move `dist` positions forward (`advance`). With `side` -1, stop
    /// leaving, entering or skipping zero-length tiles once the distance
    /// is covered; with 1, do all of that at the end position.
    pub(crate) fn advance(
        &mut self,
        tiles: &mut Tiles,
        mut dist: usize,
        side: i8,
        mut walker: Option<&mut dyn TileWalker>,
    ) -> Result<(), BuildError> {
        let (mut tile, mut index, mut before_break) = (self.tile, self.index, self.before_break);
        let parents = &mut self.parents;
        while dist > 0 || side > 0 {
            if !tiles.is_composite(tile) {
                let len = tiles[tile].length;
                if index < len && dist > 0 {
                    let take = dist.min(len - index);
                    if let Some(walker) = &mut walker {
                        walker.skip(tiles, tile, index, index + take)?;
                    }
                    dist -= take;
                    index += take;
                }
                if index == len {
                    before_break = tiles.break_after(tile) != 0;
                    (tile, index) = parents.pop().expect("a leaf tile has a parent");
                    index += 1;
                } else if dist == 0 {
                    break;
                }
            } else if before_break {
                if dist == 0 {
                    break;
                }
                if let Some(walker) = &mut walker {
                    walker.line_break(tiles)?;
                }
                dist -= 1;
                before_break = false;
            } else if index == tiles[tile].children.len() {
                if dist == 0 && parents.is_empty() {
                    break;
                }
                if let Some(walker) = &mut walker {
                    walker.leave(tiles, tile);
                }
                before_break = tiles.break_after(tile) != 0;
                (tile, index) = parents.pop().expect("advanced past the end of the tree");
                index += 1;
            } else {
                let next = tiles[tile].children[index];
                let brk = tiles.break_after(next) != 0;
                let length = tiles[next].length;
                let fits = if side > 0 {
                    length <= dist
                } else {
                    length < dist
                };
                // Upstream's `|| !next.isComposite` tests a method, which is
                // always defined, so only the walker's answer counts.
                let skip = fits
                    && match &mut walker {
                        Some(walker) => walker.skip(tiles, next, 0, length)?,
                        None => true,
                    };
                if skip {
                    before_break = brk;
                    index += 1;
                    dist -= length;
                } else {
                    parents.push((tile, index));
                    tile = next;
                    index = 0;
                    if tiles.is_composite(next) {
                        if let Some(walker) = &mut walker {
                            walker.enter(tiles, next)?;
                        }
                    }
                }
            }
        }
        self.tile = tile;
        self.index = index;
        self.before_break = before_break;
        Ok(())
    }
}
