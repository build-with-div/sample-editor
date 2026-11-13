//! Port of `selection.ts`: selection ranges and multi-range selections.

use crate::change::ChangeDesc;
use crate::RangeError;

// A range's flags:
// - 3 bits for the bidi level (7 means unset), meaningful only for cursors;
// - 2 bits for the side the cursor is associated with;
// - 1 bit for whether the range is inverted (head before anchor),
//   meaningful only for non-empty ranges;
// - 1 bit for the "undirectional" flag, used for macOS-style selections
//   that pick a head and anchor side from the next shift-motion.
const BIDI_LEVEL_MASK: u8 = 7;
const ASSOC_BEFORE: u8 = 8;
const ASSOC_AFTER: u8 = 16;
const INVERTED: u8 = 32;
const UNDIRECTIONAL: u8 = 64;

/// A single selection range. An [`EditorSelection`] holds one or more.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectionRange {
    from: usize,
    to: usize,
    flags: u8,
    goal_column: Option<f64>,
}

impl SelectionRange {
    fn create(from: usize, to: usize, flags: u8, goal_column: Option<f64>) -> SelectionRange {
        SelectionRange {
            from,
            to,
            flags,
            goal_column,
        }
    }

    /// The lower boundary of the range.
    pub fn from(&self) -> usize {
        self.from
    }

    /// The upper boundary of the range.
    pub fn to(&self) -> usize {
        self.to
    }

    /// The anchor: the side that doesn't move when the range is extended.
    pub fn anchor(&self) -> usize {
        if self.flags & INVERTED != 0 {
            self.to
        } else {
            self.from
        }
    }

    /// The head: the side that moves when the range is extended.
    pub fn head(&self) -> usize {
        if self.flags & INVERTED != 0 {
            self.from
        } else {
            self.to
        }
    }

    /// True when anchor and head are at the same position.
    pub fn is_empty(&self) -> bool {
        self.from == self.to
    }

    /// The goal column (stored horizontal offset, in pixels) of a cursor,
    /// which keeps the horizontal position when moving across lines of
    /// different length.
    pub fn goal_column(&self) -> Option<f64> {
        self.goal_column
    }

    /// For a cursor explicitly associated with the character on one of its
    /// sides, that side: -1 before, 1 after, 0 for no association.
    pub fn assoc(&self) -> i8 {
        if self.flags & ASSOC_BEFORE != 0 {
            -1
        } else if self.flags & ASSOC_AFTER != 0 {
            1
        } else {
            0
        }
    }

    /// Whether selection-extending commands may swap head and anchor, as in
    /// macOS-style selections.
    pub fn undirectional(&self) -> bool {
        self.flags & UNDIRECTIONAL != 0
    }

    /// The bidirectional text level of this cursor, if any.
    pub fn bidi_level(&self) -> Option<u8> {
        let level = self.flags & BIDI_LEVEL_MASK;
        (level != 7).then_some(level)
    }

    /// Map this range through a change into the updated document.
    pub fn map(&self, change: &ChangeDesc, assoc: i32) -> SelectionRange {
        let (from, to) = if self.is_empty() {
            let pos = change.map_pos(self.from, assoc);
            (pos, pos)
        } else {
            (change.map_pos(self.from, 1), change.map_pos(self.to, -1))
        };
        if from == self.from && to == self.to {
            *self
        } else {
            SelectionRange::create(from, to, self.flags, self.goal_column)
        }
    }

    /// Extend this range to cover at least `from`..`to`.
    pub fn extend(&self, from: usize, to: usize, assoc: i8) -> SelectionRange {
        let anchor = self.anchor();
        if from <= anchor && to >= anchor {
            return EditorSelection::range_with(from, to, None, None, assoc);
        }
        let head = if from.abs_diff(anchor) > to.abs_diff(anchor) {
            from
        } else {
            to
        };
        EditorSelection::range_with(anchor, head, None, None, assoc)
    }

