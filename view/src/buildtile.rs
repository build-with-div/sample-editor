//! Port of `buildtile.ts`: building and updating the tile tree.
//!
//! [`TileUpdate`] walks the document guided by the changed ranges. For
//! unchanged ranges it copies the old tree's tiles into the new tree; for
//! changed ranges it iterates the document text and the decorations to
//! make new tiles, reusing old tiles' DOM where it can. The new tree is
//! built without the synced flag and then synced, which puts the DOM in
//! order.
//!
//! Compositions (`addComposition`, `getCompositionContext`) come with the
//! view's input handling.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, LazyLock};

use div::{Document, Element};
use editor_state::{
    RangeCursor, RangeError, RangeSet, RangeValue, SpanIterator, Text, TextCursor, TextIterator,
};

use crate::attributes::{combine_attrs, Attrs};
use crate::decoration::{
    BlockWrapper, Decoration, DecorationKind, DecorationSet, DecorationSpec, Widget, WidgetType,
};
use crate::dom;
use crate::extension::ChangedRange;
use crate::tile::{flag::*, Adopted, TileId, TileKind, TilePointer, TileWalker, Tiles};

/// Why drawing the content failed.
#[derive(Debug)]
pub enum BuildError {
    /// The decorations break a rule (upstream throws a `RangeError`).
    Range(RangeError),
    /// Div's DOM refused an operation.
    Dom(div::Error),
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::Range(error) => write!(f, "RangeError: {}", error.0),
            BuildError::Dom(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for BuildError {}

impl From<div::Error> for BuildError {
    fn from(error: div::Error) -> Self {
        BuildError::Dom(error)
    }
}

/// How an update reused an old tile (`Reused`): whole, or only its DOM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reused {
    Full,
    Dom,
}

const CHUNK: usize = 512;
const BUCKET: usize = 6;
const WRAPPER_RESET: usize = 10000;

// The cache buckets, by tile class, as upstream numbers them.
const WIDGETS: usize = 0;
const LINES: usize = 1;
const TEXTS: usize = 2;
const MARKS: usize = 3;
const BUFFERS: usize = 4;
const WRAPPERS: usize = 5;
const DOCS: usize = 6;

fn bucket_of(kind: &TileKind) -> usize {
    match kind {
        TileKind::Widget(_) => WIDGETS,
        TileKind::Line(_) => LINES,
        TileKind::Text(_) => TEXTS,
        TileKind::Mark(_) => MARKS,
        TileKind::WidgetBuffer => BUFFERS,
        TileKind::BlockWrapper(_) => WRAPPERS,
        TileKind::Doc => DOCS,
    }
}

fn line_base_attrs() -> Attrs {
    [("class", "cm-line")].into_iter().collect()
}

// A block wrapper that covers the current position.
struct OpenWrapper {
    from: usize,
    to: usize,
    wrapper: Arc<BlockWrapper>,
    rank: f64,
}

/// Old tiles that an update may reuse (`TileCache`). Leaf and line tiles
/// may be reused whole; other tiles get new tiles that reuse their DOM.
pub(crate) struct TileCache {
    // Circular buffers, with `index` as the current position.
    buckets: [Vec<TileId>; 7],
    index: [usize; 7],
    pub(crate) reused: HashMap<TileId, Reused>,
}

impl TileCache {
    fn new() -> TileCache {
        TileCache {
            buckets: Default::default(),
            index: [0; 7],
            reused: HashMap::new(),
        }
    }

    // Put a tile in the cache.
    fn add(&mut self, tiles: &Tiles, tile: TileId) {
        let i = bucket_of(&tiles[tile].kind);
        let bucket = &mut self.buckets[i];
        if bucket.len() < BUCKET {
            bucket.push(tile);
        } else {
            self.index[i] = (self.index[i] + 1) % BUCKET;
            bucket[self.index[i]] = tile;
        }
    }

    // Take the oldest unused tile of a class that passes `test`.
    fn find(
        &mut self,
        tiles: &Tiles,
        bucket: usize,
        test: impl Fn(TileId) -> bool,
        reuse: Reused,
    ) -> Option<TileId> {
        let (list, off) = (&mut self.buckets[bucket], self.index[bucket]);
        for j in 0..list.len() {
            let index = (j + off) % list.len();
            let tile = list[index];
            debug_assert_eq!(bucket_of(&tiles[tile].kind), bucket);
            if test(tile) && !self.reused.contains_key(&tile) {
                list.remove(index);
                if index < off {
                    self.index[bucket] -= 1;
                }
                self.reused.insert(tile, reuse);
                return Some(tile);
            }
        }
        None
    }

