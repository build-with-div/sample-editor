//! Part of `docview.ts`: drawing the content and redrawing it after
//! changes, without the editor view around it.
//!
//! [`ContentTiles`] holds what `DocView` keeps for drawing: the tile tree,
//! the decorations and the block wrappers. Its constructor and
//! [`update`](ContentTiles::update) do what `DocView`'s do, apart from
//! compositions, the selection, measuring and the viewport, which come
//! with the editor view.

use std::sync::Arc;

use div::{Document, Element, ElementTag};
use editor_state::{ChangeSet, RangeComparator, RangeSet, Text};

use crate::buildtile::{destroy_dropped, BuildError, Built, TileUpdate};
use crate::decoration::{add_range, BlockWrapper, Decoration, DecorationSet};
use crate::extension::ChangedRange;
use crate::tile::{flag, TileId, TileKind, Tiles};

/// The drawn content of an editor (`DocView`'s tile tree), in a
/// `.cm-content` element.
pub struct ContentTiles {
    tiles: Tiles,
    root: TileId,
    document: Document,
    content: Element,
    decorations: Vec<DecorationSet>,
    block_wrappers: Vec<RangeSet<BlockWrapper>>,
}

impl ContentTiles {
    /// Draw `doc` with `decorations` and `block_wrappers` into `content`.
    pub fn new(
        document: &Document,
        content: Element,
        doc: &Text,
        decorations: Vec<DecorationSet>,
        block_wrappers: Vec<RangeSet<BlockWrapper>>,
    ) -> Result<ContentTiles, BuildError> {
        let mut tiles = Tiles::default();
        let root = tiles.new_doc(content.clone());
        let mut view = ContentTiles {
            tiles,
            root,
            document: document.clone(),
            content,
            decorations,
            block_wrappers,
        };
        view.update_inner(&[ChangedRange::new(0, 0, 0, doc.len())], doc)?;
        Ok(view)
    }

    /// Redraw for a change from the previous document to `doc` and new
    /// decorations and block wrappers. Returns whether anything was
    /// redrawn.
    pub fn update(
        &mut self,
        doc: &Text,
        changes: &ChangeSet,
        decorations: Vec<DecorationSet>,
        block_wrappers: Vec<RangeSet<BlockWrapper>>,
    ) -> Result<bool, BuildError> {
        let mut changed_ranges = Vec::new();
        changes.iter_changed_ranges(
            |from_a, to_a, from_b, to_b| {
                changed_ranges.push(ChangedRange::new(from_a, to_a, from_b, to_b))
            },
            false,
        );
        let prev_deco = std::mem::replace(&mut self.decorations, decorations);
        let prev_wrappers = std::mem::replace(&mut self.block_wrappers, block_wrappers);
        let deco_diff = find_changed_deco(&prev_deco, &self.decorations, changes);
        if !deco_diff.is_empty() {
            changed_ranges = ChangedRange::extend_with_ranges(&changed_ranges, &deco_diff);
        }
        let block_diff = find_changed_wrappers(&prev_wrappers, &self.block_wrappers, changes);
        if !block_diff.is_empty() {
            changed_ranges = ChangedRange::extend_with_ranges(&changed_ranges, &block_diff);
        }
        if self.tiles[self.root].flags & flag::SYNCED != 0 && changed_ranges.is_empty() {
            return Ok(false);
        }
        self.update_inner(&changed_ranges, doc)?;
        Ok(true)
    }

    fn update_inner(&mut self, changes: &[ChangedRange], doc: &Text) -> Result<(), BuildError> {
        let old = self.root;
        let disallow = vec![false; self.decorations.len()];
        let Built {
            root,
            reused,
            merged,
        } = TileUpdate::new(
            &mut self.tiles,
            &self.document,
            &self.content,
            doc,
            old,
            &self.block_wrappers,
            &self.decorations,
            &disallow,
        )
        .run(changes)?;
        self.root = root;
        self.tiles.sync(root)?;
        destroy_dropped(&mut self.tiles, old, Some(&reused))?;
        for tile in merged {
            self.tiles[tile].dom.remove()?;
            self.tiles.release(tile);
        }
        Ok(())
    }

    /// The tile tree as upstream's `toString` writes it, such as
    /// `DocTile(LineTile("one")#,LineTile("two"))`.
    pub fn describe(&self) -> String {
        let mut out = String::new();
        self.tiles.describe(self.root, &mut out);
        out
    }

    /// The number of tiles alive, to check that updates free what they
    /// drop.
    pub fn tile_count(&self) -> usize {
        self.tiles.live()
    }

    /// The line or block widget at `pos` and the offset in it, described
    /// as [`describe`](Self::describe) does (`resolveBlock`).
    pub fn resolve_block(&self, pos: usize, side: i32) -> (String, usize) {
        let (tile, offset) = self.tiles.resolve_block(self.root, pos, side);
        let mut out = String::new();
        self.tiles.describe(tile, &mut out);
        (out, offset)
    }

    /// The `outerHTML` of each block in the content, read from Div's DOM
    /// in the order of the tile tree.
    ///
    /// div-gap: 24 node traversal over text nodes; 23 attribute
    /// enumeration; 25 `nodeName`. Div's element API cannot walk text
    /// nodes or list attributes, so this follows the tile tree for the
    /// order of nodes, reads the attributes upstream's tiles and the
    /// tests' widgets write, by name, and names elements by their kind.
    pub fn block_html(&self) -> Result<Vec<String>, div::Error> {
        self.tiles[self.root]
            .children
            .iter()
            .map(|&block| {
                let mut out = String::new();
                self.tile_html(block, &mut out)?;
                Ok(out)
            })
            .collect()
    }