    /// Compare by anchor, head and goal column, and with `include_assoc`
    /// also by the [`assoc`](Self::assoc) of cursors.
    pub fn eq_with(&self, other: &SelectionRange, include_assoc: bool) -> bool {
        self.anchor() == other.anchor()
            && self.head() == other.head()
            && self.goal_column == other.goal_column
            && (!include_assoc || !self.is_empty() || self.assoc() == other.assoc())
    }

    /// The JSON representation: `(anchor, head)`.
    pub fn to_json(&self) -> (usize, usize) {
        (self.anchor(), self.head())
    }

    /// Read a range from its JSON representation.
    pub fn from_json((anchor, head): (usize, usize)) -> SelectionRange {
        EditorSelection::range(anchor, head)
    }
}

/// A selection: one or more ranges, sorted and not overlapping, with one
/// main range.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorSelection {
    ranges: Vec<SelectionRange>,
    main_index: usize,
}

impl EditorSelection {
    /// The ranges, sorted by position. They do not overlap, but may touch
    /// if they are not empty.
    pub fn ranges(&self) -> &[SelectionRange] {
        &self.ranges
    }

    /// The index of the main range, usually the one added last.
    pub fn main_index(&self) -> usize {
        self.main_index
    }

    /// Map the selection through a change.
    pub fn map(&self, change: &ChangeDesc, assoc: i32) -> EditorSelection {
        if change.is_empty() {
            return self.clone();
        }
        EditorSelection::create(
            self.ranges.iter().map(|r| r.map(change, assoc)).collect(),
            self.main_index,
        )
        .expect("mapping keeps at least one range")
    }

    /// Compare by range positions, and with `include_assoc` also by the
    /// `assoc` of cursors.
    pub fn eq_with(&self, other: &EditorSelection, include_assoc: bool) -> bool {
        self.ranges.len() == other.ranges.len()
            && self.main_index == other.main_index
            && self
                .ranges
                .iter()
                .zip(&other.ranges)
                .all(|(a, b)| a.eq_with(b, include_assoc))
    }

    /// The main range.
    pub fn main(&self) -> SelectionRange {
        self.ranges[self.main_index]
    }

    /// A selection holding only the main range.
    pub fn as_single(&self) -> EditorSelection {
        if self.ranges.len() == 1 {
            self.clone()
        } else {
            EditorSelection {
                ranges: vec![self.main()],
                main_index: 0,
            }
        }
    }

    /// Add a range, making it the main range when `main` is true.
    pub fn add_range(&self, range: SelectionRange, main: bool) -> EditorSelection {
        let mut ranges = Vec::with_capacity(self.ranges.len() + 1);
        ranges.push(range);
        ranges.extend_from_slice(&self.ranges);
        EditorSelection::create(ranges, if main { 0 } else { self.main_index + 1 })
            .expect("at least one range")
    }

    /// Replace the range at `which` (by default the main one), then merge
    /// and sort.
    pub fn replace_range(&self, range: SelectionRange, which: Option<usize>) -> EditorSelection {
        let mut ranges = self.ranges.clone();
        ranges[which.unwrap_or(self.main_index)] = range;
        EditorSelection::create(ranges, self.main_index).expect("at least one range")
    }

    /// The JSON representation: the ranges and the main index.
    pub fn to_json(&self) -> (Vec<(usize, usize)>, usize) {
        (
            self.ranges.iter().map(SelectionRange::to_json).collect(),
            self.main_index,
        )
    }

    /// Read a selection from its JSON representation.
    pub fn from_json(
        ranges: &[(usize, usize)],
        main: usize,
    ) -> Result<EditorSelection, RangeError> {
        if main >= ranges.len() {
            return Err(RangeError(
                "Invalid JSON representation for EditorSelection".to_string(),
            ));
        }
        Ok(EditorSelection {
            ranges: ranges
                .iter()
                .copied()
                .map(SelectionRange::from_json)
                .collect(),
            main_index: main,
        })
    }

    /// A selection holding the single range `anchor`..`head`.
    pub fn single(anchor: usize, head: usize) -> EditorSelection {
        EditorSelection {
            ranges: vec![EditorSelection::range(anchor, head)],
            main_index: 0,
        }
    }

