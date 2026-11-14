//! Port of `heightmap.ts`: the tree of line and widget heights the view
//! uses to place content it has not drawn, find the viewport and size the
//! gap spacers.
//!
//! Upstream's node classes (`HeightMapText`, `HeightMapBlock`,
//! `HeightMapGap`, `HeightMapBranch`) are one [`HeightMap`] type here. A
//! tree owns its nodes: the methods that rebuild part of it (`replace`,
//! `updateHeight`, `applyChanges`) take the tree by value and return the new
//! one, where upstream mutates nodes in place and returns `this` or a
//! replacement. Upstream's identity check in `replace(old, val)`, which
//! flags a height change when a node is replaced by one of another class,
//! compares the node kinds.
//!
//! Heights and tops are `f64` CSS pixels. Positions are `usize`, except
//! that the `value` of [`HeightMap::line_at`] is a position or a height
//! depending on the [`QueryType`], as upstream.

use std::cell::Cell;
use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, LazyLock};

use editor_state::{ChangeDesc, RangeComparator, RangeSet, SpanIterator, Text};

use crate::decoration::{add_range, BlockType, Decoration, DecorationSet, ReplaceSpec, Widget};
use crate::extension::ChangedRange;

const WRAPPING_WHITE_SPACE: [&str; 4] = ["pre-wrap", "normal", "pre-line", "break-spaces"];

thread_local! {
    // Tracks, during `update_height`, whether any actual height changed
    // (`heightChangeFlag`).
    static HEIGHT_CHANGE_FLAG: Cell<bool> = const { Cell::new(false) };
}

/// Reset the height-change flag (`clearHeightChangeFlag`).
pub fn clear_height_change_flag() {
    HEIGHT_CHANGE_FLAG.with(|flag| flag.set(false));
}

/// Whether a height changed since the flag was cleared
/// (`getHeightChangeFlag`).
pub fn height_change_flag() -> bool {
    HEIGHT_CHANGE_FLAG.with(Cell::get)
}

fn flag_height_change() {
    HEIGHT_CHANGE_FLAG.with(|flag| flag.set(true));
}

/// Estimates the heights of content that has not been measured
/// (`HeightOracle`).
#[derive(Clone)]
pub struct HeightOracle {
    /// The document the estimates are for.
    pub doc: Text,
    height_samples: HashSet<i64>,
    /// The height of an entire line (`line-height`).
    pub line_height: f64,
    /// The width of a character.
    pub char_width: f64,
    /// The height of the font itself (`font-size`).
    pub text_height: f64,
    /// The number of characters that fit on a line.
    pub line_length: f64,
    /// Whether lines wrap.
    pub line_wrapping: bool,
}

impl HeightOracle {
    /// An oracle with upstream's default metrics.
    pub fn new(line_wrapping: bool) -> HeightOracle {
        HeightOracle {
            doc: Text::empty(),
            height_samples: HashSet::new(),
            line_height: 14.0,
            char_width: 7.0,
            text_height: 14.0,
            line_length: 30.0,
            line_wrapping,
        }
    }

    /// The estimated height of the lines from `from` to `to`
    /// (`heightForGap`).
    pub fn height_for_gap(&self, from: usize, to: usize) -> f64 {
        let mut lines = (self.doc.line_at(to).number - self.doc.line_at(from).number + 1) as f64;
        if self.line_wrapping {
            lines += (((to - from) as f64 - lines * self.line_length * 0.5) / self.line_length)
                .ceil()
                .max(0.0);
        }
        self.line_height * lines
    }

    /// The estimated height of a line of `length` characters
    /// (`heightForLine`).
    pub fn height_for_line(&self, length: usize) -> f64 {
        if !self.line_wrapping {
            return self.line_height;
        }
        let lines = 1.0
            + ((length as f64 - self.line_length) / (self.line_length - 5.0).max(1.0))
                .ceil()
                .max(0.0);
        lines * self.line_height
    }

    /// Set the document (`setDoc`).
    pub fn set_doc(&mut self, doc: Text) -> &mut Self {
        self.doc = doc;
        self
    }

    /// Whether a `white-space` value changes whether lines wrap
    /// (`mustRefreshForWrapping`).
    pub fn must_refresh_for_wrapping(&self, white_space: &str) -> bool {
        WRAPPING_WHITE_SPACE.contains(&white_space) != self.line_wrapping
    }

    /// Record measured line heights, and whether any is new
    /// (`mustRefreshForHeights`). A negative entry is space above the next
    /// block, which is skipped.
    pub fn must_refresh_for_heights(&mut self, line_heights: &[f64]) -> bool {
        let mut new_height = false;
        let mut i = 0;
        while i < line_heights.len() {
            let h = line_heights[i];
            if h < 0.0 {
                i += 1;
            } else if self.height_samples.insert((h * 10.0).floor() as i64) {
                // Rounded to 0.1 px.
                new_height = true;
            }
            i += 1;
        }
        new_height
    }

    /// Update the metrics; returns whether the estimates changed
    /// (`refresh`).
    pub fn refresh(
        &mut self,
        white_space: &str,
        line_height: f64,
        char_width: f64,
        text_height: f64,
        line_length: f64,
        known_heights: &[f64],
    ) -> bool {
        let line_wrapping = WRAPPING_WHITE_SPACE.contains(&white_space);
        let changed =
            (line_height - self.line_height).abs() > 0.3 || self.line_wrapping != line_wrapping;
        self.line_wrapping = line_wrapping;
        self.line_height = line_height;
        self.char_width = char_width;
        self.text_height = text_height;
        self.line_length = line_length;
        if changed {
            self.height_samples.clear();
            let mut i = 0;
            while i < known_heights.len() {
                let h = known_heights[i];
                if h < 0.0 {
                    i += 1;
                } else {
                    self.height_samples.insert((h * 10.0).floor() as i64);
                }
                i += 1;
            }
        }
        changed
    }
}

