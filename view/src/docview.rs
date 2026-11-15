//! `docview.ts`: drawing the content, redrawing it after changes and
//! measuring it.
//!
//! [`ContentTiles`] holds what `DocView` keeps for drawing: the tile tree,
//! the decorations and the block wrappers. Its constructor and
//! [`update`](ContentTiles::update) draw without a view, for the tests.
//! `DocView` owns one inside the editor view and adds the height lock,
//! the decorations the view draws (gap spacers and line gaps among
//! them) and the measurement of line heights and text size.
//! Compositions and the selection come with input handling (M7).

use std::sync::Arc;

use div::{Document, Element, ElementTag};
use editor_state::{ChangeSet, EditorState, RangeComparator, RangeSet, Text};

use crate::buildtile::{destroy_dropped, BuildError, Built, TileUpdate};
use crate::decoration::{
    add_range, BlockWrapper, Decoration, DecorationSet, ReplaceSpec, Widget, WidgetType,
};
use crate::dom::{bounding_rect, set_attribute, set_style, Rect};
use crate::extension::{ChangedRange, ViewUpdate, BLOCK_WRAPPERS, DECORATIONS, OUTER_DECORATIONS};
use crate::tile::{flag, TileId, TileKind, Tiles};
use crate::viewstate::{js, MustMeasure, ViewState, Viewport};

