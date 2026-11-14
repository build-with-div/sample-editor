//! Port of `rangeset.ts`: range values, range sets, their cursors and
//! builder, and the span iteration and comparison CodeMirror's view uses
//! for decorations, gutter markers and folds.
//!
//! Positions are `usize` in the public API. Inside, positions are `i64`
//! and sides `i32`, so that upstream's sentinels (`-1e9`, `-1` for "no
//! position yet") keep their meaning; upstream's `a - b || c - d`
//! comparisons are written as [`Ordering`] chains.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use crate::change::{ChangeDesc, MapMode, Touch};

// The maximum amount of ranges to store in a single chunk.
const CHUNK_SIZE: usize = 250;
// A large value used for maximum and minimum positions and sides.
const FAR: i64 = 1_000_000_000;
const FAR_SIDE: i32 = 1_000_000_000;

/// A value associated with a [`Range`] (`RangeValue`).
pub trait RangeValue: Send + Sync + 'static {
    /// Compare with another value, when comparing range sets (`eq`). The
    /// default compares by identity. Unless only a fixed number of
    /// distinct values exist, implement this properly.
    fn range_eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }

    /// The bias at the start of the range, which orders it among ranges
    /// starting at the same position (`startSide`). Defaults to 0.
    fn start_side(&self) -> i32 {
        0
    }

    /// The bias at the end of the range (`endSide`). Defaults to 0.
    fn end_side(&self) -> i32 {
        0
    }

    /// How an empty range is mapped, which decides whether a change
    /// deletes it (`mapMode`). Defaults to [`MapMode::TrackDel`].
    fn map_mode(&self) -> MapMode {
        MapMode::TrackDel
    }

    /// Whether this value marks a point range (`point`). Regular ranges
    /// affect the content they cover and mean nothing when empty. Point
    /// ranges mean something on their own; a non-empty one is atomic and
    /// shadows the ranges inside it.
    fn point(&self) -> bool {
        false
    }

    /// A [`Range`] with this value (`range`).
    fn range(self: &Arc<Self>, from: usize, to: usize) -> Range<Self>
    where
        Self: Sized,
    {
        Range::new(from, to, self.clone())
    }
}

fn cmp_val<T: RangeValue>(a: &Arc<T>, b: &Arc<T>) -> bool {
    Arc::ptr_eq(a, b) || a.range_eq(b)
}

fn cmp2(a: i64, b: i64, side_a: i32, side_b: i32) -> Ordering {
    a.cmp(&b).then(side_a.cmp(&side_b))
}

/// A value associated with a range of positions (`Range`).
pub struct Range<T> {
    /// The start position.
    pub from: usize,
    /// The end position.
    pub to: usize,
    /// The value.
    pub value: Arc<T>,
}

impl<T> Range<T> {
    /// A range with `value` from `from` to `to`.
    pub fn new(from: usize, to: usize, value: Arc<T>) -> Range<T> {
        Range { from, to, value }
    }
}

impl<T> Clone for Range<T> {
    fn clone(&self) -> Self {
        Range {
            from: self.from,
            to: self.to,
            value: self.value.clone(),
        }
    }
}

impl<T> fmt::Debug for Range<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Range({}..{})", self.from, self.to)
    }
}

fn cmp_range<T: RangeValue>(a: &Range<T>, b: &Range<T>) -> Ordering {
    a.from
        .cmp(&b.from)
        .then(a.value.start_side().cmp(&b.value.start_side()))
}

/// Receives the differences [`RangeSet::compare`] finds
/// (`RangeComparator`).
pub trait RangeComparator<T> {
    /// A range, in positions of the new document, whose sets of values
    /// differ between the old (`active_a`) and new (`active_b`) sets.
    fn compare_range(&mut self, from: usize, to: usize, active_a: &[Arc<T>], active_b: &[Arc<T>]);

    /// A changed, inserted or deleted point range.
    fn compare_point(
        &mut self,
        from: usize,
        to: usize,
        point_a: Option<&Arc<T>>,
        point_b: Option<&Arc<T>>,
    );

    /// Whether [`bound_change`](Self::bound_change) should be called
    /// (whether upstream's optional `boundChange` method exists).
    fn wants_bound_changes(&self) -> bool {
        false
    }

    /// A changed boundary between ranges: for example, where two partial
    /// ranges used to meet before one bigger range covered the span.
    fn bound_change(&mut self, _pos: usize) {}
}

/// Receives the spans [`RangeSet::spans`] produces (`SpanIterator`).
/// The whole iterated range is covered by `span` and `point` calls.
pub trait SpanIterator<T> {
    /// A stretch not covered by a point, with the values of the ranges
    /// covering it, of which the first `open_start` are continued from
    /// before the span.
    fn span(&mut self, from: usize, to: usize, active: &[Arc<T>], open_start: usize);

    /// A point, with the active ranges of higher precedence covering it.
    /// `open_start` counts those that started before the point, or is
    /// `active.len() + 1` when the point starts before the iterated
    /// range. `index` is the position of the point's set among the
    /// iterated sets.
    fn point(
        &mut self,
        from: usize,
        to: usize,
        value: &Arc<T>,
        active: &[Arc<T>],
        open_start: usize,
        index: usize,
    );
}

struct Chunk<T> {
    from: Vec<i64>,
    to: Vec<i64>,
    value: Vec<Arc<T>>,
    // The largest point in the chunk, or -1 for no points, so that scans
    // interested only in points can skip range-only chunks.
    max_point: i64,
}

impl<T: RangeValue> Chunk<T> {
    fn length(&self) -> i64 {
        *self.to.last().expect("a chunk is not empty")
    }