/// Measured block heights from the DOM, starting at position `from`
/// (`MeasuredHeights`). A negative height is space above the next block.
#[derive(Clone, Debug)]
pub struct MeasuredHeights {
    /// The position of the first measured block.
    pub from: usize,
    /// The heights.
    pub heights: Vec<f64>,
    /// The next height to use.
    pub index: usize,
}

impl MeasuredHeights {
    /// Measured heights from `from`.
    pub fn new(from: usize, heights: Vec<f64>) -> MeasuredHeights {
        MeasuredHeights {
            from,
            heights,
            index: 0,
        }
    }

    /// Whether heights are left (`more`).
    pub fn more(&self) -> bool {
        self.index < self.heights.len()
    }

    fn next(&mut self) -> f64 {
        let height = self.heights[self.index];
        self.index += 1;
        height
    }
}

#[derive(Clone, Debug)]
enum BlockContent {
    Blocks(Vec<BlockInfo>),
    Deco(Arc<Decoration>),
    Breaks(usize),
}

/// What a [`BlockInfo`] is (`type` upstream).
#[derive(Clone, Copy, Debug)]
pub enum BlockKind<'a> {
    /// A single block of this type.
    Type(BlockType),
    /// When querying lines, the blocks that make up the line.
    Blocks(&'a [BlockInfo]),
}

/// A block-level element in the view (`BlockInfo`).
#[derive(Clone, Debug)]
pub struct BlockInfo {
    /// The start of the element in the document.
    pub from: usize,
    /// The length of the element.
    pub length: usize,
    /// The top of the element, relative to the top of the document.
    pub top: f64,
    /// Its height.
    pub height: f64,
    content: BlockContent,
}

impl BlockInfo {
    fn new(from: usize, length: usize, top: f64, height: f64, content: BlockContent) -> BlockInfo {
        BlockInfo {
            from,
            length,
            top,
            height,
            content,
        }
    }

    /// The type of element (`type`). A line query may return the blocks
    /// that make up the line.
    pub fn kind(&self) -> BlockKind<'_> {
        match &self.content {
            BlockContent::Breaks(_) => BlockKind::Type(BlockType::Text),
            BlockContent::Blocks(blocks) => BlockKind::Blocks(blocks),
            BlockContent::Deco(deco) => BlockKind::Type(deco.block_type()),
        }
    }

    /// The end of the element (`to`).
    pub fn to(&self) -> usize {
        self.from + self.length
    }

    /// The bottom of the element (`bottom`).
    pub fn bottom(&self) -> f64 {
        self.top + self.height
    }

    /// For a widget block, its widget (`widget`).
    pub fn widget(&self) -> Option<&Widget> {
        match &self.content {
            BlockContent::Deco(deco) => deco.widget_type(),
            _ => None,
        }
    }

    /// For a text block, the number of line breaks widgets in it add
    /// (`widgetLineBreaks`).
    pub fn widget_line_breaks(&self) -> usize {
        match self.content {
            BlockContent::Breaks(breaks) => breaks,
            _ => 0,
        }
    }

    /// This block followed by `other`, as one line (`join`).
    pub fn join(&self, other: &BlockInfo) -> BlockInfo {
        let mut content = match &self.content {
            BlockContent::Blocks(blocks) => blocks.clone(),
            _ => vec![self.clone()],
        };
        match &other.content {
            BlockContent::Blocks(blocks) => content.extend(blocks.iter().cloned()),
            _ => content.push(other.clone()),
        }
        BlockInfo::new(
            self.from,
            self.length + other.length,
            self.top,
            self.height + other.height,
            BlockContent::Blocks(content),
        )
    }
}

/// How [`HeightMap::line_at`] reads its `value` (`QueryType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryType {
    /// A document position.
    ByPos,
    /// A height.
    ByHeight,
    /// A document position, without computing heights.
    ByPosNoHeight,
}

const BREAK: u8 = 1;
const OUTDATED: u8 = 2;
const SINGLE_LINE: u8 = 4;

const EPSILON: f64 = 1e-3;

#[derive(Clone, Copy, PartialEq, Eq)]
enum KindId {
    Text,
    Block,
    Gap,
    Branch,
}

struct TextLine {
    space_above: f64,
    // Amount of collapsed content in the line.
    collapsed: usize,
    // Maximum inline widget height.
    widget_height: f64,
    // Number of widget-introduced line breaks on the line.
    breaks: usize,
}

struct BlockNode {
    space_above: f64,
    deco: Option<Arc<Decoration>>,
}

struct Branch {
    left: HeightMap,
    right: HeightMap,
    size: usize,
}

enum Node {
    Text(TextLine),
    Block(BlockNode),
    Gap,
    Branch(Box<Branch>),
}

/// A tree of the heights of the document's lines and blocks
/// (`HeightMap`). Leaves are lines, block widgets, and gaps of content
/// whose height is only estimated.
pub struct HeightMap {
    length: usize,
    height: f64,
    flags: u8,
    node: Node,
}

// `SpaceDeco`: the decoration of the space above a block.
static SPACE_DECO: LazyLock<Arc<Decoration>> =
    LazyLock::new(|| Decoration::replace(ReplaceSpec::default()));

impl HeightMap {
    fn text(length: usize, height: f64, above: f64) -> HeightMap {
        HeightMap {
            length,
            height,
            flags: OUTDATED,
            node: Node::Text(TextLine {
                space_above: above,
                collapsed: 0,
                widget_height: 0.0,
                breaks: 0,
            }),
        }
    }

    fn block(length: usize, height: f64, deco: Option<Arc<Decoration>>) -> HeightMap {
        HeightMap {
            length,
            height,
            flags: OUTDATED,
            node: Node::Block(BlockNode {
                space_above: 0.0,
                deco,
            }),
        }
    }

    fn gap(length: usize) -> HeightMap {
        HeightMap {
            length,
            height: 0.0,
            flags: OUTDATED,
            node: Node::Gap,
        }
    }