    /// Sort and merge `ranges` into a valid selection.
    pub fn create(
        ranges: Vec<SelectionRange>,
        main_index: usize,
    ) -> Result<EditorSelection, RangeError> {
        if ranges.is_empty() {
            return Err(RangeError(
                "A selection needs at least one range".to_string(),
            ));
        }
        let mut pos = 0;
        for range in &ranges {
            if if range.is_empty() {
                range.from <= pos
            } else {
                range.from < pos
            } {
                return Ok(EditorSelection::normalized(ranges, main_index));
            }
            pos = range.to;
        }
        Ok(EditorSelection { ranges, main_index })
    }

    /// A cursor range at `pos`.
    pub fn cursor(pos: usize) -> SelectionRange {
        EditorSelection::cursor_with(pos, 0, None, None)
    }

    /// A cursor range with an association side, bidi level and goal
    /// column.
    pub fn cursor_with(
        pos: usize,
        assoc: i8,
        bidi_level: Option<u8>,
        goal_column: Option<f64>,
    ) -> SelectionRange {
        let side = match assoc {
            0 => 0,
            a if a < 0 => ASSOC_BEFORE,
            _ => ASSOC_AFTER,
        };
        SelectionRange::create(
            pos,
            pos,
            side | bidi_level.map_or(7, |l| l.min(6)),
            goal_column,
        )
    }

    /// A selection range from `anchor` to `head`.
    pub fn range(anchor: usize, head: usize) -> SelectionRange {
        EditorSelection::range_with(anchor, head, None, None, 0)
    }

    /// A selection range with a goal column, bidi level and association
    /// side. With `assoc` 0, a non-empty range associates away from its
    /// head.
    pub fn range_with(
        anchor: usize,
        head: usize,
        goal_column: Option<f64>,
        bidi_level: Option<u8>,
        assoc: i8,
    ) -> SelectionRange {
        let mut flags = bidi_level.map_or(7, |l| l.min(6));
        let mut assoc = assoc;
        if assoc == 0 && anchor != head {
            assoc = if head < anchor { 1 } else { -1 };
        }
        if assoc != 0 {
            flags |= if assoc < 0 { ASSOC_BEFORE } else { ASSOC_AFTER };
        }
        if head < anchor {
            SelectionRange::create(head, anchor, flags | INVERTED, goal_column)
        } else {
            SelectionRange::create(anchor, head, flags, goal_column)
        }
    }

    /// An [undirectional](SelectionRange::undirectional) range.
    pub fn undirectional_range(from: usize, to: usize) -> SelectionRange {
        SelectionRange::create(from, to, UNDIRECTIONAL, None)
    }

    fn normalized(ranges: Vec<SelectionRange>, main_index: usize) -> EditorSelection {
        // A stable sort, as JavaScript's `Array.prototype.sort` is, that
        // keeps track of the main range by identity, as `indexOf` does.
        let mut indexed: Vec<(usize, SelectionRange)> = ranges.into_iter().enumerate().collect();
        indexed.sort_by_key(|(_, r)| r.from);
        let mut main_index = indexed
            .iter()
            .position(|(i, _)| *i == main_index)
            .expect("the main range is present");
        let mut ranges: Vec<SelectionRange> = indexed.into_iter().map(|(_, r)| r).collect();
        let mut i = 1;
        while i < ranges.len() {
            let (range, prev) = (ranges[i], ranges[i - 1]);
            if if range.is_empty() {
                range.from <= prev.to
            } else {
                range.from < prev.to
            } {
                let (from, to) = (prev.from, range.to.max(prev.to));
                if i <= main_index {
                    main_index -= 1;
                }
                i -= 1;
                let merged = if range.anchor() > range.head() {
                    EditorSelection::range(to, from)
                } else {
                    EditorSelection::range(from, to)
                };
                ranges.splice(i..i + 2, [merged]);
            }
            i += 1;
        }
        EditorSelection { ranges, main_index }
    }
}