    // Find a widget tile to draw `widget` with: one with an equal widget,
    // or else one whose DOM the widget can update (`findWidget`).
    fn find_widget(
        &mut self,
        tiles: &mut Tiles,
        document: &Document,
        widget: &Widget,
        length: usize,
        flags: u32,
    ) -> Option<TileId> {
        if self.buckets[WIDGETS].is_empty() {
            return None;
        }
        let (mut i, mut pass) = (0, 0);
        loop {
            if i == self.buckets[WIDGETS].len() {
                if pass == 1 {
                    return None;
                }
                pass = 1;
                i = 0;
            }
            let tile = self.buckets[WIDGETS][i];
            let old = tiles.widget(tile).expect("a widget tile").clone();
            let dom = tiles[tile].dom.element().expect("a widget element").clone();
            if !self.reused.contains_key(&tile)
                && (if pass == 0 {
                    old.compare(widget)
                } else {
                    old.same_type(widget) && widget.update_dom(&dom, document, &old)
                })
            {
                self.buckets[WIDGETS].remove(i);
                if i < self.index[WIDGETS] {
                    self.index[WIDGETS] -= 1;
                }
                let old_flags = tiles[tile].flags;
                return Some(
                    if old.ptr_eq(widget)
                        && tiles[tile].length == length
                        && old_flags & (WIDGET | BREAK_AFTER) == flags
                    {
                        self.reused.insert(tile, Reused::Full);
                        tile
                    } else {
                        self.reused.insert(tile, Reused::Dom);
                        tiles.widget_tile(
                            dom,
                            length,
                            widget.clone(),
                            (old_flags & !(WIDGET | BREAK_AFTER)) | flags,
                        )
                    },
                );
            }
            i += 1;
        }
    }

    fn reuse(&mut self, tile: TileId) -> TileId {
        self.reused.insert(tile, Reused::Full);
        tile
    }

    fn maybe_reuse(&mut self, tiles: &Tiles, tile: TileId, reuse: Reused) -> Option<Adopted> {
        if self.reused.contains_key(&tile) {
            return None;
        }
        self.reused.insert(tile, reuse);
        Some(tiles.adopt(tile))
    }
}

// Builds the new tile tree (`TileBuilder`). Marks and block wrappers
// belong to their content: adding content opens the ones it needs.
struct TileBuilder {
    document: Document,
    cache: TileCache,
    root: TileId,
    block_wrappers: RangeCursor<BlockWrapper>,
    cur_line: Option<TileId>,
    last_block: Option<TileId>,
    after_widget: Option<TileId>,
    pos: usize,
    wrappers: Vec<OpenWrapper>,
    wrapper_pos: usize,
    // Reused text tiles merged into the text before them, whose DOM is
    // removed after the update.
    merged: Vec<TileId>,
}

type Marks = [Arc<Decoration>];

impl TileBuilder {
    fn add_text(
        &mut self,
        tiles: &mut Tiles,
        text: &str,
        marks: &Marks,
        open_start: usize,
        tile: Option<TileId>,
    ) -> Result<(), BuildError> {
        self.flush_buffer(tiles)?;
        let parent = self.ensure_marks(tiles, marks, open_start)?;
        let prev = tiles.last_child(parent).filter(|&prev| {
            tiles.is_text(prev)
                && tiles[prev].flags & COMPOSITION == 0
                && tiles[prev].length + text.len() < CHUNK
        });
        if let Some(prev) = prev {
            // Upstream replaces `prev` with a new tile on its DOM. Changing
            // it in place is the same: nothing else holds it, and the old
            // tree, if it came from there, is past it.
            let tile_ref = &mut tiles[prev];
            if let TileKind::Text(old) = &mut tile_ref.kind {
                old.push_str(text);
            }
            tile_ref.length += text.len();
            tile_ref.flags = 0;
            if let Some(tile) = tile {
                self.merged.push(tile);
            }
        } else {
            let tile = match tile {
                Some(tile) => tile,
                None => {
                    let dom = self
                        .cache
                        .find(tiles, TEXTS, |_| true, Reused::Dom)
                        .map(|found| tiles.adopt(found));
                    tiles.text_of(&self.document, text, dom)?
                }
            };
            tiles.append(parent, tile);
        }
        self.pos += text.len();
        self.after_widget = None;
        Ok(())
    }

    fn add_inline_widget(
        &mut self,
        tiles: &mut Tiles,
        widget: TileId,
        marks: &Marks,
        open_start: usize,
    ) -> Result<(), BuildError> {
        // Adjacent same-side-facing non-replacing widgets don't need
        // buffers between them.
        let flags = tiles[widget].flags;
        let no_space = self.after_widget.is_some_and(|after| {
            flags & POINT_WIDGET != 0 && tiles[after].flags & POINT_WIDGET == flags & POINT_WIDGET
        });
        if !no_space {
            self.flush_buffer(tiles)?;
        }
        let parent = self.ensure_marks(tiles, marks, open_start)?;
        if !no_space && flags & BEFORE == 0 {
            let buffer = self.get_buffer(tiles, 1)?;
            tiles.append(parent, buffer);
        }
        tiles.append(parent, widget);
        self.pos += tiles[widget].length;
        self.after_widget = Some(widget);
        Ok(())
    }