/// The drawn content of an editor (`DocView`'s tile tree), in a
/// `.cm-content` element.
pub struct ContentTiles {
    pub(crate) tiles: Tiles,
    pub(crate) root: TileId,
    pub(crate) document: Document,
    pub(crate) content: Element,
    decorations: Vec<DecorationSet>,
    block_wrappers: Vec<RangeSet<BlockWrapper>>,
    disallow: Vec<bool>,
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
            disallow: Vec::new(),
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
        let disallow = vec![false; decorations.len()];
        self.redraw(
            doc,
            changes,
            changed_ranges,
            decorations,
            block_wrappers,
            disallow,
        )
    }

    /// The decoration and redraw part of `DocView.update`: extend
    /// `changed_ranges` with where the decorations and block wrappers
    /// changed, then redraw. `disallow` marks the decoration sets that may
    /// not cover a composition (`dynamicDecorationMap`).
    pub(crate) fn redraw(
        &mut self,
        doc: &Text,
        changes: &ChangeSet,
        mut changed_ranges: Vec<ChangedRange>,
        decorations: Vec<DecorationSet>,
        block_wrappers: Vec<RangeSet<BlockWrapper>>,
        disallow: Vec<bool>,
    ) -> Result<bool, BuildError> {
        self.disallow = disallow;
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
        let disallow = if self.disallow.len() == self.decorations.len() {
            self.disallow.clone()
        } else {
            vec![false; self.decorations.len()]
        };
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

/// The content's drawing as the editor view drives it (`DocView`): the
/// decorations it draws, including the gap spacers outside the viewport
/// and the line gaps, the redraw after updates and the measurements of
/// the drawn content.
///
/// Compositions and the DOM selection come with the view's input handling
/// in milestone M7.
pub(crate) struct DocView {
    pub(crate) content: ContentTiles,
    /// The minimum width of the content (`minWidth`), with the range of
    /// the line that set it. Once a line of some width is seen, the editor
    /// stays at least that wide until that line changes.
    pub(crate) min_width: f64,
    min_width_from: usize,
    min_width_to: usize,
}

impl DocView {
    /// Draw `state` into `content_dom` (`new DocView`).
    pub(crate) fn new(
        document: &Document,
        content_dom: Element,
        state: &EditorState,
        vs: &mut ViewState,
    ) -> Result<DocView, BuildError> {
        let (decorations, wrappers) = update_deco(state, vs)?;
        vs.must_measure_content = MustMeasure::Yes;
        lock_height(&content_dom, vs)?;
        let content = ContentTiles::new(
            document,
            content_dom.clone(),
            state.doc(),
            decorations,
            wrappers,
        )?;
        let view = DocView {
            content,
            min_width: 0.0,
            min_width_from: 0,
            min_width_to: 0,
        };
        view.unlock_height()?;
        Ok(view)
    }

    /// Update for a view update; returns whether anything was redrawn
    /// (`update`).
    pub(crate) fn update(
        &mut self,
        update: &ViewUpdate,
        vs: &mut ViewState,
    ) -> Result<bool, BuildError> {
        let changed_ranges = update.changed_ranges.clone();
        if self.min_width > 0.0 && !changed_ranges.is_empty() {
            if !changed_ranges
                .iter()
                .all(|r| r.to_a < self.min_width_from || r.from_a > self.min_width_to)
            {
                self.min_width = 0.0;
                self.min_width_from = 0;
                self.min_width_to = 0;
            } else {
                self.min_width_from = update.changes.map_pos(self.min_width_from, 1);
                self.min_width_to = update.changes.map_pos(self.min_width_to, 1);
            }
        }
        let (decorations, wrappers) = update_deco(&update.state, vs)?;
        let disallow = vec![false; decorations.len()];
        lock_height(&self.content.content, vs)?;
        let redrawn = self.content.redraw(
            update.state.doc(),
            &update.changes,
            changed_ranges,
            decorations,
            wrappers,
            disallow,
        )?;
        if redrawn {
            vs.must_measure_content = MustMeasure::Yes;
        }
        self.unlock_height()?;
        Ok(redrawn)
    }

    // The end of `updateInner`: release the height lock and keep the
    // content at least `minWidth` wide.
    fn unlock_height(&self) -> Result<(), div::Error> {
        let dom = &self.content.content;
        set_style(dom, "height", "")?;
        let basis = if self.min_width != 0.0 {
            format!("{}px", js(self.min_width))
        } else {
            String::new()
        };
        set_style(dom, "flex-basis", &basis)
    }

    /// The heights of the blocks drawn in `viewport`, with negative
    /// entries for space above a block (`measureVisibleLineHeights`).
    pub(crate) fn measure_visible_line_heights(
        &mut self,
        viewport: Viewport,
        content_width: f64,
        scroll_width: f64,
        ltr: bool,
    ) -> Result<Vec<f64>, div::Error> {
        let mut scan = LineScan {
            result: Vec::new(),
            from: viewport.from,
            to: viewport.to,
            is_wider: content_width > scroll_width.max(self.min_width) + 1.0,
            widest: -1.0,
            ltr,
            space_above: 0.0,
            content_width,
            min_width: None,
        };
        scan.scan(&self.content.tiles, self.content.root, 0, None)?;
        if let Some((width, from, to)) = scan.min_width {
            self.min_width = width;
            self.min_width_from = from;
            self.min_width_to = to;
        }
        Ok(scan.result)
    }

    /// The line height, character width and text height of the content's
    /// font (`measureTextSize`).
    ///
    /// div-gap: 5 Range geometry. Upstream measures the text nodes of a
    /// short drawn line with `Range.getClientRects`, or else lays out a
    /// dummy line of 27 characters. Div cannot measure text nodes, so this
    /// always lays out the dummy line, with its text in a `<span>` whose
    /// box stands for the text's rectangle.
    pub(crate) fn measure_text_size(&self) -> Result<(f64, f64, f64), div::Error> {
        let content = &self.content.content;
        let document = &self.content.document;
        let dummy = crate::dom::create_element(document, "div")?;
        set_attribute(&dummy, "class", "cm-line")?;
        set_style(&dummy, "width", "99999px")?;
        set_style(&dummy, "position", "absolute")?;
        let text = crate::dom::create_element(document, "span")?;
        text.set_text("abc def ghi jkl mno pqr stu")?;
        dummy.append_child(&text)?;
        content.append_child(&dummy)?;
        let rect = bounding_rect(&text)?;
        let line_height = bounding_rect(&dummy)?.height();
        let char_width = if rect.width() != 0.0 {
            rect.width() / 27.0
        } else {
            7.0
        };
        let text_height = if rect.height() != 0.0 {
            rect.height()
        } else {
            line_height
        };
        dummy.remove()?;
        Ok((line_height, char_width, text_height))
    }

    /// The drawn line or block widget at `pos`, with the offset in it.
    pub(crate) fn block_at(&self, pos: usize, side: i32) -> (TileId, usize) {
        self.content
            .tiles
            .resolve_block(self.content.root, pos, side)
    }

    /// Remove the content's tiles and their DOM (`destroy`).
    pub(crate) fn destroy(self) -> Result<(), div::Error> {
        self.content.destroy()
    }
}

// The scan of `measureVisibleLineHeights`.
struct LineScan {
    result: Vec<f64>,
    from: usize,
    to: usize,
    is_wider: bool,
    widest: f64,
    ltr: bool,
    space_above: f64,
    content_width: f64,
    min_width: Option<(f64, usize, usize)>,
}

impl LineScan {
    fn scan(
        &mut self,
        tiles: &Tiles,
        tile: TileId,
        mut pos: usize,
        measure_bounds: Option<Rect>,
    ) -> Result<(), div::Error> {
        let children = &tiles[tile].children;
        for (i, &child) in children.iter().enumerate() {
            if pos > self.to {
                break;
            }
            let end = pos + tiles[child].length;
            let element = tiles[child].dom.element().expect("a block element");
            let child_rect = bounding_rect(element)?;
            let height = child_rect.height();
            if let Some(bounds) = measure_bounds {
                if i == 0 {
                    self.space_above += child_rect.top - bounds.top;
                }
            }
            if matches!(tiles[child].kind, TileKind::BlockWrapper(_)) {
                if end > self.from {
                    self.scan(tiles, child, pos, Some(child_rect))?;
                }
            } else if pos >= self.from {
                if self.space_above > 0.0 {
                    self.result.push(-self.space_above);
                }
                self.result.push(height + self.space_above);
                self.space_above = 0.0;
                if self.is_wider {
                    // `child.dom.lastChild`'s client rectangles. div-gap: 5
                    // Range geometry: a text node has none in Div, so only
                    // an element at the end of a line is measured.
                    let last = tiles
                        .last_child(child)
                        .and_then(|last| tiles[last].dom.element().cloned());
                    let rects = match last {
                        Some(last) => last.client_rects()?,
                        None => Vec::new(),
                    };
                    if let Some(rect) = rects.last() {
                        let (left, right) = (rect.x as f64, (rect.x + rect.width) as f64);
                        let width = if self.ltr {
                            right - child_rect.left
                        } else {
                            child_rect.right - left
                        };
                        if width > self.widest {
                            self.widest = width;
                            self.min_width = Some((self.content_width, pos, end));
                        }
                    }
                }
            }
            if let Some(bounds) = measure_bounds {
                if i == children.len() - 1 {
                    self.space_above += bounds.bottom - child_rect.bottom;
                }
            }
            pos = end + tiles.break_after(child);
        }
        Ok(())
    }
}

// `this.tile.dom.style.height = contentHeight / scaleY + "px"`: the
// content's height is locked while it is redrawn.
fn lock_height(content: &Element, vs: &ViewState) -> Result<(), div::Error> {
    set_style(
        content,
        "height",
        &format!("{}px", js(vs.content_height() / vs.scale_y)),
    )
}

/// The decorations and block wrappers to draw (`updateDeco`): the
/// formatting of an `EditContext` composition (none until M7), the
/// decorations, the outer decorations joined, the gap spacers and the
/// line gaps.
pub(crate) fn update_deco(
    state: &EditorState,
    vs: &ViewState,
) -> Result<(Vec<DecorationSet>, Vec<RangeSet<BlockWrapper>>), BuildError> {
    let mut decorations = vec![Decoration::none()];
    decorations.extend(state.facet(&DECORATIONS).iter().cloned());
    let outer = state.facet(&OUTER_DECORATIONS);
    if !outer.is_empty() {
        decorations.push(RangeSet::join(outer));
    }
    decorations.push(compute_block_gap_deco(state, vs)?);
    decorations.push(vs.line_gap_deco.clone());
    let wrappers = state.facet(&BLOCK_WRAPPERS).clone();
    Ok((decorations, wrappers))
}

/// Gap spacers for the parts of the document outside the viewports
/// (`computeBlockGapDeco`).
fn compute_block_gap_deco(
    state: &EditorState,
    vs: &ViewState,
) -> Result<DecorationSet, BuildError> {
    let mut deco = Vec::new();
    let mut pos = 0usize;
    let mut i = 0;
    loop {
        let next = vs.viewports.get(i).copied();
        let end = match next {
            Some(next) => next.from as i64 - 1,
            None => state.doc().len() as i64,
        };
        if end > pos as i64 {
            let end = end as usize;
            let height = (vs.line_block_at(end).bottom() - vs.line_block_at(pos).top) / vs.scale_y;
            deco.push(
                Decoration::replace(ReplaceSpec {
                    widget: Some(Widget::new(BlockGapWidget { height })),
                    block: true,
                    inclusive: Some(true),
                    is_block_gap: true,
                    ..Default::default()
                })
                .range(pos, end)
                .map_err(BuildError::Range)?,
            );
        }
        let Some(next) = next else {
            break;
        };
        pos = next.to + 1;
        i += 1;
    }
    Ok(Decoration::set(deco, false))
}

/// The spacer for an undrawn part of the document (`BlockGapWidget`).
struct BlockGapWidget {
    height: f64,
}

impl WidgetType for BlockGapWidget {
    fn to_dom(&self, document: &Document) -> Result<Element, div::Error> {
        let elt = crate::dom::create_element(document, "div")?;
        set_attribute(&elt, "class", "cm-gap")?;
        self.write_height(&elt)?;
        Ok(elt)
    }

    fn eq(&self, other: &Self) -> bool {
        other.height == self.height
    }

    fn update_dom(&self, dom: &Element, _document: &Document, _from: &Self) -> bool {
        self.write_height(dom).is_ok()
    }

    fn editable(&self) -> bool {
        true
    }

    fn estimated_height(&self) -> f64 {
        self.height
    }
}

impl BlockGapWidget {
    // `elt.style.height = this.height + "px"`.
    fn write_height(&self, elt: &Element) -> Result<(), div::Error> {
        set_attribute(elt, "style", &format!("height: {}px", js(self.height)))
    }
}

/// Write the start tag of `element` with the attributes `names`, in that
/// order, that it has (div-gap: 23, see [`ContentTiles::block_html`]).
pub(crate) fn open_tag_with(
    element: &Element,
    names: &[&str],
    out: &mut String,
) -> Result<(), div::Error> {
    out.push_str("<div");
    for &name in names {
        if let Some(value) = element.get_attribute(name)? {
            out.push(' ');
            out.push_str(name);
            out.push_str("=\"");
            escape(&value, true, out);
            out.push('"');
        }
    }
    out.push('>');
    Ok(())
}