    fn branch(left: HeightMap, brk: bool, right: HeightMap) -> HeightMap {
        let flags = if brk { BREAK } else { 0 }
            | if left.outdated() || right.outdated() {
                OUTDATED
            } else {
                0
            };
        HeightMap {
            length: left.length + usize::from(brk) + right.length,
            height: left.height + right.height,
            flags,
            node: Node::Branch(Box::new(Branch {
                size: left.size() + right.size(),
                left,
                right,
            })),
        }
    }

    /// An empty map (`HeightMap.empty`).
    pub fn empty() -> HeightMap {
        HeightMap::text(0, 0.0, 0.0)
    }

    /// The number of characters covered.
    pub fn length(&self) -> usize {
        self.length
    }

    /// The height of this part of the document.
    pub fn height(&self) -> f64 {
        self.height
    }

    /// The number of leaves (`size`).
    pub fn size(&self) -> usize {
        match &self.node {
            Node::Branch(branch) => branch.size,
            _ => 1,
        }
    }

    /// Whether heights need recomputing (`outdated`).
    pub fn outdated(&self) -> bool {
        self.flags & OUTDATED > 0
    }

    fn set_outdated(&mut self, value: bool) {
        self.flags = if value { OUTDATED } else { 0 } | (self.flags & !OUTDATED);
    }

    fn brk(&self) -> usize {
        usize::from(self.flags & BREAK > 0)
    }

    /// A branch's two children, for tests that check the tree's balance.
    #[doc(hidden)]
    pub fn children(&self) -> Option<(&HeightMap, &HeightMap)> {
        match &self.node {
            Node::Branch(branch) => Some((&branch.left, &branch.right)),
            _ => None,
        }
    }

    fn kind_id(&self) -> KindId {
        match self.node {
            Node::Text(_) => KindId::Text,
            Node::Block(_) => KindId::Block,
            Node::Gap => KindId::Gap,
            Node::Branch(_) => KindId::Branch,
        }
    }

    fn set_height(&mut self, height: f64) {
        if self.height != height {
            if (self.height - height).abs() > EPSILON {
                flag_height_change();
            }
            self.height = height;
        }
    }

    fn space_above(&self) -> f64 {
        match &self.node {
            Node::Text(text) => text.space_above,
            Node::Block(block) => block.space_above,
            _ => 0.0,
        }
    }

    fn set_space_above(&mut self, value: f64) {
        match &mut self.node {
            Node::Text(text) => text.space_above = value,
            Node::Block(block) => block.space_above = value,
            _ => {}
        }
    }

    // Leaf blocks: `mainBlock`.
    fn main_block(&self, top: f64, offset: usize) -> BlockInfo {
        let space_above = self.space_above();
        let content = match &self.node {
            Node::Text(text) => BlockContent::Breaks(text.breaks),
            Node::Block(BlockNode {
                deco: Some(deco), ..
            }) => BlockContent::Deco(deco.clone()),
            _ => BlockContent::Breaks(0),
        };
        BlockInfo::new(
            offset,
            self.length,
            top + space_above,
            self.height - space_above,
            content,
        )
    }

    /// The block at `height` (`blockAt`).
    pub fn block_at(
        &self,
        height: f64,
        oracle: &HeightOracle,
        top: f64,
        offset: usize,
    ) -> BlockInfo {
        match &self.node {
            Node::Text(_) | Node::Block(_) => {
                let space_above = self.space_above();
                if space_above != 0.0 && height < top + space_above {
                    BlockInfo::new(
                        offset,
                        0,
                        top,
                        space_above,
                        BlockContent::Deco(SPACE_DECO.clone()),
                    )
                } else {
                    self.main_block(top, offset)
                }
            }
            Node::Gap => self.gap_block_at(height, oracle, top, offset),
            Node::Branch(branch) => {
                let mid = top + branch.left.height;
                if height < mid {
                    branch.left.block_at(height, oracle, top, offset)
                } else {
                    branch.right.block_at(
                        height,
                        oracle,
                        mid,
                        offset + branch.left.length + self.brk(),
                    )
                }
            }
        }
    }

    /// The line at `value`, a position or a height depending on `ty`
    /// (`lineAt`). Blocks on the same line are joined.
    pub fn line_at(
        &self,
        value: f64,
        ty: QueryType,
        oracle: &HeightOracle,
        top: f64,
        offset: usize,
    ) -> BlockInfo {
        match &self.node {
            Node::Text(_) | Node::Block(_) => {
                let main = self.main_block(top, offset);
                if self.space_above() != 0.0 {
                    self.block_at(0.0, oracle, top, offset).join(&main)
                } else {
                    main
                }
            }
            Node::Gap => self.gap_line_at(value, ty, oracle, top, offset),
            Node::Branch(branch) => {
                let right_top = top + branch.left.height;
                let right_offset = offset + branch.left.length + self.brk();
                let left = if ty == QueryType::ByHeight {
                    value < right_top
                } else {
                    value < right_offset as f64
                };
                let base = if left {
                    branch.left.line_at(value, ty, oracle, top, offset)
                } else {
                    branch
                        .right
                        .line_at(value, ty, oracle, right_top, right_offset)
                };
                if self.brk() > 0
                    || if left {
                        base.to() < right_offset
                    } else {
                        base.from > right_offset
                    }
                {
                    return base;
                }
                let sub_query = if ty == QueryType::ByPosNoHeight {
                    QueryType::ByPosNoHeight
                } else {
                    QueryType::ByPos
                };
                if left {
                    base.join(&branch.right.line_at(
                        right_offset as f64,
                        sub_query,
                        oracle,
                        right_top,
                        right_offset,
                    ))
                } else {
                    branch
                        .left
                        .line_at(right_offset as f64, sub_query, oracle, top, offset)
                        .join(&base)
                }
            }
        }
    }

