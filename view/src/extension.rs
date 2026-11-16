//! Part of `extension.ts`: [`ChangedRange`], the view's update object
//! ([`ViewUpdate`]), scroll targets and the view's facets. View plugins
//! come with the gutters, which are the first to need them.

use std::sync::{Arc, LazyLock};

use editor_state::{
    Callback, ChangeDesc, ChangeSet, EditorSelection, EditorState, Facet, FacetConfig, Line,
    RangeSet, SelectionRange, SpanIterator, StateEffectType, Transaction,
};

use crate::attributes::Attrs;
use crate::bidi::{auto_direction, Direction, Isolate};
use crate::decoration::{BlockWrapper, Decoration, DecorationSet, DecorationSpec};

/// A changed range, in positions of the old (`a`) and the new (`b`)
/// document (`ChangedRange`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChangedRange {
    /// Start in the old document.
    pub from_a: usize,
    /// End in the old document.
    pub to_a: usize,
    /// Start in the new document.
    pub from_b: usize,
    /// End in the new document.
    pub to_b: usize,
}

impl ChangedRange {
    /// A changed range.
    pub fn new(from_a: usize, to_a: usize, from_b: usize, to_b: usize) -> ChangedRange {
        ChangedRange {
            from_a,
            to_a,
            from_b,
            to_b,
        }
    }

    /// The smallest range covering both (`join`).
    pub fn join(&self, other: &ChangedRange) -> ChangedRange {
        ChangedRange::new(
            self.from_a.min(other.from_a),
            self.to_a.max(other.to_a),
            self.from_b.min(other.from_b),
            self.to_b.max(other.to_b),
        )
    }

    /// Add this range to a set sorted by position, joining it with the
    /// ranges it touches (`addToSet`).
    pub fn add_to_set(&self, set: &mut Vec<ChangedRange>) {
        let mut i = set.len();
        let mut me = *self;
        while i > 0 {
            let range = set[i - 1];
            if range.from_a > me.to_a {
                i -= 1;
                continue;
            }
            if range.to_a < me.from_a {
                break;
            }
            me = me.join(&range);
            set.remove(i - 1);
            i -= 1;
        }
        set.insert(i, me);
    }

    /// Extend `diff` to cover the content in `ranges`, a flat list of
    /// `from_b`, `to_b` pairs in unchanged content (`extendWithRanges`).
    pub fn extend_with_ranges(diff: &[ChangedRange], ranges: &[usize]) -> Vec<ChangedRange> {
        if ranges.is_empty() {
            return diff.to_vec();
        }
        const FAR: i64 = 1_000_000_000;
        let mut result = Vec::new();
        let (mut d_i, mut r_i, mut off) = (0usize, 0usize, 0i64);
        loop {
            let next_d = diff.get(d_i).map_or(FAR, |d| d.from_b as i64);
            let next_r = ranges.get(r_i).map_or(FAR, |&r| r as i64);
            let from_b = next_d.min(next_r);
            if from_b == FAR {
                break;
            }
            let from_a = from_b + off;
            let (mut to_b, mut to_a) = (from_b, from_a);
            loop {
                if r_i < ranges.len() && ranges[r_i] as i64 <= to_b {
                    let end = ranges[r_i + 1] as i64;
                    r_i += 2;
                    to_b = to_b.max(end);
                    let mut i = d_i;
                    while i < diff.len() && diff[i].from_b as i64 <= to_b {
                        off = diff[i].to_a as i64 - diff[i].to_b as i64;
                        i += 1;
                    }
                    to_a = to_a.max(end + off);
                } else if d_i < diff.len() && diff[d_i].from_b as i64 <= to_b {
                    let next = diff[d_i];
                    d_i += 1;
                    to_b = to_b.max(next.to_b as i64);
                    to_a = to_a.max(next.to_a as i64);
                    off = next.to_a as i64 - next.to_b as i64;
                } else {
                    break;
                }
            }
            result.push(ChangedRange::new(
                from_a as usize,
                to_a as usize,
                from_b as usize,
                to_b as usize,
            ));
        }
        result
    }
}