    // The index of the position and side, comparing with the ranges'
    // `from` (or with `to` when `end`).
    fn find_index(&self, pos: i64, side: i32, end: bool, start_at: usize) -> usize {
        let arr = if end { &self.to } else { &self.from };
        let (mut lo, mut hi) = (start_at, arr.len());
        loop {
            if lo == hi {
                return lo;
            }
            let mid = (lo + hi) >> 1;
            let value_side = if end {
                self.value[mid].end_side()
            } else {
                self.value[mid].start_side()
            };
            let diff = cmp2(arr[mid], pos, value_side, side);
            if mid == lo {
                return if diff != Ordering::Less { lo } else { hi };
            }
            if diff != Ordering::Less {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
    }

    // Returns false when `f` stopped the iteration.
    fn between(
        &self,
        offset: i64,
        from: i64,
        to: i64,
        f: &mut dyn FnMut(usize, usize, &Arc<T>) -> bool,
    ) -> bool {
        let mut i = self.find_index(from, -FAR_SIDE, true, 0);
        let e = self.find_index(to, FAR_SIDE, false, i);
        while i < e {
            if !f(
                (self.from[i] + offset) as usize,
                (self.to[i] + offset) as usize,
                &self.value[i],
            ) {
                return false;
            }
            i += 1;
        }
        true
    }

    fn map(
        &self,
        offset: i64,
        changes: &ChangeDesc,
        mut base_pos: i64,
        mut base_side: i32,
        spill: &mut dyn FnMut(i64, i64, Arc<T>),
    ) -> (Option<Chunk<T>>, i64) {
        let mut value: Vec<Arc<T>> = Vec::new();
        let mut from: Vec<i64> = Vec::new();
        let mut to: Vec<i64> = Vec::new();
        let mut new_pos = -1;
        let mut max_point = -1;
        'iter: for i in 0..self.value.len() {
            let val = &self.value[i];
            let (start_side, end_side) = (val.start_side(), val.end_side());
            let cur_from = self.from[i] + offset;
            let cur_to = self.to[i] + offset;
            let (new_from, mut new_to);
            if cur_from == cur_to {
                let Some(mapped) =
                    changes.map_pos_mode(cur_from as usize, start_side, val.map_mode())
                else {
                    continue;
                };
                new_from = mapped as i64;
                new_to = new_from;
                if start_side != end_side {
                    new_to = changes.map_pos(cur_from as usize, end_side) as i64;
                    if new_to < new_from {
                        continue;
                    }
                }
            } else {
                new_from = changes.map_pos(cur_from as usize, start_side) as i64;
                new_to = changes.map_pos(cur_to as usize, end_side) as i64;
                if new_from > new_to || new_from == new_to && start_side > 0 && end_side <= 0 {
                    continue;
                }
            }
            if (new_to - new_from).cmp(&0).then(end_side.cmp(&start_side)) == Ordering::Less {
                continue;
            }
            if new_pos < 0 {
                new_pos = new_from;
            }
            if val.point() {
                max_point = max_point.max(new_to - new_from);
            }

            if cmp2(new_from, base_pos, start_side, base_side) != Ordering::Less {
                value.push(val.clone());
                from.push(new_from - new_pos);
                to.push(new_to - new_pos);
                base_pos = new_to;
                base_side = end_side;
            } else {
                if new_from == new_to {
                    // Try to reorder points to fit in here.
                    for i in (1..=value.len()).rev() {
                        if cmp2(
                            new_from,
                            to[i - 1] + new_pos,
                            start_side,
                            value[i - 1].end_side(),
                        ) != Ordering::Less
                        {
                            value.insert(i, val.clone());
                            from.insert(i, new_from - new_pos);
                            to.insert(i, new_to - new_pos);
                            continue 'iter;
                        }
                        if cmp2(
                            new_from,
                            from[i - 1] + new_pos,
                            end_side,
                            value[i - 1].start_side(),
                        ) == Ordering::Greater
                        {
                            break;
                        }
                    }
                }
                // Otherwise, spill into a new layer.
                spill(new_from, new_to, val.clone());
            }
        }
        let mapped = (!value.is_empty()).then_some(Chunk {
            from,
            to,
            value,
            max_point,
        });
        (mapped, new_pos)
    }
}

fn chunk_key<T>(chunk: &Arc<Chunk<T>>) -> usize {
    Arc::as_ptr(chunk) as usize
}

struct SetInner<T> {
    chunk_pos: Vec<i64>,
    chunks: Vec<Arc<Chunk<T>>>,
    // `None` only for the empty set, whose next layer upstream is itself.
    next_layer: Option<RangeSet<T>>,
    max_point: i64,
}

/// An immutable collection of [ranges](Range), efficient to map and
/// update (`RangeSet`). Ranges are stored in chunks, and ranges that
/// overlap ranges before them go to further layers. Cloning is cheap.
pub struct RangeSet<T>(Arc<SetInner<T>>);

impl<T> Clone for RangeSet<T> {
    fn clone(&self) -> Self {
        RangeSet(self.0.clone())
    }
}

impl<T: RangeValue> fmt::Debug for RangeSet<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut list = f.debug_list();
        let mut cursor = self.iter(0);
        while cursor.value().is_some() {
            list.entry(&(cursor.from()..cursor.to()));
            cursor.next();
        }
        list.finish()
    }
}

/// How [`RangeSet::update`] changes a set (`RangeSetUpdate`).
pub struct RangeSetUpdate<'a, T> {
    /// Ranges to add, sorted by `from` and `start_side` unless `sort` is
    /// set.
    pub add: Vec<Range<T>>,
    /// Sort `add` first.
    pub sort: bool,
    /// Keep only the existing ranges for which this returns true.
    pub filter: Filter<'a, T>,
    /// Apply `filter` only from this position (0 by default).
    pub filter_from: Option<usize>,
    /// Apply `filter` only up to this position (the set's end by
    /// default).
    pub filter_to: Option<usize>,
}

impl<T> Default for RangeSetUpdate<'_, T> {
    fn default() -> Self {
        RangeSetUpdate {
            add: Vec::new(),
            sort: false,
            filter: None,
            filter_from: None,
            filter_to: None,
        }
    }
}

type Filter<'a, T> = Option<&'a mut dyn FnMut(usize, usize, &Arc<T>) -> bool>;

impl<T: RangeValue> RangeSet<T> {
    fn create(
        chunk_pos: Vec<i64>,
        chunks: Vec<Arc<Chunk<T>>>,
        next_layer: RangeSet<T>,
        max_point: i64,
    ) -> Self {
        RangeSet(Arc::new(SetInner {
            chunk_pos,
            chunks,
            next_layer: Some(next_layer),
            max_point,
        }))
    }

    /// The empty set (`RangeSet.empty`).
    pub fn empty() -> Self {
        RangeSet(Arc::new(SetInner {
            chunk_pos: Vec::new(),
            chunks: Vec::new(),
            next_layer: None,
            max_point: -1,
        }))
    }

    /// Whether the set has no ranges (`isEmpty`, internal upstream).
    pub fn is_empty(&self) -> bool {
        self.0.next_layer.is_none()
    }