    fn add_mark(
        &mut self,
        tiles: &mut Tiles,
        tile: TileId,
        marks: &Marks,
        open_start: usize,
    ) -> Result<(), BuildError> {
        self.flush_buffer(tiles)?;
        let parent = self.ensure_marks(tiles, marks, open_start)?;
        tiles.append(parent, tile);
        self.pos += tiles[tile].length;
        self.after_widget = None;
        Ok(())
    }

    fn add_block_widget(&mut self, tiles: &mut Tiles, widget: TileId) -> Result<(), BuildError> {
        let parent = self.get_block_pos(tiles)?;
        tiles.append(parent, widget);
        self.pos += tiles[widget].length;
        self.last_block = Some(widget);
        self.end_line(tiles)
    }

    fn continue_widget(&mut self, tiles: &mut Tiles, length: usize) {
        let widget = self
            .after_widget
            .or(self.last_block)
            .expect("a widget to continue");
        tiles[widget].length += length;
        self.pos += length;
    }

    fn add_line_start(
        &mut self,
        tiles: &mut Tiles,
        attrs: Option<Attrs>,
        dom: Option<Adopted>,
    ) -> Result<(), BuildError> {
        let attrs = attrs.unwrap_or_else(line_base_attrs);
        let keep_attrs = dom.is_some();
        let dom = match dom {
            Some(dom) => Some(dom),
            None => self
                .cache
                .find(tiles, LINES, |_| true, Reused::Dom)
                .map(|found| tiles.adopt(found)),
        };
        let tile = tiles.line_start(&self.document, attrs, dom, keep_attrs)?;
        let parent = self.get_block_pos(tiles)?;
        self.last_block = Some(tile);
        self.cur_line = Some(tile);
        tiles.append(parent, tile);
        Ok(())
    }

    fn add_line(&mut self, tiles: &mut Tiles, tile: TileId) -> Result<(), BuildError> {
        let parent = self.get_block_pos(tiles)?;
        tiles.append(parent, tile);
        self.pos += tiles[tile].length;
        self.last_block = Some(tile);
        self.end_line(tiles)
    }

    fn add_break(&mut self, tiles: &mut Tiles) -> Result<(), BuildError> {
        let last = self.last_block.expect("a block before a line break");
        tiles[last].flags |= BREAK_AFTER;
        self.end_line(tiles)?;
        self.pos += 1;
        Ok(())
    }

    fn add_line_start_if_not_covered(
        &mut self,
        tiles: &mut Tiles,
        attrs: Option<Attrs>,
    ) -> Result<(), BuildError> {
        if !self.block_pos_covered(tiles) {
            self.add_line_start(tiles, attrs, None)?;
        }
        Ok(())
    }

    fn ensure_line(&mut self, tiles: &mut Tiles, attrs: Option<Attrs>) -> Result<(), BuildError> {
        if self.cur_line.is_none() {
            self.add_line_start(tiles, attrs, None)?;
        }
        Ok(())
    }

    fn ensure_marks(
        &mut self,
        tiles: &mut Tiles,
        marks: &Marks,
        mut open_start: usize,
    ) -> Result<TileId, BuildError> {
        let mut parent = self.cur_line.expect("marks go in a line");
        for mark in marks.iter().rev() {
            let last = tiles
                .last_child(parent)
                .filter(|&last| tiles.mark(last).is_some_and(|m| m.range_eq(mark)));
            match last {
                Some(last) if open_start > 0 => {
                    parent = last;
                    open_start -= 1;
                }
                _ => {
                    let dom = self
                        .cache
                        .find(
                            tiles,
                            MARKS,
                            |t| tiles.mark(t).is_some_and(|m| m.range_eq(mark)),
                            Reused::Dom,
                        )
                        .map(|found| tiles.adopt(found));
                    let tile = tiles.mark_of(&self.document, mark, dom)?;
                    tiles.append(parent, tile);
                    parent = tile;
                    open_start = 0;
                }
            }
        }
        Ok(parent)
    }

    fn end_line(&mut self, tiles: &mut Tiles) -> Result<(), BuildError> {
        let Some(line) = self.cur_line else {
            return Ok(());
        };
        self.flush_buffer(tiles)?;
        let needs_break = match tiles.last_child(line) {
            None => true,
            Some(last) => {
                !has_content(tiles, line, false)
                    || tiles.is_widget(last)
                        && !dom::is_br(tiles[last].dom.element().expect("a widget element"))?
            }
        };
        if needs_break {
            let widget = &*BREAK_WIDGET;
            let tile = match self
                .cache
                .find_widget(tiles, &self.document, widget, 0, AFTER)
            {
                Some(tile) => tile,
                None => tiles.widget_tile(widget.to_dom(&self.document)?, 0, widget.clone(), AFTER),
            };
            tiles.append(line, tile);
        }
        self.cur_line = None;
        self.after_widget = None;
        Ok(())
    }