/// The flags of a [`ViewUpdate`] (`UpdateFlag`).
pub mod update_flag {
    /// The editor gained or lost focus.
    pub const FOCUS: u32 = 1;
    /// The height of the content changed.
    pub const HEIGHT: u32 = 2;
    /// The viewport changed.
    pub const VIEWPORT: u32 = 4;
    /// The viewport moved to cover a different part of the document.
    pub const VIEWPORT_MOVED: u32 = 8;
    /// The editor's geometry changed.
    pub const GEOMETRY: u32 = 16;
}

/// Describes an update of the view (`ViewUpdate`).
#[derive(Clone)]
pub struct ViewUpdate {
    /// The changes made to the document by this update.
    pub changes: ChangeSet,
    /// The state before the update.
    pub start_state: EditorState,
    /// The new state.
    pub state: EditorState,
    /// The transactions in this update. May be empty.
    pub transactions: Vec<Transaction>,
    /// The update's [`update_flag`]s.
    pub flags: u32,
    /// The ranges of the document the changes touch.
    pub changed_ranges: Vec<ChangedRange>,
}

impl ViewUpdate {
    /// An update from `start_state` to `state` by `transactions`
    /// (`ViewUpdate.create`).
    pub fn create(
        start_state: &EditorState,
        state: &EditorState,
        transactions: Vec<Transaction>,
    ) -> ViewUpdate {
        let mut changes = ChangeSet::empty(start_state.doc().len());
        for tr in &transactions {
            changes = changes.compose(tr.changes());
        }
        let mut changed_ranges = Vec::new();
        changes.iter_changed_ranges(
            |from_a, to_a, from_b, to_b| {
                changed_ranges.push(ChangedRange::new(from_a, to_a, from_b, to_b))
            },
            false,
        );
        ViewUpdate {
            changes,
            start_state: start_state.clone(),
            state: state.clone(),
            transactions,
            flags: 0,
            changed_ranges,
        }
    }

    /// Whether the viewport or visible ranges changed (`viewportChanged`).
    pub fn viewport_changed(&self) -> bool {
        self.flags & update_flag::VIEWPORT > 0
    }

    /// Whether the viewport moved to a different part of the document
    /// (`viewportMoved`).
    pub fn viewport_moved(&self) -> bool {
        self.flags & update_flag::VIEWPORT_MOVED > 0
    }

    /// Whether the height of a block or the document changed
    /// (`heightChanged`).
    pub fn height_changed(&self) -> bool {
        self.flags & update_flag::HEIGHT > 0
    }

    /// Whether the document changed or the size of the editor or its
    /// content may have (`geometryChanged`).
    pub fn geometry_changed(&self) -> bool {
        self.doc_changed() || self.flags & (update_flag::GEOMETRY | update_flag::HEIGHT) > 0
    }

    /// Whether the editor gained or lost focus (`focusChanged`).
    pub fn focus_changed(&self) -> bool {
        self.flags & update_flag::FOCUS > 0
    }

    /// Whether the document changed (`docChanged`).
    pub fn doc_changed(&self) -> bool {
        !self.changes.is_empty()
    }

    /// Whether a transaction set the selection (`selectionSet`).
    pub fn selection_set(&self) -> bool {
        self.transactions.iter().any(|tr| tr.selection().is_some())
    }

    /// Whether nothing happened (`empty`).
    pub fn empty(&self) -> bool {
        self.flags == 0 && self.transactions.is_empty()
    }
}

/// How to scroll a position into view (`ScrollStrategy`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollStrategy {
    /// Scroll as little as possible.
    Nearest,
    /// Put the position at the start of the view.
    Start,
    /// Put it at the end.
    End,
    /// Center it.
    Center,
}

/// A request to scroll a range into view (`ScrollTarget`).
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollTarget {
    /// The range to show.
    pub range: SelectionRange,
    /// The vertical strategy.
    pub y: ScrollStrategy,
    /// The horizontal strategy.
    pub x: ScrollStrategy,
    /// The vertical margin.
    pub y_margin: f64,
    /// The horizontal margin.
    pub x_margin: f64,
    /// Whether this is a scroll snapshot rather than a request: `range`
    /// is in the reference line, `y_margin` the distance from its top to
    /// the editor's and `x_margin` the editor's `scrollLeft`.
    pub is_snapshot: bool,
}

impl ScrollTarget {
    /// A scroll target (`new ScrollTarget`).
    pub fn new(
        range: SelectionRange,
        y: ScrollStrategy,
        x: ScrollStrategy,
        y_margin: f64,
        x_margin: f64,
    ) -> ScrollTarget {
        ScrollTarget {
            range,
            y,
            x,
            y_margin,
            x_margin,
            is_snapshot: false,
        }
    }