    /// Whether `self` and `other` are the same set object.
    pub fn ptr_eq(&self, other: &RangeSet<T>) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    fn next_layer(&self) -> &RangeSet<T> {
        self.0.next_layer.as_ref().unwrap_or(self)
    }

    fn length_inner(&self) -> i64 {
        match self.0.chunks.len() {
            0 => 0,
            n => self.chunk_end(n - 1).max(self.next_layer().length_inner()),
        }
    }

    /// The end of the last range (`length`, internal upstream).
    pub fn len(&self) -> usize {
        self.length_inner() as usize
    }

    /// The number of ranges in the set (`size`).
    pub fn size(&self) -> usize {
        if self.is_empty() {
            return 0;
        }
        self.next_layer().size() + self.0.chunks.iter().map(|c| c.value.len()).sum::<usize>()
    }

    /// The largest point range in the set, or -1 for none (`maxPoint`,
    /// internal upstream).
    pub fn max_point(&self) -> i64 {
        self.0.max_point
    }

    fn chunk_end(&self, index: usize) -> i64 {
        self.0.chunk_pos[index] + self.0.chunks[index].length()
    }

    /// The number of chunks in the set's first layer. Internal upstream;
    /// for tests that check the set's structure.
    #[doc(hidden)]
    pub fn chunk_count(&self) -> usize {
        self.0.chunks.len()
    }

    /// Whether chunk `index` of this set's first layer is the same object
    /// as that of `other`. For tests that check structural sharing.
    #[doc(hidden)]
    pub fn shares_chunk(&self, other: &RangeSet<T>, index: usize) -> bool {
        Arc::ptr_eq(&self.0.chunks[index], &other.0.chunks[index])
    }

    /// The set's second layer, or `None` when it has one layer. For tests
    /// that check the set's structure.
    #[doc(hidden)]
    pub fn layer_below(&self) -> Option<&RangeSet<T>> {
        self.0.next_layer.as_ref().filter(|next| !next.is_empty())
    }

    /// Add ranges or filter out existing ones (`update`).
    pub fn update(&self, spec: RangeSetUpdate<'_, T>) -> RangeSet<T> {
        let filter_to = spec
            .filter_to
            .map_or_else(|| self.length_inner(), |to| to as i64);
        self.update_inner(
            spec.add,
            spec.sort,
            spec.filter,
            spec.filter_from.unwrap_or(0) as i64,
            filter_to,
        )
    }

    fn update_inner(
        &self,
        mut add: Vec<Range<T>>,
        sort: bool,
        mut filter: Filter<'_, T>,
        filter_from: i64,
        filter_to: i64,
    ) -> RangeSet<T> {
        if add.is_empty() && filter.is_none() {
            return self.clone();
        }
        if sort {
            add.sort_by(cmp_range);
        }
        if self.is_empty() {
            return if add.is_empty() {
                self.clone()
            } else {
                RangeSet::of(add, false)
            };
        }

        let inner = &self.0;
        let mut cur = LayerCursor::new(self.clone(), None, -1, 0);
        cur.goto(0, -FAR_SIDE);
        let mut i = 0;
        let mut spill: Vec<Range<T>> = Vec::new();
        let mut builder = RangeSetBuilder::new();
        while cur.value.is_some() || i < add.len() {
            if i < add.len()
                && cmp2(
                    cur.from,
                    add[i].from as i64,
                    cur.start_side(),
                    add[i].value.start_side(),
                ) != Ordering::Less
            {
                let range = add[i].clone();
                i += 1;
                if !builder.add_inner(
                    range.from as i64,
                    range.to as i64,
                    range.value.clone(),
                    false,
                ) {
                    spill.push(range);
                }
            } else if cur.range_index == 1
                && cur.chunk_index < inner.chunks.len()
                && (i == add.len() || self.chunk_end(cur.chunk_index) < add[i].from as i64)
                && (filter.is_none()
                    || filter_from > self.chunk_end(cur.chunk_index)
                    || filter_to < inner.chunk_pos[cur.chunk_index])
                && builder.add_chunk(
                    inner.chunk_pos[cur.chunk_index],
                    inner.chunks[cur.chunk_index].clone(),
                )
            {
                cur.next_chunk();
            } else {
                let value = cur.value.clone().expect("a range");
                let keep = match filter.as_mut() {
                    None => true,
                    Some(f) => {
                        filter_from > cur.to
                            || filter_to < cur.from
                            || f(cur.from as usize, cur.to as usize, &value)
                    }
                };
                if keep && !builder.add_inner(cur.from, cur.to, value.clone(), false) {
                    spill.push(Range::new(cur.from as usize, cur.to as usize, value));
                }
                cur.next();
            }
        }

        let next = if self.next_layer().is_empty() && spill.is_empty() {
            RangeSet::empty()
        } else {
            self.next_layer()
                .update_inner(spill, false, filter, filter_from, filter_to)
        };
        builder.finish_inner(next)
    }

    /// Map the set through `changes` (`map`).
    pub fn map(&self, changes: &ChangeDesc) -> RangeSet<T> {
        if changes.is_empty() || self.is_empty() {
            return self.clone();
        }
        let inner = &self.0;
        let mut chunks: Vec<Arc<Chunk<T>>> = Vec::new();
        let mut chunk_pos: Vec<i64> = Vec::new();
        let mut max_point = -1;
        let mut spilled: Option<RangeSetBuilder<T>> = None;
        for (i, chunk) in inner.chunks.iter().enumerate() {
            let start = inner.chunk_pos[i];
            match changes.touches_range(start as usize, (start + chunk.length()) as usize) {
                Touch::No => {
                    max_point = max_point.max(chunk.max_point);
                    chunks.push(chunk.clone());
                    chunk_pos.push(changes.map_pos(start as usize, -1) as i64);
                }
                Touch::Yes => {
                    let (prev_pos, prev_side) = match (chunks.last(), chunk_pos.last()) {
                        (Some(last), Some(&pos)) => (
                            pos + last.length(),
                            last.value.last().expect("a range").end_side(),
                        ),
                        _ => (-1, -1),
                    };
                    let (mapped, pos) = chunk.map(
                        start,
                        changes,
                        prev_pos,
                        prev_side,
                        &mut |from, to, value| {
                            spilled
                                .get_or_insert_with(RangeSetBuilder::new)
                                .add_range(from, to, value, false)
                        },
                    );
                    if let Some(mapped) = mapped {
                        max_point = max_point.max(mapped.max_point);
                        chunks.push(Arc::new(mapped));
                        chunk_pos.push(pos);
                    }
                }
                Touch::Cover => {}
            }
        }
        let mut next = self.next_layer().map(changes);
        if let Some(spilled) = spilled {
            next = spilled.finish_inner(next);
        }
        if chunks.is_empty() {
            next
        } else {
            RangeSet::create(chunk_pos, chunks, next, max_point)
        }
    }