    fn update_block_wrappers(&mut self) {
        if self.wrapper_pos > self.pos + WRAPPER_RESET {
            self.block_wrappers.goto(self.pos);
            self.wrappers.clear();
        }
        let pos = self.pos;
        self.wrappers.retain(|wrapper| wrapper.to >= pos);
        let cur = &mut self.block_wrappers;
        while let Some(value) = cur.value().cloned() {
            if cur.from() > pos {
                break;
            }
            if cur.to() >= pos {
                let wrap = OpenWrapper {
                    from: cur.from(),
                    to: cur.to(),
                    rank: cur.rank() as f64 * 102.0 + value.rank(),
                    wrapper: value,
                };
                let mut i = self.wrappers.len();
                while i > 0 {
                    let prev = &self.wrappers[i - 1];
                    let order = if prev.rank != wrap.rank {
                        prev.rank - wrap.rank
                    } else {
                        prev.to as f64 - wrap.to as f64
                    };
                    if order >= 0.0 {
                        break;
                    }
                    i -= 1;
                }
                self.wrappers.insert(i, wrap);
            }
            cur.next();
        }
        self.wrapper_pos = self.pos;
    }

    fn get_block_pos(&mut self, tiles: &mut Tiles) -> Result<TileId, BuildError> {
        self.update_block_wrappers();
        let mut parent = self.root;
        for wrap in &self.wrappers {
            let last = tiles.last_child(parent).filter(|&last| {
                tiles
                    .wrapper(last)
                    .is_some_and(|wrapper| wrapper.range_eq(&wrap.wrapper))
            });
            match last {
                Some(last) if wrap.from < self.pos => parent = last,
                _ => {
                    let dom = self
                        .cache
                        .find(
                            tiles,
                            WRAPPERS,
                            |t| tiles.wrapper(t).is_some_and(|w| w.range_eq(&wrap.wrapper)),
                            Reused::Dom,
                        )
                        .map(|found| tiles.adopt(found));
                    let tile = tiles.block_wrapper_of(&self.document, &wrap.wrapper, dom)?;
                    tiles.append(parent, tile);
                    parent = tile;
                }
            }
        }
        Ok(parent)
    }

    fn block_pos_covered(&self, tiles: &Tiles) -> bool {
        self.last_block.is_some_and(|last| {
            tiles.break_after(last) == 0
                && (!tiles.is_widget(last) || tiles[last].flags & (AFTER | INC_END) > 0)
        })
    }

    fn get_buffer(&mut self, tiles: &mut Tiles, side: i8) -> Result<TileId, BuildError> {
        let flags = SYNCED | if side < 0 { BEFORE } else { AFTER };
        match self.cache.find(tiles, BUFFERS, |_| true, Reused::Full) {
            Some(found) => {
                tiles[found].flags = flags;
                Ok(found)
            }
            None => Ok(tiles.widget_buffer(&self.document, flags)?),
        }
    }

    fn flush_buffer(&mut self, tiles: &mut Tiles) -> Result<(), BuildError> {
        if let Some(after) = self.after_widget {
            if tiles[after].flags & AFTER == 0 {
                let parent = tiles[after].parent.expect("a placed widget");
                let buffer = self.get_buffer(tiles, -1)?;
                tiles.append(parent, buffer);
                self.after_widget = None;
            }
        }
        Ok(())
    }
}

// Reads the document text in pieces (`TextStream`).
struct TextStream<'a> {
    cursor: TextCursor<'a>,
    skip_count: usize,
    text: &'a str,
    text_off: usize,
}

impl<'a> TextStream<'a> {
    fn new(doc: &'a Text) -> TextStream<'a> {
        TextStream {
            cursor: doc.iter(1),
            skip_count: 0,
            text: "",
            text_off: 0,
        }
    }

    // Advance past replaced content.
    fn skip(&mut self, len: usize) {
        if self.text_off + len <= self.text.len() {
            self.text_off += len;
        } else {
            self.skip_count += len - (self.text.len() - self.text_off);
            self.text = "";
            self.text_off = 0;
        }
    }

    // The next piece of text, at most `max_len` bytes, or `None` for a line
    // break.
    fn next(&mut self, max_len: usize) -> Option<&'a str> {
        if self.text_off == self.text.len() {
            self.cursor.advance(self.skip_count as isize);
            self.skip_count = 0;
            assert!(
                !self.cursor.done(),
                "Ran out of text content when drawing inline views"
            );
            let value = self.cursor.value();
            self.text = value;
            let len = char_floor(value, 0, max_len);
            self.text_off = len;
            return (!self.cursor.line_break()).then(|| &value[..len]);
        }
        let end = char_floor(self.text, self.text_off, max_len);
        let chars = &self.text[self.text_off..end];
        self.text_off = end;
        Some(chars)
    }
}