    /// Map the target through changes (`map`).
    pub fn map(&self, changes: &ChangeDesc) -> ScrollTarget {
        if changes.is_empty() {
            return self.clone();
        }
        ScrollTarget {
            range: self.range.map(changes, -1),
            ..self.clone()
        }
    }

    /// Clip the target to the document of `state` (`clip`).
    pub fn clip(&self, state: &EditorState) -> ScrollTarget {
        if self.range.to() <= state.doc().len() {
            return self.clone();
        }
        ScrollTarget {
            range: EditorSelection::cursor(state.doc().len()),
            ..self.clone()
        }
    }
}

/// The effect that scrolls a target into view (`scrollIntoView`).
pub(crate) static SCROLL_INTO_VIEW: LazyLock<StateEffectType<ScrollTarget>> = LazyLock::new(|| {
    StateEffectType::define_with_map(|t: &ScrollTarget, ch: &ChangeDesc| Some(t.map(ch)))
});

/// Whether the editor is editable (`editable`).
pub(crate) static EDITABLE: LazyLock<Facet<bool, bool>> = LazyLock::new(|| {
    Facet::define_with(FacetConfig::new(|values: &[bool]| {
        values.first().copied().unwrap_or(true)
    }))
});

/// Whether the native selection is hidden (`nativeSelectionHidden`).
pub(crate) static NATIVE_SELECTION_HIDDEN: LazyLock<Facet<bool, bool>> = LazyLock::new(|| {
    Facet::define_with(FacetConfig::new(|values: &[bool]| values.contains(&true)))
});

/// Decorations drawn in the content (`decorations`). Upstream also takes
/// functions of the view, which come with view plugins.
pub(crate) static DECORATIONS: LazyLock<Facet<DecorationSet>> = LazyLock::new(Facet::define);

/// Decorations drawn around the content's other decorations
/// (`outerDecorations`). Upstream also takes functions of the view.
pub(crate) static OUTER_DECORATIONS: LazyLock<Facet<DecorationSet>> = LazyLock::new(Facet::define);

/// Whether the text direction is read per line (`perLineTextDirection`).
pub(crate) static PER_LINE_TEXT_DIRECTION: LazyLock<Facet<bool, bool>> = LazyLock::new(|| {
    Facet::define_with(FacetConfig::new(|values: &[bool]| values.contains(&true)))
});

/// Ranges ordered as bidi isolates (`bidiIsolatedRanges`). Upstream also
/// takes functions of the view.
pub(crate) static BIDI_ISOLATED_RANGES: LazyLock<Facet<DecorationSet>> =
    LazyLock::new(Facet::define);

// An isolate whose direction `dir=auto` still has to resolve.
struct UnresolvedIsolate {
    from: usize,
    to: usize,
    direction: Option<Direction>,
    inner: Vec<UnresolvedIsolate>,
}

// Builds the isolates of a line from the spans of the isolated ranges.
struct IsolateBuilder {
    line_from: usize,
    result: Vec<UnresolvedIsolate>,
}

impl SpanIterator<Decoration> for IsolateBuilder {
    fn span(&mut self, from_doc: usize, to_doc: usize, active: &[Arc<Decoration>], open: usize) {
        let (from, to) = (from_doc - self.line_from, to_doc - self.line_from);
        let mut level = &mut self.result;
        let mut open = open as isize;
        for deco in active.iter().rev() {
            let direction = match deco.spec() {
                DecorationSpec::Mark(spec) => spec.bidi_isolate,
                _ => None,
            };
            let extends = open > 0
                && level
                    .last()
                    .is_some_and(|last| last.to == from && last.direction == direction);
            if extends {
                let update = level.last_mut().expect("the isolate to extend");
                update.to = to;
                level = &mut update.inner;
            } else {
                level.push(UnresolvedIsolate {
                    from,
                    to,
                    direction,
                    inner: Vec::new(),
                });
                level = &mut level.last_mut().expect("the isolate just added").inner;
            }
            open -= 1;
        }
    }

    fn point(
        &mut self,
        _from: usize,
        _to: usize,
        _value: &Arc<Decoration>,
        _active: &[Arc<Decoration>],
        _open_start: usize,
        _index: usize,
    ) {
    }
}