    /// Call `f` for each line from `from` to `to` (`forEachLine`).
    pub fn for_each_line(
        &self,
        from: usize,
        to: usize,
        oracle: &HeightOracle,
        top: f64,
        offset: usize,
        f: &mut dyn FnMut(BlockInfo),
    ) {
        match &self.node {
            Node::Text(_) | Node::Block(_) => {
                if from <= offset + self.length && to >= offset {
                    f(self.line_at(0.0, QueryType::ByPos, oracle, top, offset));
                }
            }
            Node::Gap => self.gap_for_each_line(from, to, oracle, top, offset, f),
            Node::Branch(branch) => {
                let right_top = top + branch.left.height;
                let right_offset = offset + branch.left.length + self.brk();
                if self.brk() > 0 {
                    if from < right_offset {
                        branch.left.for_each_line(from, to, oracle, top, offset, f);
                    }
                    if to >= right_offset {
                        branch
                            .right
                            .for_each_line(from, to, oracle, right_top, right_offset, f);
                    }
                } else {
                    let mid =
                        self.line_at(right_offset as f64, QueryType::ByPos, oracle, top, offset);
                    if from < mid.from {
                        branch.left.for_each_line(
                            from,
                            to.min(mid.from - 1),
                            oracle,
                            top,
                            offset,
                            f,
                        );
                    }
                    let mid_to = mid.to();
                    if mid_to >= from && mid.from <= to {
                        f(mid);
                    }
                    if to > mid_to {
                        branch.right.for_each_line(
                            from.max(mid_to + 1),
                            to,
                            oracle,
                            right_top,
                            right_offset,
                            f,
                        );
                    }
                }
            }
        }
    }

    // Gaps: `heightMetrics`, giving the first and last line numbers and
    // the height per line and per character.
    fn height_metrics(&self, oracle: &HeightOracle, offset: usize) -> (usize, usize, f64, f64) {
        let first_line = oracle.doc.line_at(offset).number;
        let last_line = oracle.doc.line_at(offset + self.length).number;
        let lines = (last_line - first_line + 1) as f64;
        let mut per_char = 0.0;
        let per_line;
        if oracle.line_wrapping {
            let total_per_line = self.height.min(oracle.line_height * lines);
            per_line = total_per_line / lines;
            if self.length as f64 > lines + 1.0 {
                per_char = (self.height - total_per_line) / (self.length as f64 - lines - 1.0);
            }
        } else {
            per_line = self.height / lines;
        }
        (first_line, last_line, per_line, per_char)
    }

    fn gap_block_at(
        &self,
        height: f64,
        oracle: &HeightOracle,
        top: f64,
        offset: usize,
    ) -> BlockInfo {
        let (first_line, last_line, per_line, per_char) = self.height_metrics(oracle, offset);
        if oracle.line_wrapping {
            let guess = offset
                + if height < oracle.line_height {
                    0
                } else {
                    (((height - top) / self.height).clamp(0.0, 1.0) * self.length as f64).round()
                        as usize
                };
            let line = oracle.doc.line_at(guess);
            let line_height = per_line + line.len() as f64 * per_char;
            let line_top = top.max(height - line_height / 2.0);
            BlockInfo::new(
                line.from,
                line.len(),
                line_top,
                line_height,
                BlockContent::Breaks(0),
            )
        } else {
            let line = ((height - top) / per_line)
                .floor()
                .min((last_line - first_line) as f64)
                .max(0.0) as usize;
            let found = oracle.doc.line(first_line + line);
            BlockInfo::new(
                found.from,
                found.len(),
                top + per_line * line as f64,
                per_line,
                BlockContent::Breaks(0),
            )
        }
    }

    fn gap_line_at(
        &self,
        value: f64,
        ty: QueryType,
        oracle: &HeightOracle,
        top: f64,
        offset: usize,
    ) -> BlockInfo {
        if ty == QueryType::ByHeight {
            return self.gap_block_at(value, oracle, top, offset);
        }
        if ty == QueryType::ByPosNoHeight {
            let line = oracle.doc.line_at(value as usize);
            return BlockInfo::new(
                line.from,
                line.to - line.from,
                0.0,
                0.0,
                BlockContent::Breaks(0),
            );
        }
        let (first_line, _, per_line, per_char) = self.height_metrics(oracle, offset);
        let line = oracle.doc.line_at(value as usize);
        let line_height = per_line + line.len() as f64 * per_char;
        let lines_above = (line.number - first_line) as f64;
        let line_top = top
            + per_line * lines_above
            + per_char * (line.from as f64 - offset as f64 - lines_above);
        BlockInfo::new(
            line.from,
            line.len(),
            top.max(line_top.min(top + self.height - line_height)),
            line_height,
            BlockContent::Breaks(0),
        )
    }

    fn gap_for_each_line(
        &self,
        from: usize,
        to: usize,
        oracle: &HeightOracle,
        top: f64,
        offset: usize,
        f: &mut dyn FnMut(BlockInfo),
    ) {
        let from = from.max(offset);
        let to = to.min(offset + self.length);
        let (first_line, _, per_line, per_char) = self.height_metrics(oracle, offset);
        let (mut pos, mut line_top) = (from, top);
        while pos <= to {
            let line = oracle.doc.line_at(pos);
            if pos == from {
                let lines_above = (line.number - first_line) as f64;
                line_top +=
                    per_line * lines_above + per_char * (from as f64 - offset as f64 - lines_above);
            }
            let line_height = per_line + per_char * line.len() as f64;
            f(BlockInfo::new(
                line.from,
                line.len(),
                line_top,
                line_height,
                BlockContent::Breaks(0),
            ));
            line_top += line_height;
            pos = line.to + 1;
        }
    }

    // Leaf blocks: `setMeasuredHeight`.
    fn set_measured_height(&mut self, measured: &mut MeasuredHeights) {
        let mut next = measured.next();
        if next < 0.0 {
            self.set_space_above(-next);
            next = measured.next();
        } else {
            self.set_space_above(0.0);
        }
        self.set_height(next);
    }

    /// Recompute heights from the oracle where outdated (or everywhere
    /// with `force`), and from `measured` where it covers
    /// (`updateHeight`).
    pub fn update_height(
        mut self,
        oracle: &HeightOracle,
        offset: usize,
        force: bool,
        measured: Option<&mut MeasuredHeights>,
    ) -> HeightMap {
        match self.node {
            Node::Text(_) | Node::Block(_) => {
                self.update_leaf_height(oracle, offset, force, measured);
                self
            }
            Node::Gap => self.update_gap_height(oracle, offset, force, measured),
            Node::Branch(_) => self.update_branch_height(oracle, offset, force, measured),
        }
    }