    /// Call `f` for the ranges that touch `from`..`to`, in no particular
    /// order, until it returns false (`between`).
    pub fn between(
        &self,
        from: usize,
        to: usize,
        mut f: impl FnMut(usize, usize, &Arc<T>) -> bool,
    ) {
        self.between_inner(from as i64, to as i64, &mut f);
    }

    fn between_inner(&self, from: i64, to: i64, f: &mut dyn FnMut(usize, usize, &Arc<T>) -> bool) {
        if self.is_empty() {
            return;
        }
        for (i, chunk) in self.0.chunks.iter().enumerate() {
            let start = self.0.chunk_pos[i];
            if to >= start
                && from <= start + chunk.length()
                && !chunk.between(start, from - start, to - start, f)
            {
                return;
            }
        }
        self.next_layer().between_inner(from, to, f)
    }

    /// A cursor over the ranges in order, starting with those that end at
    /// or after `from` (`iter`).
    pub fn iter(&self, from: usize) -> RangeCursor<T> {
        RangeSet::iter_sets(std::slice::from_ref(self), from)
    }

    /// A cursor over the ranges of several sets in order, starting at
    /// `from` (`RangeSet.iter`).
    pub fn iter_sets(sets: &[RangeSet<T>], from: usize) -> RangeCursor<T> {
        let mut cursor = Cursor::from_sets(sets, None, -1);
        cursor.goto(from as i64, -FAR_SIDE);
        RangeCursor(cursor)
    }

    /// Compare two groups of sets, telling `comparator` where they
    /// differ (`RangeSet.compare`). `text_diff` describes how the document
    /// changed between them. With `min_point_size` above -1, ranges that
    /// are not points and points smaller than it are ignored.
    // `ChangeDesc::is_empty` means "no changes" and `len` is the document
    // length, as upstream's `empty` and `length`; both are checked.
    #[allow(clippy::len_zero)]
    pub fn compare(
        old_sets: &[RangeSet<T>],
        new_sets: &[RangeSet<T>],
        text_diff: &ChangeDesc,
        comparator: &mut dyn RangeComparator<T>,
        min_point_size: i64,
    ) {
        let keep = |set: &&RangeSet<T>| {
            set.0.max_point > 0 || !set.is_empty() && set.0.max_point >= min_point_size
        };
        let a: Vec<RangeSet<T>> = old_sets.iter().filter(keep).cloned().collect();
        let b: Vec<RangeSet<T>> = new_sets.iter().filter(keep).cloned().collect();
        let shared = Arc::new(find_shared_chunks(&a, &b, Some(text_diff)));

        let mut side_a = SpanCursor::new(&a, Some(shared.clone()), min_point_size);
        let mut side_b = SpanCursor::new(&b, Some(shared), min_point_size);

        text_diff.iter_gaps(|from_a, from_b, length| {
            compare(
                &mut side_a,
                from_a as i64,
                &mut side_b,
                from_b as i64,
                length as i64,
                comparator,
            )
        });
        if text_diff.is_empty() && text_diff.len() == 0 {
            compare(&mut side_a, 0, &mut side_b, 0, 0, comparator);
        }
    }

    /// Whether two groups of sets hold equivalent ranges from `from` to
    /// `to` (`RangeSet.eq`).
    pub fn eq(
        old_sets: &[RangeSet<T>],
        new_sets: &[RangeSet<T>],
        from: usize,
        to: Option<usize>,
    ) -> bool {
        let to = to.map_or(FAR - 1, |to| to as i64);
        let contains = |sets: &[RangeSet<T>], set: &RangeSet<T>| sets.iter().any(|s| s.ptr_eq(set));
        let a: Vec<RangeSet<T>> = old_sets
            .iter()
            .filter(|set| !set.is_empty() && !contains(new_sets, set))
            .cloned()
            .collect();
        let b: Vec<RangeSet<T>> = new_sets
            .iter()
            .filter(|set| !set.is_empty() && !contains(old_sets, set))
            .cloned()
            .collect();
        if a.len() != b.len() {
            return false;
        }
        if a.is_empty() {
            return true;
        }
        let shared = Arc::new(find_shared_chunks(&a, &b, None));
        let mut side_a = SpanCursor::new(&a, Some(shared.clone()), 0);
        let mut side_b = SpanCursor::new(&b, Some(shared), 0);
        side_a.goto(from as i64, -FAR_SIDE);
        side_b.goto(from as i64, -FAR_SIDE);
        loop {
            let points_differ = match (&side_a.point, &side_b.point) {
                (Some(pa), Some(pb)) => !cmp_val(pa, pb),
                (Some(_), None) => true,
                _ => false,
            };
            if side_a.to != side_b.to
                || !same_values(&side_a.active, &side_b.active)
                || points_differ
            {
                return false;
            }
            if side_a.to > to {
                return true;
            }
            side_a.next();
            side_b.next();
        }
    }

    /// Iterate over several sets at once, telling `iterator` which ranges
    /// cover each piece of `from`..`to` (`RangeSet.spans`). With
    /// `min_point_size` above -1, only points at least that big count.
    /// Returns the open count at the end (see [`SpanIterator::span`]).
    pub fn spans(
        sets: &[RangeSet<T>],
        from: usize,
        to: usize,
        iterator: &mut dyn SpanIterator<T>,
        min_point_size: i64,
    ) -> usize {
        let (from, to) = (from as i64, to as i64);
        let mut cursor = SpanCursor::new(sets, None, min_point_size);
        cursor.goto(from, -FAR_SIDE);
        let mut pos = from;
        let mut open_ranges = cursor.open_start.max(0) as usize;
        loop {
            let cur_to = cursor.to.min(to);
            if let Some(point) = cursor.point.clone() {
                let active = cursor.active_for_point(cursor.to);
                let open_count = if cursor.point_from < from {
                    active.len() + 1
                } else if point.start_side() < 0 {
                    active.len()
                } else {
                    active.len().min(open_ranges)
                };
                iterator.point(
                    pos as usize,
                    cur_to as usize,
                    &point,
                    &active,
                    open_count,
                    cursor.point_rank as usize,
                );
                open_ranges = cursor.open_end(cur_to).min(active.len());
            } else if cur_to > pos {
                iterator.span(pos as usize, cur_to as usize, &cursor.active, open_ranges);
                open_ranges = cursor.open_end(cur_to);
            }
            if cursor.to > to {
                return open_ranges + usize::from(cursor.point.is_some() && cursor.to > to);
            }
            pos = cursor.to;
            cursor.next();
        }
    }