// The end of a piece of at most `max_len` bytes of `text` from `start`,
// on a character boundary. Positions in the document are on boundaries,
// so only the chunk size can split a character; then the piece is cut
// before it.
fn char_floor(text: &str, start: usize, max_len: usize) -> usize {
    let mut end = text.len().min(start + max_len);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if end == start && start < text.len() {
        end = start + text[start..].chars().next().map_or(0, char::len_utf8);
    }
    end
}

/// One pass over the document that builds a new tile tree from the old
/// one (`TileUpdate`).
pub(crate) struct TileUpdate<'a> {
    tiles: &'a mut Tiles,
    document: Document,
    doc: &'a Text,
    text: TextStream<'a>,
    builder: TileBuilder,
    old: TilePointer,
    old_root: TileId,
    open_widget: bool,
    open_marks: usize,
    decorations: &'a [DecorationSet],
    disallow_block_effects_for: &'a [bool],
}

/// What a [`TileUpdate`] made: the new tree, the old tiles it reused, and
/// the reused text tiles it merged away.
pub(crate) struct Built {
    pub(crate) root: TileId,
    pub(crate) reused: HashMap<TileId, Reused>,
    pub(crate) merged: Vec<TileId>,
}

impl<'a> TileUpdate<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        tiles: &'a mut Tiles,
        document: &Document,
        content: &Element,
        doc: &'a Text,
        old: TileId,
        block_wrappers: &[RangeSet<BlockWrapper>],
        decorations: &'a [DecorationSet],
        disallow_block_effects_for: &'a [bool],
    ) -> TileUpdate<'a> {
        let root = tiles.new_doc(content.clone());
        let mut cache = TileCache::new();
        cache.reused.insert(old, Reused::Dom);
        TileUpdate {
            tiles,
            document: document.clone(),
            doc,
            text: TextStream::new(doc),
            builder: TileBuilder {
                document: document.clone(),
                cache,
                root,
                block_wrappers: RangeSet::iter_sets(block_wrappers, 0),
                cur_line: None,
                last_block: None,
                after_widget: None,
                pos: 0,
                wrappers: Vec::new(),
                wrapper_pos: 0,
                merged: Vec::new(),
            },
            old: TilePointer::new(old),
            old_root: old,
            open_widget: false,
            open_marks: 0,
            decorations,
            disallow_block_effects_for,
        }
    }

    /// Build the new tree for `changes` (`run`).
    pub(crate) fn run(mut self, changes: &[ChangedRange]) -> Result<Built, BuildError> {
        let (mut pos_a, mut pos_b) = (0, 0);
        let mut changes = changes.iter();
        loop {
            let next = changes.next();
            let skip_a = next.map_or(self.tiles[self.old_root].length, |next| next.from_a);
            if skip_a > pos_a {
                let len = skip_a - pos_a;
                self.preserve(len, next.is_none())?;
                pos_b += len;
            }
            let Some(next) = next else {
                break;
            };
            self.forward(next.from_a, next.to_a, 1)?;
            self.emit(pos_b, next.to_b)?;
            pos_b = next.to_b;
            pos_a = next.to_a;
        }
        if self.builder.cur_line.is_some() {
            self.builder.end_line(self.tiles)?;
        }
        Ok(Built {
            root: self.builder.root,
            reused: self.builder.cache.reused,
            merged: self.builder.merged,
        })
    }

    // Copy `length` positions of the old tree into the new one.
    fn preserve(&mut self, length: usize, inc_end: bool) -> Result<(), BuildError> {
        let active_marks = get_marks(&self.old, self.tiles);
        let mut walker = PreserveWalker {
            builder: &mut self.builder,
            document: &self.document,
            open_widget: &mut self.open_widget,
            open_marks: self.open_marks,
            active_marks,
        };
        self.old.advance(
            self.tiles,
            length,
            if inc_end { 1 } else { -1 },
            Some(&mut walker),
        )?;
        self.text.skip(length);
        Ok(())
    }

    // Build new tiles for `from..to` of the new document.
    fn emit(&mut self, from: usize, to: usize) -> Result<(), BuildError> {
        let mut spans = EmitSpans {
            builder: &mut self.builder,
            tiles: &mut *self.tiles,
            text: &mut self.text,
            doc: self.doc,
            disallow_block_effects_for: self.disallow_block_effects_for,
            pending_line_attrs: None,
            mark_count: None,
            error: None,
        };
        let open_end = RangeSet::spans(self.decorations, from, to, &mut spans, -1);
        if let Some(error) = spans.error {
            return Err(error);
        }
        let (mark_count, pending) = (spans.mark_count, spans.pending_line_attrs);
        if let Some(mark_count) = mark_count {
            self.open_widget = open_end > mark_count;
        }
        if !self.open_widget {
            self.builder
                .add_line_start_if_not_covered(self.tiles, pending)?;
        }
        self.open_marks = open_end;
        Ok(())
    }

    // Move the old tree's pointer over a changed range, caching the tiles
    // near its ends for reuse.
    fn forward(&mut self, from: usize, to: usize, side: i8) -> Result<(), BuildError> {
        let mut walker = ReuseWalker {
            cache: &mut self.builder.cache,
        };
        if to - from <= 10 {
            self.old
                .advance(self.tiles, to - from, side, Some(&mut walker))?;
        } else {
            self.old.advance(self.tiles, 5, -1, Some(&mut walker))?;
            self.old.advance(self.tiles, to - from - 10, -1, None)?;
            self.old.advance(self.tiles, 5, side, Some(&mut walker))?;
        }
        Ok(())
    }
}