    fn update_leaf_height(
        &mut self,
        oracle: &HeightOracle,
        offset: usize,
        force: bool,
        measured: Option<&mut MeasuredHeights>,
    ) {
        match measured {
            Some(measured) if measured.from <= offset && measured.more() => {
                self.set_measured_height(measured);
            }
            _ => {
                if let Node::Text(text) = &mut self.node {
                    if force || self.flags & OUTDATED > 0 {
                        text.space_above = 0.0;
                        let height = text
                            .widget_height
                            .max(oracle.height_for_line(self.length - text.collapsed))
                            + text.breaks as f64 * oracle.line_height;
                        self.set_height(height);
                    }
                }
            }
        }
        self.set_outdated(false);
    }

    fn update_gap_height(
        mut self,
        oracle: &HeightOracle,
        offset: usize,
        force: bool,
        measured: Option<&mut MeasuredHeights>,
    ) -> HeightMap {
        let end = offset + self.length;
        match measured {
            Some(measured) if measured.from <= end && measured.more() => {
                // Fill in part of this gap with measured lines. Gaps hold
                // only plain text, so those lines have no widgets or
                // collapsed ranges.
                let mut nodes = Vec::new();
                let mut pos = offset.max(measured.from);
                let mut single_height = -1.0;
                if measured.from > offset {
                    nodes.push(Some(
                        HeightMap::gap(measured.from - offset - 1)
                            .update_height(oracle, offset, false, None),
                    ));
                }
                while pos <= end && measured.more() {
                    let len = oracle.doc.line_at(pos).len();
                    if !nodes.is_empty() {
                        nodes.push(None);
                    }
                    let mut height = measured.next();
                    let mut above = 0.0;
                    if height < 0.0 {
                        above = -height;
                        height = measured.next();
                    }
                    if single_height == -1.0 {
                        single_height = height;
                    } else if (height - single_height).abs() >= EPSILON {
                        single_height = -2.0;
                    }
                    let mut line = HeightMap::text(len, height, above);
                    line.set_outdated(false);
                    nodes.push(Some(line));
                    pos += len + 1;
                }
                if pos <= end {
                    nodes.push(None);
                    nodes.push(Some(
                        HeightMap::gap(end - pos).update_height(oracle, pos, false, None),
                    ));
                }
                let result = HeightMap::of(nodes);
                let (_, _, per_line, _) = self.height_metrics(oracle, offset);
                if single_height < 0.0
                    || (result.height - self.height).abs() >= EPSILON
                    || (single_height - per_line).abs() >= EPSILON
                {
                    flag_height_change();
                }
                replaced(KindId::Gap, result)
            }
            _ => {
                if force || self.outdated() {
                    let height = oracle.height_for_gap(offset, end);
                    self.set_height(height);
                    self.set_outdated(false);
                }
                self
            }
        }
    }

    fn update_branch_height(
        self,
        oracle: &HeightOracle,
        offset: usize,
        force: bool,
        mut measured: Option<&mut MeasuredHeights>,
    ) -> HeightMap {
        let HeightMap {
            length,
            height,
            flags,
            node: Node::Branch(branch),
        } = self
        else {
            unreachable!()
        };
        let Branch {
            mut left,
            mut right,
            size,
        } = *branch;
        let brk = usize::from(flags & BREAK > 0);
        let right_start = offset + left.length + brk;
        let (old_left, old_right) = (left.kind_id(), right.kind_id());
        let mut rebalance = false;
        let left_end = offset + left.length;
        if measured
            .as_deref()
            .is_some_and(|m| m.from <= left_end && m.more())
        {
            rebalance = true;
            left = left.update_height(oracle, offset, force, measured.as_deref_mut());
        } else {
            left = left.update_height(oracle, offset, force, None);
        }
        let right_end = right_start + right.length;
        if measured
            .as_deref()
            .is_some_and(|m| m.from <= right_end && m.more())
        {
            rebalance = true;
            right = right.update_height(oracle, right_start, force, measured);
        } else {
            right = right.update_height(oracle, right_start, force, None);
        }
        if rebalance {
            return HeightMap::balanced(height, flags, left, right, old_left, old_right);
        }
        let mut me = HeightMap {
            length,
            height: left.height + right.height,
            flags,
            node: Node::Branch(Box::new(Branch { left, right, size })),
        };
        me.set_outdated(false);
        me
    }

    // Branches: `balanced`, given the branch's old height and flags.
    fn balanced(
        height: f64,
        flags: u8,
        left: HeightMap,
        right: HeightMap,
        old_left: KindId,
        old_right: KindId,
    ) -> HeightMap {
        let brk = flags & BREAK > 0;
        if left.size() > 2 * right.size() || right.size() > 2 * left.size() {
            return HeightMap::of(if brk {
                vec![Some(left), None, Some(right)]
            } else {
                vec![Some(left), Some(right)]
            });
        }
        if left.kind_id() != old_left || right.kind_id() != old_right {
            flag_height_change();
        }
        let outdated = left.outdated() || right.outdated();
        let mut me = HeightMap {
            length: left.length + usize::from(brk) + right.length,
            height,
            flags,
            node: Node::Gap,
        };
        me.set_height(left.height + right.height);
        me.set_outdated(outdated);
        me.node = Node::Branch(Box::new(Branch {
            size: left.size() + right.size(),
            left,
            right,
        }));
        me
    }