    /// A set of `ranges`, which must be sorted by `from` and `start_side`
    /// unless `sort` is true (`RangeSet.of`).
    ///
    /// # Panics
    ///
    /// When the ranges are not sorted and `sort` is false.
    pub fn of(ranges: Vec<Range<T>>, sort: bool) -> RangeSet<T> {
        let mut build = RangeSetBuilder::new();
        let ranges = if sort { lazy_sort(ranges) } else { ranges };
        for range in ranges {
            build.add(range.from, range.to, range.value);
        }
        build.finish()
    }

    /// Join several sets into one (`RangeSet.join`).
    pub fn join(sets: &[RangeSet<T>]) -> RangeSet<T> {
        let Some(last) = sets.last() else {
            return RangeSet::empty();
        };
        let mut result = last.clone();
        for set in sets[..sets.len() - 1].iter().rev() {
            let mut layer = set.clone();
            while !layer.is_empty() {
                let max_point = layer.0.max_point.max(result.0.max_point);
                result = RangeSet::create(
                    layer.0.chunk_pos.clone(),
                    layer.0.chunks.clone(),
                    result,
                    max_point,
                );
                layer = layer.next_layer().clone();
            }
        }
        result
    }
}

fn lazy_sort<T: RangeValue>(mut ranges: Vec<Range<T>>) -> Vec<Range<T>> {
    if ranges
        .windows(2)
        .any(|w| cmp_range(&w[0], &w[1]) == Ordering::Greater)
    {
        ranges.sort_by(cmp_range);
    }
    ranges
}

/// Builds a [`RangeSet`] directly from ranges added in order
/// (`RangeSetBuilder`).
pub struct RangeSetBuilder<T> {
    chunks: Vec<Arc<Chunk<T>>>,
    chunk_pos: Vec<i64>,
    chunk_start: i64,
    last: Option<Arc<T>>,
    last_from: i64,
    last_to: i64,
    from: Vec<i64>,
    to: Vec<i64>,
    value: Vec<Arc<T>>,
    max_point: i64,
    set_max_point: i64,
    next_layer: Option<Box<RangeSetBuilder<T>>>,
}

impl<T: RangeValue> Default for RangeSetBuilder<T> {
    fn default() -> Self {
        RangeSetBuilder::new()
    }
}

impl<T: RangeValue> RangeSetBuilder<T> {
    /// An empty builder.
    pub fn new() -> Self {
        RangeSetBuilder {
            chunks: Vec::new(),
            chunk_pos: Vec::new(),
            chunk_start: -1,
            last: None,
            last_from: -FAR,
            last_to: -FAR,
            from: Vec::new(),
            to: Vec::new(),
            value: Vec::new(),
            max_point: -1,
            set_max_point: -1,
            next_layer: None,
        }
    }

    fn finish_chunk(&mut self) {
        self.chunks.push(Arc::new(Chunk {
            from: std::mem::take(&mut self.from),
            to: std::mem::take(&mut self.to),
            value: std::mem::take(&mut self.value),
            max_point: self.max_point,
        }));
        self.chunk_pos.push(self.chunk_start);
        self.chunk_start = -1;
        self.set_max_point = self.set_max_point.max(self.max_point);
        self.max_point = -1;
    }

    // `from` and the side against the last range's end; `last` is only
    // read when the positions are equal, which needs a last range.
    fn cmp_last_end(&self, pos: i64, side: i32) -> Ordering {
        pos.cmp(&self.last_to)
            .then_with(|| side.cmp(&self.last.as_ref().map_or(-FAR_SIDE, |last| last.end_side())))
    }

    /// Add a range. Ranges must be added sorted by `from` and
    /// `start_side`.
    ///
    /// # Panics
    ///
    /// When a range is added out of order.
    pub fn add(&mut self, from: usize, to: usize, value: Arc<T>) {
        self.add_range(from as i64, to as i64, value, true);
    }

    fn add_range(&mut self, from: i64, to: i64, value: Arc<T>, strict: bool) {
        if !self.add_inner(from, to, value.clone(), strict) {
            self.next_layer
                .get_or_insert_with(|| Box::new(RangeSetBuilder::new()))
                .add_range(from, to, value, strict);
        }
    }

    fn add_inner(&mut self, from: i64, to: i64, value: Arc<T>, strict: bool) -> bool {
        let diff = self.cmp_last_end(from, value.start_side());
        if strict && diff != Ordering::Greater {
            let before_last = from.cmp(&self.last_from).then_with(|| {
                value.start_side().cmp(
                    &self
                        .last
                        .as_ref()
                        .map_or(-FAR_SIDE, |last| last.start_side()),
                )
            });
            if before_last == Ordering::Less {
                panic!("Ranges must be added sorted by `from` position and `startSide`");
            }
        }
        if diff == Ordering::Less {
            return false;
        }
        if self.from.len() == CHUNK_SIZE {
            self.finish_chunk();
        }
        if self.chunk_start < 0 {
            self.chunk_start = from;
        }
        self.from.push(from - self.chunk_start);
        self.to.push(to - self.chunk_start);
        self.last_from = from;
        self.last_to = to;
        if value.point() {
            self.max_point = self.max_point.max(to - from);
        }
        self.last = Some(value.clone());
        self.value.push(value);
        true
    }

    fn add_chunk(&mut self, from: i64, chunk: Arc<Chunk<T>>) -> bool {
        if self.cmp_last_end(from, chunk.value[0].start_side()) == Ordering::Less {
            return false;
        }
        if !self.from.is_empty() {
            self.finish_chunk();
        }
        self.set_max_point = self.set_max_point.max(chunk.max_point);
        let last = chunk.value.len() - 1;
        self.last = Some(chunk.value[last].clone());
        self.last_from = chunk.from[last] + from;
        self.last_to = chunk.to[last] + from;
        self.chunks.push(chunk);
        self.chunk_pos.push(from);
        true
    }