// The walker that puts skipped old tiles in the cache (`reuseWalker`).
struct ReuseWalker<'b> {
    cache: &'b mut TileCache,
}

impl TileWalker for ReuseWalker<'_> {
    fn enter(&mut self, tiles: &mut Tiles, tile: TileId) -> Result<(), BuildError> {
        self.cache.add(tiles, tile);
        Ok(())
    }

    fn leave(&mut self, _tiles: &mut Tiles, _tile: TileId) {}

    fn skip(
        &mut self,
        tiles: &mut Tiles,
        tile: TileId,
        _from: usize,
        _to: usize,
    ) -> Result<bool, BuildError> {
        self.cache.add(tiles, tile);
        Ok(!tiles.is_composite(tile))
    }

    fn line_break(&mut self, _tiles: &mut Tiles) -> Result<(), BuildError> {
        Ok(())
    }
}

// What `preserve` does with a tile the pointer moves over.
enum Skipped {
    Widget(Widget),
    Text(String),
    Line,
    Buffer,
    Mark,
    Other,
}

impl Skipped {
    fn of(tiles: &Tiles, tile: TileId) -> Skipped {
        match &tiles[tile].kind {
            TileKind::Widget(widget) => Skipped::Widget(widget.clone()),
            TileKind::Text(text) => Skipped::Text(text.clone()),
            TileKind::Line(_) => Skipped::Line,
            TileKind::WidgetBuffer => Skipped::Buffer,
            TileKind::Mark(_) => Skipped::Mark,
            TileKind::Doc | TileKind::BlockWrapper(_) => Skipped::Other,
        }
    }
}

// The walker that copies unchanged content (`preserve`).
struct PreserveWalker<'b> {
    builder: &'b mut TileBuilder,
    document: &'b Document,
    open_widget: &'b mut bool,
    open_marks: usize,
    active_marks: Vec<Arc<Decoration>>,
}

impl TileWalker for PreserveWalker<'_> {
    fn skip(
        &mut self,
        tiles: &mut Tiles,
        tile: TileId,
        from: usize,
        to: usize,
    ) -> Result<bool, BuildError> {
        let b = &mut *self.builder;
        match Skipped::of(tiles, tile) {
            Skipped::Widget(widget) => {
                if *self.open_widget {
                    b.continue_widget(tiles, to - from);
                } else {
                    let length = tiles[tile].length;
                    let new_tile = if to > 0 || from < length {
                        let dom = b.cache.maybe_reuse(tiles, tile, Reused::Dom);
                        let flags = tiles[tile].flags & WIDGET;
                        tiles.widget_of(self.document, &widget, to - from, flags, dom)?
                    } else {
                        b.cache.reuse(tile)
                    };
                    if tiles[new_tile].flags & BLOCK != 0 {
                        tiles[new_tile].flags &= !BREAK_AFTER;
                        b.add_block_widget(tiles, new_tile)?;
                    } else {
                        b.ensure_line(tiles, None)?;
                        b.add_inline_widget(tiles, new_tile, &self.active_marks, self.open_marks)?;
                        self.open_marks = self.active_marks.len();
                    }
                }
            }
            Skipped::Text(text) => {
                b.ensure_line(tiles, None)?;
                if from == 0 && to == text.len() && !b.cache.reused.contains_key(&tile) {
                    let tile = b.cache.reuse(tile);
                    b.add_text(
                        tiles,
                        &text,
                        &self.active_marks,
                        self.open_marks,
                        Some(tile),
                    )?;
                } else {
                    b.cache.add(tiles, tile);
                    b.add_text(
                        tiles,
                        &text[from..to],
                        &self.active_marks,
                        self.open_marks,
                        None,
                    )?;
                }
                self.open_marks = self.active_marks.len();
            }
            Skipped::Line => {
                tiles[tile].flags &= !BREAK_AFTER;
                b.cache.reused.insert(tile, Reused::Full);
                b.add_line(tiles, tile)?;
            }
            Skipped::Buffer => b.cache.add(tiles, tile),
            Skipped::Mark => {
                b.ensure_line(tiles, None)?;
                b.add_mark(tiles, tile, &self.active_marks, self.open_marks)?;
                b.cache.reused.insert(tile, Reused::Full);
                self.open_marks = self.active_marks.len();
            }
            Skipped::Other => return Ok(false),
        }
        *self.open_widget = false;
        Ok(true)
    }

    fn enter(&mut self, tiles: &mut Tiles, tile: TileId) -> Result<(), BuildError> {
        let b = &mut *self.builder;
        let line_attrs = match &tiles[tile].kind {
            TileKind::Line(attrs) => Some(attrs.clone()),
            _ => None,
        };
        if let Some(attrs) = line_attrs {
            let dom = b.cache.maybe_reuse(tiles, tile, Reused::Dom);
            b.add_line_start(tiles, Some(attrs), dom)?;
        } else {
            b.cache.add(tiles, tile);
            if let Some(mark) = tiles.mark(tile) {
                self.active_marks.insert(0, mark.clone());
            }
        }
        *self.open_widget = false;
        Ok(())
    }

    fn leave(&mut self, tiles: &mut Tiles, tile: TileId) {
        if tiles.is_line(tile) {
            if !self.active_marks.is_empty() {
                self.active_marks.clear();
                self.open_marks = 0;
            }
        } else if tiles.mark(tile).is_some() {
            self.active_marks.remove(0);
            self.open_marks = self.open_marks.min(self.active_marks.len());
        }
    }

    fn line_break(&mut self, tiles: &mut Tiles) -> Result<(), BuildError> {
        self.builder.add_break(tiles)?;
        *self.open_widget = false;
        Ok(())
    }
}