    // `replace`: replace `from..to` with `nodes`, which use `None` for
    // line breaks.
    fn replace(self, from: usize, to: usize, mut nodes: Vec<Option<HeightMap>>) -> HeightMap {
        match self.node {
            Node::Text(_) => {
                let this_length = self.length;
                let fits = nodes.len() == 1
                    && nodes[0].as_ref().is_some_and(|node| {
                        (matches!(node.node, Node::Text(_))
                            || matches!(node.node, Node::Gap) && node.flags & SINGLE_LINE > 0)
                            && this_length.abs_diff(node.length) < 10
                    });
                if !fits {
                    return HeightMap::of(nodes);
                }
                let mut node = nodes.pop().flatten().expect("checked above");
                if matches!(node.node, Node::Gap) {
                    node = HeightMap::text(node.length, self.height, self.space_above());
                } else {
                    node.height = self.height;
                }
                if !self.outdated() {
                    node.set_outdated(false);
                }
                node
            }
            Node::Block(_) => HeightMap::of(nodes),
            Node::Gap => {
                let after = self.length - to;
                if after > 0 {
                    match nodes.last_mut() {
                        Some(Some(last)) if matches!(last.node, Node::Gap) => {
                            *last = HeightMap::gap(last.length + after);
                        }
                        _ => {
                            nodes.push(None);
                            nodes.push(Some(HeightMap::gap(after - 1)));
                        }
                    }
                }
                if from > 0 {
                    match nodes.first_mut() {
                        Some(Some(first)) if matches!(first.node, Node::Gap) => {
                            *first = HeightMap::gap(from + first.length);
                        }
                        _ => {
                            nodes.insert(0, None);
                            nodes.insert(0, Some(HeightMap::gap(from - 1)));
                        }
                    }
                }
                HeightMap::of(nodes)
            }
            Node::Branch(branch) => {
                let (height, flags, length) = (self.height, self.flags, self.length);
                let Branch { left, right, .. } = *branch;
                let brk = usize::from(flags & BREAK > 0);
                let right_start = left.length + brk;
                let (old_left, old_right) = (left.kind_id(), right.kind_id());
                if to < right_start {
                    let left = left.replace(from, to, nodes);
                    return HeightMap::balanced(height, flags, left, right, old_left, old_right);
                }
                if from > left.length {
                    let right = right.replace(from - right_start, to - right_start, nodes);
                    return HeightMap::balanced(height, flags, left, right, old_left, old_right);
                }
                // The replaced range spans the break between the
                // children, so upstream's `this.decomposeLeft(from)` only
                // reads the left child and `this.decomposeRight(to)` only
                // the right one.
                let mut result = Vec::new();
                if from > 0 {
                    left.decompose_left(from, &mut result);
                }
                let left_end = result.len();
                result.extend(nodes);
                if from > 0 {
                    merge_gaps(&mut result, left_end as isize - 1);
                }
                if to < length {
                    let right_begin = result.len();
                    right.decompose_right(to - right_start, &mut result);
                    merge_gaps(&mut result, right_begin as isize);
                }
                HeightMap::of(result)
            }
        }
    }

    // `decomposeLeft`: push the nodes covering `0..to`.
    fn decompose_left(self, to: usize, result: &mut Vec<Option<HeightMap>>) {
        match self.node {
            Node::Gap => {
                result.push(Some(HeightMap::gap(to - 1)));
                result.push(None);
            }
            Node::Branch(branch) => {
                let brk = self.flags & BREAK > 0;
                let Branch { left, right, .. } = *branch;
                let mut left_len = left.length;
                if to <= left_len {
                    return left.decompose_left(to, result);
                }
                result.push(Some(left));
                if brk {
                    left_len += 1;
                    if to >= left_len {
                        result.push(None);
                    }
                }
                if to > left_len {
                    right.decompose_left(to - left_len, result);
                }
            }
            _ => result.push(Some(self)),
        }
    }

    // `decomposeRight`: push the nodes covering `from..`.
    fn decompose_right(self, from: usize, result: &mut Vec<Option<HeightMap>>) {
        match self.node {
            Node::Gap => {
                result.push(None);
                result.push(Some(HeightMap::gap(self.length - from - 1)));
            }
            Node::Branch(branch) => {
                let brk = self.flags & BREAK > 0;
                let Branch { left, right, .. } = *branch;
                let left_len = left.length;
                let right_start = left_len + usize::from(brk);
                if from >= right_start {
                    return right.decompose_right(from - right_start, result);
                }
                if from < left_len {
                    left.decompose_right(from, result);
                }
                if brk && from < right_start {
                    result.push(None);
                }
                result.push(Some(right));
            }
            _ => result.push(Some(self)),
        }
    }

    /// Bring the map up to date with document changes and decorations
    /// (`applyChanges`). `oracle.doc` must be the new document; the
    /// oracle's document is the new one again afterwards.
    pub fn apply_changes(
        self,
        decorations: &[DecorationSet],
        old_doc: &Text,
        oracle: &mut HeightOracle,
        changes: &[ChangedRange],
    ) -> HeightMap {
        let mut me = self;
        let doc = oracle.doc.clone();
        let mut i = changes.len();
        while i > 0 {
            i -= 1;
            let ChangedRange {
                mut from_a,
                mut to_a,
                mut from_b,
                mut to_b,
            } = changes[i];
            oracle.set_doc(old_doc.clone());
            let mut start = me.line_at(from_a as f64, QueryType::ByPosNoHeight, oracle, 0.0, 0);
            let end = if start.to() >= to_a {
                start.clone()
            } else {
                me.line_at(to_a as f64, QueryType::ByPosNoHeight, oracle, 0.0, 0)
            };
            to_b = (to_b as i64 + end.to() as i64 - to_a as i64) as usize;
            to_a = end.to();
            while i > 0 && start.from <= changes[i - 1].to_a {
                from_a = changes[i - 1].from_a;
                from_b = changes[i - 1].from_b;
                i -= 1;
                if from_a < start.from {
                    start = me.line_at(from_a as f64, QueryType::ByPosNoHeight, oracle, 0.0, 0);
                }
            }
            from_b = (from_b as i64 + start.from as i64 - from_a as i64) as usize;
            from_a = start.from;
            oracle.set_doc(doc.clone());
            let nodes = NodeBuilder::build(oracle, decorations, from_b, to_b);
            let old = me.kind_id();
            me = replaced(old, me.replace(from_a, to_a, nodes));
        }
        me.update_height(oracle, 0, false, None)
    }