    fn tile_html(&self, tile: TileId, out: &mut String) -> Result<(), div::Error> {
        let t = &self.tiles[tile];
        match &t.kind {
            TileKind::Text(_) => escape(&t.dom.text_node()?.data()?, false, out),
            TileKind::Widget(_) | TileKind::WidgetBuffer => {
                element_html(t.dom.element().expect("an element"), out)?
            }
            _ => {
                let element = t.dom.element().expect("an element");
                let tag = open_tag(element, out)?;
                for &child in &t.children {
                    self.tile_html(child, out)?;
                }
                close_tag(tag, out);
            }
        }
        Ok(())
    }

    /// Remove the content's tiles and their DOM (`destroy`).
    pub fn destroy(mut self) -> Result<(), div::Error> {
        let root = self.root;
        // The content element belongs to the caller.
        for child in self.tiles[root].children.clone() {
            destroy_dropped(&mut self.tiles, child, None)?;
        }
        self.tiles.destroy(root);
        self.tiles.release(root);
        Ok(())
    }
}

// Attributes the serializer reads, in the order the tiles and the tests'
// widgets write them (div-gap 23).
const ATTRIBUTES: [&str; 8] = [
    "class",
    "aria-hidden",
    "contenteditable",
    "style",
    "title",
    "lang",
    "dir",
    "data-test",
];

fn tag_name(kind: ElementTag) -> String {
    match kind {
        ElementTag::Div => "div".into(),
        ElementTag::Span => "span".into(),
        ElementTag::Image => "img".into(),
        ElementTag::Break => "br".into(),
        other => format!("{other:?}").to_lowercase(),
    }
}

fn open_tag(element: &Element, out: &mut String) -> Result<String, div::Error> {
    let tag = tag_name(element.kind()?);
    out.push('<');
    out.push_str(&tag);
    for name in ATTRIBUTES {
        if let Some(value) = element.get_attribute(name)? {
            out.push(' ');
            out.push_str(name);
            out.push_str("=\"");
            escape(&value, true, out);
            out.push('"');
        }
    }
    out.push('>');
    Ok(tag)
}

fn close_tag(tag: String, out: &mut String) {
    if tag != "img" && tag != "br" {
        out.push_str("</");
        out.push_str(&tag);
        out.push('>');
    }
}

// An element that is not a tile's: its element children, or its text when
// it has none.
fn element_html(element: &Element, out: &mut String) -> Result<(), div::Error> {
    let tag = open_tag(element, out)?;
    let mut child = element.first_child()?;
    if child.is_none() {
        escape(&element.text_content()?, false, out);
    }
    while let Some(current) = child {
        element_html(&current, out)?;
        child = current.next_sibling()?;
    }
    close_tag(tag, out);
    Ok(())
}

// Escape as the HTML serializer does: `&`, `<`, `>` and no-break spaces in
// text; `&`, `"` and no-break spaces in attribute values.
fn escape(text: &str, attribute: bool, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            '"' if attribute => out.push_str("&quot;"),
            '<' if !attribute => out.push_str("&lt;"),
            '>' if !attribute => out.push_str("&gt;"),
            ch => out.push(ch),
        }
    }
}

// Records where two decoration sets differ (`DecorationComparator`).
struct DecorationComparator {
    changes: Vec<usize>,
}

impl RangeComparator<Decoration> for DecorationComparator {
    fn compare_range(
        &mut self,
        from: usize,
        to: usize,
        _active_a: &[Arc<Decoration>],
        _active_b: &[Arc<Decoration>],
    ) {
        add_range(from, to, &mut self.changes, 0);
    }

    fn compare_point(
        &mut self,
        from: usize,
        to: usize,
        _point_a: Option<&Arc<Decoration>>,
        _point_b: Option<&Arc<Decoration>>,
    ) {
        add_range(from, to, &mut self.changes, 0);
    }

    fn wants_bound_changes(&self) -> bool {
        true
    }

    fn bound_change(&mut self, pos: usize) {
        add_range(pos, pos, &mut self.changes, 0);
    }
}

fn find_changed_deco(a: &[DecorationSet], b: &[DecorationSet], diff: &ChangeSet) -> Vec<usize> {
    let mut comp = DecorationComparator {
        changes: Vec::new(),
    };
    RangeSet::compare(a, b, diff.desc(), &mut comp, -1);
    comp.changes
}

// Records where two block wrapper sets differ (`WrapperComparator`).
struct WrapperComparator {
    changes: Vec<usize>,
}

impl RangeComparator<BlockWrapper> for WrapperComparator {
    fn compare_range(
        &mut self,
        from: usize,
        to: usize,
        _active_a: &[Arc<BlockWrapper>],
        _active_b: &[Arc<BlockWrapper>],
    ) {
        add_range(from, to, &mut self.changes, 0);
    }

    fn compare_point(
        &mut self,
        _from: usize,
        _to: usize,
        _point_a: Option<&Arc<BlockWrapper>>,
        _point_b: Option<&Arc<BlockWrapper>>,
    ) {
    }

    fn wants_bound_changes(&self) -> bool {
        true
    }

    fn bound_change(&mut self, pos: usize) {
        add_range(pos, pos, &mut self.changes, 0);
    }
}

fn find_changed_wrappers(
    a: &[RangeSet<BlockWrapper>],
    b: &[RangeSet<BlockWrapper>],
    diff: &ChangeSet,
) -> Vec<usize> {
    let mut comp = WrapperComparator {
        changes: Vec::new(),
    };
    RangeSet::compare(a, b, diff.desc(), &mut comp, -1);
    comp.changes
}