    /// Finish the set (`finish`).
    pub fn finish(self) -> RangeSet<T> {
        self.finish_inner(RangeSet::empty())
    }

    fn finish_inner(mut self, next: RangeSet<T>) -> RangeSet<T> {
        if !self.from.is_empty() {
            self.finish_chunk();
        }
        if self.chunks.is_empty() {
            return next;
        }
        let next = match self.next_layer {
            Some(layer) => layer.finish_inner(next),
            None => next,
        };
        RangeSet::create(self.chunk_pos, self.chunks, next, self.set_max_point)
    }
}

fn find_shared_chunks<T: RangeValue>(
    a: &[RangeSet<T>],
    b: &[RangeSet<T>],
    text_diff: Option<&ChangeDesc>,
) -> HashSet<usize> {
    let mut in_a: HashMap<usize, i64> = HashMap::new();
    for set in a {
        for (i, chunk) in set.0.chunks.iter().enumerate() {
            if chunk.max_point <= 0 {
                in_a.insert(chunk_key(chunk), set.0.chunk_pos[i]);
            }
        }
    }
    let mut shared = HashSet::new();
    for set in b {
        for (i, chunk) in set.0.chunks.iter().enumerate() {
            let Some(&known) = in_a.get(&chunk_key(chunk)) else {
                continue;
            };
            let mapped = text_diff.map_or(known, |diff| diff.map_pos(known as usize, -1) as i64);
            let untouched = text_diff.is_none_or(|diff| {
                diff.touches_range(known as usize, (known + chunk.length()) as usize) == Touch::No
            });
            if mapped == set.0.chunk_pos[i] && untouched {
                shared.insert(chunk_key(chunk));
            }
        }
    }
    shared
}

struct LayerCursor<T> {
    layer: RangeSet<T>,
    skip: Option<Arc<HashSet<usize>>>,
    min_point: i64,
    rank: i64,
    from: i64,
    to: i64,
    value: Option<Arc<T>>,
    chunk_index: usize,
    range_index: usize,
}

impl<T: RangeValue> LayerCursor<T> {
    fn new(
        layer: RangeSet<T>,
        skip: Option<Arc<HashSet<usize>>>,
        min_point: i64,
        rank: i64,
    ) -> Self {
        LayerCursor {
            layer,
            skip,
            min_point,
            rank,
            from: 0,
            to: 0,
            value: None,
            chunk_index: 0,
            range_index: 0,
        }
    }

    fn start_side(&self) -> i32 {
        self.value.as_ref().map_or(0, |v| v.start_side())
    }

    fn end_side(&self) -> i32 {
        self.value.as_ref().map_or(0, |v| v.end_side())
    }

    fn skipped(&self, index: usize) -> bool {
        self.skip
            .as_ref()
            .is_some_and(|skip| skip.contains(&chunk_key(&self.layer.0.chunks[index])))
    }

    fn goto(&mut self, pos: i64, side: i32) {
        self.chunk_index = 0;
        self.range_index = 0;
        self.goto_inner(pos, side, false);
    }

    fn goto_inner(&mut self, pos: i64, side: i32, mut forward: bool) {
        let layer = self.layer.clone();
        let chunks = &layer.0.chunks;
        while self.chunk_index < chunks.len() {
            let next = &chunks[self.chunk_index];
            if !(self.skipped(self.chunk_index)
                || layer.chunk_end(self.chunk_index) < pos
                || next.max_point < self.min_point)
            {
                break;
            }
            self.chunk_index += 1;
            forward = false;
        }
        if self.chunk_index < chunks.len() {
            let range_index = chunks[self.chunk_index].find_index(
                pos - layer.0.chunk_pos[self.chunk_index],
                side,
                true,
                0,
            );
            if !forward || self.range_index < range_index {
                self.set_range_index(range_index);
            }
        }
        self.next();
    }

    fn forward(&mut self, pos: i64, side: i32) {
        if cmp2(self.to, pos, self.end_side(), side) == Ordering::Less {
            self.goto_inner(pos, side, true);
        }
    }

    fn next(&mut self) {
        loop {
            if self.chunk_index == self.layer.0.chunks.len() {
                self.from = FAR;
                self.to = FAR;
                self.value = None;
                break;
            }
            let chunk_pos = self.layer.0.chunk_pos[self.chunk_index];
            let chunk = self.layer.0.chunks[self.chunk_index].clone();
            self.from = chunk_pos + chunk.from[self.range_index];
            self.to = chunk_pos + chunk.to[self.range_index];
            let value = chunk.value[self.range_index].clone();
            let is_point = value.point();
            self.value = Some(value);
            self.set_range_index(self.range_index + 1);
            if self.min_point < 0 || is_point && self.to - self.from >= self.min_point {
                break;
            }
        }
    }

    fn set_range_index(&mut self, index: usize) {
        if index == self.layer.0.chunks[self.chunk_index].value.len() {
            self.chunk_index += 1;
            if self.skip.is_some() {
                while self.chunk_index < self.layer.0.chunks.len() && self.skipped(self.chunk_index)
                {
                    self.chunk_index += 1;
                }
            }
            self.range_index = 0;
        } else {
            self.range_index = index;
        }
    }

    fn next_chunk(&mut self) {
        self.chunk_index += 1;
        self.range_index = 0;
        self.next();
    }

    fn compare(&self, other: &LayerCursor<T>) -> Ordering {
        self.from
            .cmp(&other.from)
            .then(self.start_side().cmp(&other.start_side()))
            .then(self.rank.cmp(&other.rank))
            .then(self.to.cmp(&other.to))
            .then(self.end_side().cmp(&other.end_side()))
    }
}

struct HeapCursor<T> {
    heap: Vec<LayerCursor<T>>,
    from: i64,
    to: i64,
    value: Option<Arc<T>>,
    rank: i64,
}

impl<T: RangeValue> HeapCursor<T> {
    fn goto(&mut self, pos: i64, side: i32) {
        for cur in &mut self.heap {
            cur.goto(pos, side);
        }
        for i in (0..=self.heap.len() >> 1).rev() {
            heap_bubble(&mut self.heap, i);
        }
        self.next();
    }

    fn forward(&mut self, pos: i64, side: i32) {
        for cur in &mut self.heap {
            cur.forward(pos, side);
        }
        for i in (0..=self.heap.len() >> 1).rev() {
            heap_bubble(&mut self.heap, i);
        }
        let end_side = self.value.as_ref().map_or(0, |v| v.end_side());
        if cmp2(self.to, pos, end_side, side) == Ordering::Less {
            self.next();
        }
    }