    // `HeightMap.of`: build a balanced tree from nodes that use `None` for
    // line breaks. There are never breaks at the start or end, or two in a
    // row, and the list is not empty.
    fn of(mut nodes: Vec<Option<HeightMap>>) -> HeightMap {
        if nodes.len() == 1 {
            return nodes
                .pop()
                .flatten()
                .expect("HeightMap.of: a line break alone");
        }
        let (mut i, mut j, mut before, mut after) = (0usize, nodes.len(), 0usize, 0usize);
        loop {
            if i == j {
                if before > after * 2 {
                    i -= 1;
                    let (split_size, parts) = split_branch(nodes[i].take());
                    let added = parts.len() - 1;
                    nodes.splice(i..i + 1, parts);
                    j += added;
                    before -= split_size;
                } else if after > before * 2 {
                    let (split_size, parts) = split_branch(nodes[j].take());
                    let added = parts.len() - 1;
                    nodes.splice(j..j + 1, parts);
                    j += added + 1;
                    after -= split_size;
                } else {
                    break;
                }
            } else if before < after {
                if let Some(next) = &nodes[i] {
                    before += next.size();
                }
                i += 1;
            } else {
                j -= 1;
                if let Some(next) = &nodes[j] {
                    after += next.size();
                }
            }
        }
        let mut brk = false;
        if nodes[i - 1].is_none() {
            brk = true;
            i -= 1;
        } else if nodes.get(i).is_none_or(Option::is_none) {
            brk = true;
            j += 1;
        }
        let right = nodes.split_off(j);
        nodes.truncate(i);
        HeightMap::branch(HeightMap::of(nodes), brk, HeightMap::of(right))
    }
}

// Split a branch into its parts for `HeightMap.of`: its size, and left,
// a break if it has one, and right.
fn split_branch(node: Option<HeightMap>) -> (usize, Vec<Option<HeightMap>>) {
    let node = node.expect("HeightMap.of: splitting a line break");
    let size = node.size();
    let brk = node.flags & BREAK > 0;
    let Node::Branch(branch) = node.node else {
        panic!("HeightMap.of: splitting a leaf");
    };
    let Branch { left, right, .. } = *branch;
    let parts = if brk {
        vec![Some(left), None, Some(right)]
    } else {
        vec![Some(left), Some(right)]
    };
    (size, parts)
}

// `replace(old, val)`: a node replaced by one of another kind is a height
// change.
fn replaced(old: KindId, val: HeightMap) -> HeightMap {
    if val.kind_id() != old {
        flag_height_change();
    }
    val
}

fn is_gap(node: Option<&Option<HeightMap>>) -> bool {
    matches!(
        node,
        Some(Some(HeightMap {
            node: Node::Gap,
            ..
        }))
    )
}

// `mergeGaps`: join two gaps around a line break at `around`.
fn merge_gaps(nodes: &mut Vec<Option<HeightMap>>, around: isize) {
    if around < 1 {
        return;
    }
    let around = around as usize;
    if matches!(nodes.get(around), Some(None))
        && is_gap(nodes.get(around - 1))
        && is_gap(nodes.get(around + 1))
    {
        let before = nodes[around - 1].as_ref().map_or(0, |n| n.length);
        let after = nodes[around + 1].as_ref().map_or(0, |n| n.length);
        nodes.splice(
            around - 1..around + 2,
            [Some(HeightMap::gap(before + 1 + after))],
        );
    }
}

impl fmt::Display for HeightMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.node {
            Node::Text(text) => {
                write!(f, "line({}", self.length)?;
                if text.collapsed != 0 {
                    write!(f, "-{}", text.collapsed)?;
                }
                if text.widget_height != 0.0 {
                    write!(f, ":{}", text.widget_height)?;
                }
                write!(f, ")")
            }
            Node::Block(_) => write!(f, "block({})", self.length),
            Node::Gap => write!(f, "gap({})", self.length),
            Node::Branch(branch) => write!(
                f,
                "{}{}{}",
                branch.left,
                if self.brk() > 0 { " " } else { "-" },
                branch.right
            ),
        }
    }
}

impl fmt::Debug for HeightMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HeightMap({self})")
    }
}

const RELEVANT_WIDGET_HEIGHT: f64 = 5.0;

// `NodeBuilder`: builds height map nodes from the decorations in a range.
struct NodeBuilder<'o> {
    nodes: Vec<Option<HeightMap>>,
    written_to: usize,
    line_start: i64,
    line_end: i64,
    // The index of the block that covers the content after it, which is
    // `covering` upstream; it covers only while it is the last node.
    covering: Option<usize>,
    pos: usize,
    oracle: &'o HeightOracle,
}

impl<'o> NodeBuilder<'o> {
    fn new(pos: usize, oracle: &'o HeightOracle) -> Self {
        NodeBuilder {
            nodes: Vec::new(),
            written_to: pos,
            line_start: -1,
            line_end: -1,
            covering: None,
            pos,
            oracle,
        }
    }

    fn is_covered(&self) -> bool {
        self.covering.is_some() && self.covering == self.nodes.len().checked_sub(1)
    }

    fn last_is_text(&self) -> bool {
        matches!(
            self.nodes.last(),
            Some(Some(HeightMap {
                node: Node::Text(_),
                ..
            }))
        )
    }

    // `nodes[nodes.length - 1] == null`, which an empty list also meets.
    fn last_is_null(&self) -> bool {
        self.nodes.last().is_none_or(Option::is_none)
    }

    fn enter_line(&mut self) {
        if self.line_start > -1 {
            return;
        }
        let line = self.oracle.doc.line_at(self.pos);
        let (from, to) = (line.from, line.to);
        self.line_start = from as i64;
        self.line_end = to as i64;
        if self.written_to < from {
            if self.written_to + 1 < from || self.last_is_null() {
                let blank = self.blank_content(self.written_to, from - 1);
                self.nodes.push(Some(blank));
            }
            self.nodes.push(None);
        }
        if self.pos > from {
            self.nodes
                .push(Some(HeightMap::text(self.pos - from, -1.0, 0.0)));
        }
        self.written_to = self.pos;
    }