/// The bidi isolates of `line` (`getIsolatedRanges`).
pub(crate) fn get_isolated_ranges(state: &EditorState, line: &Line) -> Vec<Isolate> {
    let sets = state.facet(&BIDI_ISOLATED_RANGES);
    if sets.is_empty() {
        return Vec::new();
    }
    let mut builder = IsolateBuilder {
        line_from: line.from,
        result: Vec::new(),
    };
    RangeSet::spans(sets, line.from, line.to, &mut builder, -1);
    resolve_isolates(line.text, builder.result)
}

// `resolveIsolates`: give `dir=auto` isolates the direction of their
// first strong character outside inner isolates, or left-to-right.
fn resolve_isolates(text: &str, isolates: Vec<UnresolvedIsolate>) -> Vec<Isolate> {
    isolates
        .into_iter()
        .map(|iso| {
            let direction = iso.direction.unwrap_or_else(|| {
                let (mut pos, mut j) = (iso.from, 0);
                while pos < iso.to {
                    let next_inner = iso.inner.get(j);
                    let end = next_inner.map_or(iso.to, |inner| inner.from);
                    if let Some(dir) = auto_direction(text, pos, end) {
                        return dir;
                    }
                    let Some(next_inner) = next_inner else {
                        break;
                    };
                    pos = next_inner.to;
                    j += 1;
                }
                Direction::Ltr
            });
            Isolate {
                from: iso.from,
                to: iso.to,
                direction,
                inner: resolve_isolates(text, iso.inner),
            }
        })
        .collect()
}

/// Block wrappers (`blockWrappers`). Upstream also takes functions of the
/// view.
pub(crate) static BLOCK_WRAPPERS: LazyLock<Facet<RangeSet<BlockWrapper>>> =
    LazyLock::new(Facet::define);

/// Attributes for the content element (`contentAttributes`). Upstream
/// also takes functions of the view, which come with view plugins.
pub(crate) static CONTENT_ATTRIBUTES: LazyLock<Facet<Attrs>> = LazyLock::new(Facet::define);

/// Attributes for the editor element (`editorAttributes`). Upstream also
/// takes functions of the view.
pub(crate) static EDITOR_ATTRIBUTES: LazyLock<Facet<Attrs>> = LazyLock::new(Facet::define);

/// The margin to keep around the cursor when a transaction scrolls it into
/// view (`EditorView.cursorScrollMargin`): `(x, y)`.
pub(crate) static CURSOR_SCROLL_MARGIN: LazyLock<Facet<(f64, f64), (f64, f64)>> =
    LazyLock::new(|| {
        Facet::define_with(FacetConfig::new(|values: &[(f64, f64)]| {
            // The last value wins; a single number upstream is `(n, n)`.
            values.last().copied().unwrap_or((5.0, 5.0))
        }))
    });

/// Margins, each optional (`Partial<Rect>`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Margins {
    /// The left margin.
    pub left: Option<f64>,
    /// The right margin.
    pub right: Option<f64>,
    /// The top margin.
    pub top: Option<f64>,
    /// The bottom margin.
    pub bottom: Option<f64>,
}

/// A source of extra margins around scroll targets (`scrollMargins`).
pub type ScrollMarginSource = Callback<dyn Fn(&crate::EditorView) -> Option<Margins> + Send + Sync>;

/// Extra margins around scroll targets (`scrollMargins`).
pub(crate) static SCROLL_MARGINS: LazyLock<Facet<ScrollMarginSource>> =
    LazyLock::new(Facet::define);

/// The margins from [`SCROLL_MARGINS`] (`getScrollMargins`): left, right,
/// top and bottom.
pub(crate) fn get_scroll_margins(view: &crate::EditorView) -> (f64, f64, f64, f64) {
    let (mut left, mut right, mut top, mut bottom) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for source in view.state().facet(&SCROLL_MARGINS).iter() {
        if let Some(m) = source(view) {
            if let Some(v) = m.left {
                left = left.max(v);
            }
            if let Some(v) = m.right {
                right = right.max(v);
            }
            if let Some(v) = m.top {
                top = top.max(v);
            }
            if let Some(v) = m.bottom {
                bottom = bottom.max(v);
            }
        }
    }
    (left, right, top, bottom)
}