// The decoration spans of a changed range (`emit`'s iterator).
struct EmitSpans<'b, 'a> {
    builder: &'b mut TileBuilder,
    tiles: &'b mut Tiles,
    text: &'b mut TextStream<'a>,
    doc: &'a Text,
    disallow_block_effects_for: &'a [bool],
    pending_line_attrs: Option<Attrs>,
    mark_count: Option<usize>,
    error: Option<BuildError>,
}

impl EmitSpans<'_, '_> {
    fn point(
        &mut self,
        from: usize,
        to: usize,
        deco: &Arc<Decoration>,
        active: &Marks,
        open_start: usize,
        index: usize,
    ) -> Result<(), BuildError> {
        if let DecorationKind::Point { block, .. } = deco.kind() {
            if self.disallow_block_effects_for.get(index) == Some(&true) {
                if block {
                    return Err(BuildError::Range(RangeError(
                        "Block decorations may not be specified via plugins".into(),
                    )));
                }
                if to > self.doc.line_at(from).to {
                    return Err(BuildError::Range(RangeError(
                        "Decorations that replace line breaks may not be specified via plugins"
                            .into(),
                    )));
                }
            }
            self.mark_count = Some(active.len());
            let (b, tiles) = (&mut *self.builder, &mut *self.tiles);
            if open_start > active.len() {
                b.continue_widget(tiles, to - from);
            } else {
                let widget = deco.widget_type().cloned().unwrap_or_else(|| {
                    if block {
                        NULL_BLOCK.clone()
                    } else {
                        NULL_INLINE.clone()
                    }
                });
                let flags = widget_flags(deco);
                let document = b.document.clone();
                let tile = match b
                    .cache
                    .find_widget(tiles, &document, &widget, to - from, flags)
                {
                    Some(tile) => tile,
                    None => tiles.widget_of(&document, &widget, to - from, flags, None)?,
                };
                if block {
                    if deco.start_side() > 0 {
                        b.add_line_start_if_not_covered(tiles, self.pending_line_attrs.take())?;
                    }
                    b.add_block_widget(tiles, tile)?;
                } else {
                    b.ensure_line(tiles, self.pending_line_attrs.take())?;
                    b.add_inline_widget(tiles, tile, active, open_start)?;
                }
            }
            self.pending_line_attrs = None;
        } else {
            self.pending_line_attrs = add_line_deco(self.pending_line_attrs.take(), deco);
        }
        if to > from {
            self.text.skip(to - from);
        }
        Ok(())
    }

    fn span(
        &mut self,
        from: usize,
        to: usize,
        active: &Marks,
        open_start: usize,
    ) -> Result<(), BuildError> {
        let (b, tiles) = (&mut *self.builder, &mut *self.tiles);
        let mut pos = from;
        while pos < to {
            match self.text.next(CHUNK.min(to - pos)) {
                None => {
                    b.add_line_start_if_not_covered(tiles, self.pending_line_attrs.take())?;
                    b.add_break(tiles)?;
                    pos += 1;
                }
                Some(chars) => {
                    b.ensure_line(tiles, self.pending_line_attrs.take())?;
                    let open = if pos == from {
                        open_start
                    } else {
                        active.len()
                    };
                    b.add_text(tiles, chars, active, open, None)?;
                    pos += chars.len();
                }
            }
            self.pending_line_attrs = None;
        }
        self.mark_count = Some(active.len());
        Ok(())
    }
}