    fn blank_content(&self, from: usize, to: usize) -> HeightMap {
        let mut gap = HeightMap::gap(to - from);
        if self.oracle.doc.line_at(from).to == to {
            gap.flags |= SINGLE_LINE;
        }
        gap
    }

    // `ensureLine`: the index of the text node the content goes in.
    fn ensure_line(&mut self) -> usize {
        self.enter_line();
        if !self.last_is_text() {
            self.nodes.push(Some(HeightMap::text(0, -1.0, 0.0)));
        }
        self.nodes.len() - 1
    }

    fn add_block(&mut self, block: HeightMap) {
        self.enter_line();
        let (start_side, end_side) = match &block.node {
            Node::Block(BlockNode {
                deco: Some(deco), ..
            }) => (
                Some(editor_state::RangeValue::start_side(&**deco)),
                Some(editor_state::RangeValue::end_side(&**deco)),
            ),
            _ => (None, None),
        };
        if start_side.is_some_and(|side| side > 0) && !self.is_covered() {
            self.ensure_line();
        }
        let length = block.length;
        self.nodes.push(Some(block));
        self.pos += length;
        self.written_to = self.pos;
        if end_side.is_some_and(|side| side > 0) {
            self.covering = Some(self.nodes.len() - 1);
        }
    }

    fn add_line_deco(&mut self, height: f64, breaks: usize, length: usize) {
        let index = self.ensure_line();
        let line = self.nodes[index].as_mut().expect("a text node");
        line.length += length;
        if let Node::Text(text) = &mut line.node {
            text.collapsed += length;
            text.widget_height = text.widget_height.max(height);
            text.breaks += breaks;
        }
        self.pos += length;
        self.written_to = self.pos;
    }

    fn finish(mut self, from: usize) -> Vec<Option<HeightMap>> {
        if self.line_start > -1 && !self.last_is_text() && !self.is_covered() {
            self.nodes.push(Some(HeightMap::text(0, -1.0, 0.0)));
        } else if self.written_to < self.pos || self.last_is_null() {
            let blank = self.blank_content(self.written_to, self.pos);
            self.nodes.push(Some(blank));
        }
        let mut pos = from;
        for node in &mut self.nodes {
            match node {
                Some(node) => {
                    if matches!(node.node, Node::Text(_)) {
                        node.update_leaf_height(self.oracle, pos, false, None);
                    }
                    pos += node.length;
                }
                None => pos += 1,
            }
        }
        self.nodes
    }

    // Always called with a range that reaches a line break or the end of
    // the document on both sides. The result uses `None` for line breaks,
    // never at the start or end or twice in a row.
    fn build(
        oracle: &HeightOracle,
        decorations: &[DecorationSet],
        from: usize,
        to: usize,
    ) -> Vec<Option<HeightMap>> {
        let mut builder = NodeBuilder::new(from, oracle);
        RangeSet::spans(decorations, from, to, &mut builder, 0);
        builder.finish(from)
    }
}

impl SpanIterator<Decoration> for NodeBuilder<'_> {
    fn span(&mut self, _from: usize, to: usize, _active: &[Arc<Decoration>], _open_start: usize) {
        if self.line_start > -1 {
            let end = (to as i64).min(self.line_end) as usize;
            if let Some(Some(
                last @ HeightMap {
                    node: Node::Text(_),
                    ..
                },
            )) = self.nodes.last_mut()
            {
                last.length += end - self.pos;
            } else if end > self.pos || !self.is_covered() {
                self.nodes
                    .push(Some(HeightMap::text(end - self.pos, -1.0, 0.0)));
            }
            self.written_to = end;
            if to > end {
                self.nodes.push(None);
                self.written_to += 1;
                self.line_start = -1;
            }
        }
        self.pos = to;
    }

    fn point(
        &mut self,
        from: usize,
        to: usize,
        deco: &Arc<Decoration>,
        _active: &[Arc<Decoration>],
        _open_start: usize,
        _index: usize,
    ) {
        if from < to || deco.height_relevant() {
            let widget = deco.widget_type();
            let mut height = widget.map_or(0.0, Widget::estimated_height);
            let breaks = widget.map_or(0, Widget::line_breaks);
            if height < 0.0 {
                height = self.oracle.line_height;
            }
            let len = to - from;
            if deco.is_block() {
                self.add_block(HeightMap::block(len, height, Some(deco.clone())));
            } else if len > 0 || breaks > 0 || height >= RELEVANT_WIDGET_HEIGHT {
                self.add_line_deco(height, breaks, len);
            }
        } else if to > from {
            self.span(from, to, &[], 0);
        }
        if self.line_end > -1 && self.line_end < self.pos as i64 {
            self.line_end = self.oracle.doc.line_at(self.pos).to as i64;
        }
    }
}

/// The ranges, as a flat list of `from`, `to` pairs, where height-relevant
/// decorations differ between `a` and `b` (`heightRelevantDecoChanges`).
pub fn height_relevant_deco_changes(
    a: &[DecorationSet],
    b: &[DecorationSet],
    diff: &ChangeDesc,
) -> Vec<usize> {
    let mut comparator = DecorationComparator {
        changes: Vec::new(),
    };
    RangeSet::compare(a, b, diff, &mut comparator, 0);
    comparator.changes
}

struct DecorationComparator {
    changes: Vec<usize>,
}

impl RangeComparator<Decoration> for DecorationComparator {
    fn compare_range(
        &mut self,
        _from: usize,
        _to: usize,
        _active_a: &[Arc<Decoration>],
        _active_b: &[Arc<Decoration>],
    ) {
    }

    fn compare_point(
        &mut self,
        from: usize,
        to: usize,
        a: Option<&Arc<Decoration>>,
        b: Option<&Arc<Decoration>>,
    ) {
        if from < to
            || a.is_some_and(|a| a.height_relevant())
            || b.is_some_and(|b| b.height_relevant())
        {
            add_range(from, to, &mut self.changes, 5);
        }
    }
}