    fn next(&mut self) {
        if self.heap.is_empty() {
            self.from = FAR;
            self.to = FAR;
            self.value = None;
            self.rank = -1;
        } else {
            let top = &mut self.heap[0];
            self.from = top.from;
            self.to = top.to;
            self.value = top.value.clone();
            self.rank = top.rank;
            if top.value.is_some() {
                top.next();
            }
            heap_bubble(&mut self.heap, 0);
        }
    }
}

fn heap_bubble<T: RangeValue>(heap: &mut [LayerCursor<T>], mut index: usize) {
    loop {
        let mut child_index = (index << 1) + 1;
        if child_index >= heap.len() {
            break;
        }
        if child_index + 1 < heap.len()
            && heap[child_index].compare(&heap[child_index + 1]) != Ordering::Less
        {
            child_index += 1;
        }
        if heap[index].compare(&heap[child_index]) == Ordering::Less {
            break;
        }
        heap.swap(index, child_index);
        index = child_index;
    }
}

// Upstream returns a single `LayerCursor` when there is one layer and a
// `HeapCursor` otherwise.
enum Cursor<T> {
    Layer(LayerCursor<T>),
    Heap(HeapCursor<T>),
}

impl<T: RangeValue> Cursor<T> {
    fn from_sets(
        sets: &[RangeSet<T>],
        skip: Option<Arc<HashSet<usize>>>,
        min_point: i64,
    ) -> Cursor<T> {
        let mut heap = Vec::new();
        for (i, set) in sets.iter().enumerate() {
            let mut cur = set.clone();
            while !cur.is_empty() {
                if cur.0.max_point >= min_point {
                    heap.push(LayerCursor::new(
                        cur.clone(),
                        skip.clone(),
                        min_point,
                        i as i64,
                    ));
                }
                cur = cur.next_layer().clone();
            }
        }
        if heap.len() == 1 {
            Cursor::Layer(heap.pop().expect("one cursor"))
        } else {
            Cursor::Heap(HeapCursor {
                heap,
                from: 0,
                to: 0,
                value: None,
                rank: 0,
            })
        }
    }

    fn goto(&mut self, pos: i64, side: i32) {
        match self {
            Cursor::Layer(c) => c.goto(pos, side),
            Cursor::Heap(c) => c.goto(pos, side),
        }
    }

    fn forward(&mut self, pos: i64, side: i32) {
        match self {
            Cursor::Layer(c) => c.forward(pos, side),
            Cursor::Heap(c) => c.forward(pos, side),
        }
    }

    fn next(&mut self) {
        match self {
            Cursor::Layer(c) => c.next(),
            Cursor::Heap(c) => c.next(),
        }
    }

    fn from(&self) -> i64 {
        match self {
            Cursor::Layer(c) => c.from,
            Cursor::Heap(c) => c.from,
        }
    }

    fn to(&self) -> i64 {
        match self {
            Cursor::Layer(c) => c.to,
            Cursor::Heap(c) => c.to,
        }
    }

    fn value(&self) -> Option<&Arc<T>> {
        match self {
            Cursor::Layer(c) => c.value.as_ref(),
            Cursor::Heap(c) => c.value.as_ref(),
        }
    }

    fn rank(&self) -> i64 {
        match self {
            Cursor::Layer(c) => c.rank,
            Cursor::Heap(c) => c.rank,
        }
    }

    fn start_side(&self) -> i32 {
        self.value().map_or(0, |v| v.start_side())
    }
}

/// Moves over ranges in order (`RangeCursor`). Unlike a Rust iterator, it
/// starts at the first range: read it, then call [`next`](Self::next).
pub struct RangeCursor<T>(Cursor<T>);

impl<T: RangeValue> RangeCursor<T> {
    /// Move to the next range.
    pub fn next(&mut self) {
        self.0.next();
    }

    /// Jump to the ranges that end at or after `pos`.
    pub fn goto(&mut self, pos: usize) {
        self.0.goto(pos as i64, -FAR_SIDE);
    }

    /// The current range's value, or `None` at the end.
    pub fn value(&self) -> Option<&Arc<T>> {
        self.0.value()
    }

    /// The current range's start (1e9 at the end).
    pub fn from(&self) -> usize {
        self.0.from() as usize
    }

    /// The current range's end (1e9 at the end).
    pub fn to(&self) -> usize {
        self.0.to() as usize
    }

    /// The index, among the iterated sets, of the set the current range
    /// comes from; -1 at the end.
    pub fn rank(&self) -> i64 {
        self.0.rank()
    }
}

struct SpanCursor<T> {
    cursor: Cursor<T>,
    active: Vec<Arc<T>>,
    active_to: Vec<i64>,
    active_rank: Vec<i64>,
    min_active: i64,
    // A currently active point range, if any.
    point: Option<Arc<T>>,
    point_from: i64,
    point_rank: i64,
    to: i64,
    end_side: i32,
    // The amount of open active ranges at the start of the iterator, not
    // including points.
    open_start: i64,
}

// Upstream reads `value[-1].endSide` and throws when a position equals the
// 1e9 sentinel before anything was found; positions never reach it, and
// this port takes the candidate instead.
fn find_min_index<T: RangeValue>(value: &[Arc<T>], array: &[i64]) -> i64 {
    let mut found: i64 = -1;
    let mut found_pos = FAR;
    for (i, &pos) in array.iter().enumerate() {
        let ord = pos.cmp(&found_pos).then_with(|| {
            if found < 0 {
                Ordering::Less
            } else {
                value[i].end_side().cmp(&value[found as usize].end_side())
            }
        });
        if ord == Ordering::Less {
            found = i as i64;
            found_pos = pos;
        }
    }
    found
}

impl<T: RangeValue> SpanCursor<T> {
    fn new(sets: &[RangeSet<T>], skip: Option<Arc<HashSet<usize>>>, min_point: i64) -> Self {
        SpanCursor {
            cursor: Cursor::from_sets(sets, skip, min_point),
            active: Vec::new(),
            active_to: Vec::new(),
            active_rank: Vec::new(),
            min_active: -1,
            point: None,
            point_from: 0,
            point_rank: 0,
            to: -FAR,
            end_side: 0,
            open_start: -1,
        }
    }