impl SpanIterator<Decoration> for EmitSpans<'_, '_> {
    fn span(&mut self, from: usize, to: usize, active: &Marks, open_start: usize) {
        if self.error.is_none() {
            if let Err(error) = EmitSpans::span(self, from, to, active, open_start) {
                self.error = Some(error);
            }
        }
    }

    fn point(
        &mut self,
        from: usize,
        to: usize,
        value: &Arc<Decoration>,
        active: &Marks,
        open_start: usize,
        index: usize,
    ) {
        if self.error.is_none() {
            if let Err(error) = EmitSpans::point(self, from, to, value, active, open_start, index) {
                self.error = Some(error);
            }
        }
    }
}

fn has_content(tiles: &Tiles, tile: TileId, require_text: bool) -> bool {
    tiles[tile].children.iter().any(|&child| {
        (if require_text {
            tiles.is_text(child)
        } else {
            tiles[child].length > 0
        }) || has_content(tiles, child, require_text)
    })
}

fn widget_flags(deco: &Decoration) -> u32 {
    let DecorationKind::Point { block, is_replace } = deco.kind() else {
        return 0;
    };
    let mut flags = if is_replace {
        (if deco.start_side() < 0 { INC_START } else { 0 })
            | (if deco.end_side() > 0 { INC_END } else { 0 })
    } else if deco.start_side() > 0 {
        AFTER
    } else {
        BEFORE
    };
    if block {
        flags |= BLOCK;
    }
    flags
}

fn add_line_deco(value: Option<Attrs>, deco: &Decoration) -> Option<Attrs> {
    let DecorationSpec::Line(spec) = deco.spec() else {
        return value;
    };
    if spec.attributes.is_none() && spec.class.is_none() {
        return value;
    }
    let mut value = value.unwrap_or_else(line_base_attrs);
    if let Some(attrs) = &spec.attributes {
        value = combine_attrs(attrs, value);
    }
    if let Some(class) = &spec.class {
        let joined = format!("{} {class}", value.get("class").unwrap_or_default());
        value.set("class", joined);
    }
    Some(value)
}

// The marks around the pointer, innermost first (`getMarks`).
fn get_marks(ptr: &TilePointer, tiles: &Tiles) -> Vec<Arc<Decoration>> {
    let mut found = Vec::new();
    for i in (2..=ptr.parents.len()).rev() {
        let tile = if i == ptr.parents.len() {
            ptr.tile
        } else {
            ptr.parents[i].0
        };
        if let Some(mark) = tiles.mark(tile) {
            found.push(mark.clone());
        }
    }
    found
}

/// Remove the tiles of `tile`'s tree that an update did not reuse
/// (`destroyDropped`), with their DOM, and free the slots of those whose
/// DOM it reused.
pub(crate) fn destroy_dropped(
    tiles: &mut Tiles,
    tile: TileId,
    reused: Option<&HashMap<TileId, Reused>>,
) -> Result<(), div::Error> {
    let r = reused.and_then(|reused| reused.get(&tile).copied());
    if r == Some(Reused::Full) {
        return Ok(());
    }
    if r.is_none() {
        tiles.destroy(tile);
    }
    for child in tiles[tile].children.clone() {
        destroy_dropped(tiles, child, reused)?;
    }
    // Upstream leaves the dropped DOM to the garbage collector, once sync
    // has taken it out of the content; Div's nodes are removed explicitly.
    if r.is_none() {
        tiles[tile].dom.remove()?;
    }
    tiles.release(tile);
    Ok(())
}

// A widget for replaced content without a widget (`NullWidget`).
struct NullWidget(&'static str);

impl WidgetType for NullWidget {
    fn to_dom(&self, document: &Document) -> Result<Element, div::Error> {
        dom::create_element(document, self.0)
    }

    fn eq(&self, other: &Self) -> bool {
        other.0 == self.0
    }

    fn update_dom(&self, dom: &Element, _document: &Document, _from: &Self) -> bool {
        let Ok(kind) = dom.kind() else {
            return false;
        };
        // div-gap: 25 `Element.nodeName`; Div's kind stands for the tag.
        kind == if self.0 == "div" {
            div::ElementTag::Div
        } else {
            div::ElementTag::Span
        }
    }

    fn is_hidden(&self) -> bool {
        true
    }
}

static NULL_INLINE: LazyLock<Widget> = LazyLock::new(|| Widget::new(NullWidget("span")));
static NULL_BLOCK: LazyLock<Widget> = LazyLock::new(|| Widget::new(NullWidget("div")));

// The `<br>` that keeps an empty line, or one ending in a widget, open
// (`BreakWidget`).
struct BreakWidget;

impl WidgetType for BreakWidget {
    fn to_dom(&self, document: &Document) -> Result<Element, div::Error> {
        dom::create_element(document, "br")
    }

    fn is_hidden(&self) -> bool {
        true
    }

    fn editable(&self) -> bool {
        true
    }
}

static BREAK_WIDGET: LazyLock<Widget> = LazyLock::new(|| Widget::new(BreakWidget));