    fn goto(&mut self, pos: i64, side: i32) {
        self.cursor.goto(pos, side);
        self.active.clear();
        self.active_to.clear();
        self.active_rank.clear();
        self.min_active = -1;
        self.to = pos;
        self.end_side = side;
        self.open_start = -1;
        self.next();
    }

    fn forward(&mut self, pos: i64, side: i32) {
        while self.min_active > -1 {
            let a = self.min_active as usize;
            if cmp2(self.active_to[a], pos, self.active[a].end_side(), side) != Ordering::Less {
                break;
            }
            self.remove_active(a);
        }
        self.cursor.forward(pos, side);
    }

    fn remove_active(&mut self, index: usize) {
        self.active.remove(index);
        self.active_to.remove(index);
        self.active_rank.remove(index);
        self.min_active = find_min_index(&self.active, &self.active_to);
    }

    fn add_active(&mut self, track_open: Option<&mut Vec<i64>>) {
        let value = self.cursor.value().expect("a range").clone();
        let (to, rank) = (self.cursor.to(), self.cursor.rank());
        // Order active marks by rank first, then by size.
        let mut i = 0;
        while i < self.active_rank.len()
            && rank
                .cmp(&self.active_rank[i])
                .then(to.cmp(&self.active_to[i]))
                == Ordering::Greater
        {
            i += 1;
        }
        self.active.insert(i, value);
        self.active_to.insert(i, to);
        self.active_rank.insert(i, rank);
        if let Some(track_open) = track_open {
            track_open.insert(i, self.cursor.from());
        }
        self.min_active = find_min_index(&self.active, &self.active_to);
    }

    // Afterwards, if `point` is set, the next range is a point; otherwise
    // it is a regular range, covered by `active`.
    fn next(&mut self) {
        let from = self.to;
        let was_point = self.point.is_some();
        self.point = None;
        let mut track_open: Option<Vec<i64>> = (self.open_start < 0).then(Vec::new);
        loop {
            let a = self.min_active;
            if a > -1
                && cmp2(
                    self.active_to[a as usize],
                    self.cursor.from(),
                    self.active[a as usize].end_side(),
                    self.cursor.start_side(),
                ) == Ordering::Less
            {
                let a = a as usize;
                if self.active_to[a] > from {
                    self.to = self.active_to[a];
                    self.end_side = self.active[a].end_side();
                    break;
                }
                self.remove_active(a);
                if let Some(track_open) = &mut track_open {
                    track_open.remove(a);
                }
            } else if self.cursor.value().is_none() {
                self.to = FAR;
                self.end_side = FAR_SIDE;
                break;
            } else if self.cursor.from() > from {
                self.to = self.cursor.from();
                self.end_side = self.cursor.start_side();
                break;
            } else {
                let next_val = self.cursor.value().expect("a range").clone();
                if !next_val.point() {
                    // Opening a range.
                    self.add_active(track_open.as_mut());
                    self.cursor.next();
                } else if was_point
                    && self.cursor.to() == self.to
                    && self.cursor.from() < self.cursor.to()
                {
                    // Ignore non-empty points that end exactly at the end
                    // of the previous point.
                    self.cursor.next();
                } else {
                    // A new point.
                    self.point_from = self.cursor.from();
                    self.point_rank = self.cursor.rank();
                    self.to = self.cursor.to();
                    self.end_side = next_val.end_side();
                    self.point = Some(next_val);
                    self.cursor.next();
                    self.forward(self.to, self.end_side);
                    break;
                }
            }
        }
        if let Some(track_open) = track_open {
            self.open_start = track_open
                .iter()
                .rev()
                .take_while(|&&pos| pos < from)
                .count() as i64;
        }
    }

    fn active_for_point(&self, to: i64) -> Vec<Arc<T>> {
        let Some(point) = &self.point else {
            return self.active.clone();
        };
        if self.active.is_empty() {
            return Vec::new();
        }
        let mut active = Vec::new();
        for i in (0..self.active.len()).rev() {
            if self.active_rank[i] < self.point_rank {
                break;
            }
            if self.active_to[i] > to
                || self.active_to[i] == to && self.active[i].end_side() >= point.end_side()
            {
                active.push(self.active[i].clone());
            }
        }
        active.reverse();
        active
    }

    fn open_end(&self, to: i64) -> usize {
        self.active_to
            .iter()
            .rev()
            .take_while(|&&end| end > to)
            .count()
    }
}

fn compare<T: RangeValue>(
    a: &mut SpanCursor<T>,
    start_a: i64,
    b: &mut SpanCursor<T>,
    start_b: i64,
    length: i64,
    comparator: &mut dyn RangeComparator<T>,
) {
    a.goto(start_a, -FAR_SIDE);
    b.goto(start_b, -FAR_SIDE);
    let end_b = start_b + length;
    let mut pos = start_b;
    let d_pos = start_b - start_a;
    let bounds = comparator.wants_bound_changes();
    let mut bound_change = false;
    loop {
        let d_end = (a.to + d_pos) - b.to;
        let diff = d_end.cmp(&0).then(a.end_side.cmp(&b.end_side));
        let end = if diff == Ordering::Less {
            a.to + d_pos
        } else {
            b.to
        };
        let clip_end = end.min(end_b);
        if a.point.is_some() || b.point.is_some() {
            let same = match (&a.point, &b.point) {
                (Some(pa), Some(pb)) => {
                    cmp_val(pa, pb)
                        && same_values(&a.active_for_point(a.to), &b.active_for_point(b.to))
                }
                _ => false,
            };
            if !same {
                comparator.compare_point(
                    pos as usize,
                    clip_end as usize,
                    a.point.as_ref(),
                    b.point.as_ref(),
                );
            }
            bound_change = false;
        } else {
            if bound_change {
                comparator.bound_change(pos as usize);
                bound_change = false;
            }
            if clip_end > pos && !same_values(&a.active, &b.active) {
                comparator.compare_range(pos as usize, clip_end as usize, &a.active, &b.active);
            }
            if bounds && clip_end < end_b && (d_end != 0 || a.open_end(end) != b.open_end(end)) {
                bound_change = true;
            }
        }
        if end > end_b {
            break;
        }
        pos = end;
        if diff != Ordering::Greater {
            a.next();
        }
        if diff != Ordering::Less {
            b.next();
        }
    }
}

fn same_values<T: RangeValue>(a: &[Arc<T>], b: &[Arc<T>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| cmp_val(a, b))
}
