//! Port of `decoration.ts`: decorations, widgets and block wrappers.
//!
//! Upstream's `Decoration` is a class hierarchy (`MarkDecoration`,
//! `LineDecoration`, `PointDecoration`); here it is one type whose
//! [`kind`](Decoration::kind) tells them apart. Upstream's spec objects
//! become the typed [`MarkSpec`], [`WidgetSpec`], [`ReplaceSpec`] and
//! [`LineSpec`]; the extra properties upstream allows on a spec go in
//! their `extra` field.
//!
//! [`WidgetType`] builds its DOM on Div's (`toDOM`, `updateDOM`,
//! `destroy`); `ignoreEvent` and `coordsAt` come with the view's event
//! handling and measurement.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, LazyLock};

use div::{Document, Element};
use editor_state::{MapMode, Range, RangeError, RangeSet, RangeValue};

use crate::attributes::{attrs_eq, combine_attrs, Attrs};
use crate::bidi::Direction;

/// Extra data a decoration spec carries for the code that made it
/// (upstream's `[other: string]: any`). It is not compared.
pub type SpecData = Arc<dyn Any + Send + Sync>;

/// A widget drawn in the content (`WidgetType`). Using a description like
/// this delays building the widget's DOM until it is needed, and avoids
/// redrawing a widget when its decoration is recreated.
pub trait WidgetType: Any + Send + Sync {
    /// Build the widget's DOM (`toDOM`). Upstream passes the editor view;
    /// until the view is ported, this gets the document to build in.
    fn to_dom(&self, document: &Document) -> Result<Element, div::Error>;

    /// Update `dom`, drawn by `from`, a widget of the same type, to show
    /// this widget instead (`updateDOM`). Returns whether it did; the
    /// default returns `false`, so the widget is drawn anew.
    fn update_dom(&self, _dom: &Element, _document: &Document, _from: &Self) -> bool
    where
        Self: Sized,
    {
        false
    }

    /// Called when the widget's DOM is removed from the editor
    /// (`destroy`).
    fn destroy(&self, _dom: &Element) {}

    /// Compare with another widget of the same type (`eq`). Used to avoid
    /// redrawing a widget when it is replaced by an equal one. The default
    /// returns `false`, so new instances are always redrawn.
    fn eq(&self, _other: &Self) -> bool
    where
        Self: Sized,
    {
        false
    }

    /// The estimated height of the widget, used for content that has not
    /// been drawn, or -1 when unknown (`estimatedHeight`). Defaults to -1.
    fn estimated_height(&self) -> f64 {
        -1.0
    }

    /// For an inline widget displayed inline (not `inline-block`) that
    /// introduces line breaks, the number of breaks (`lineBreaks`).
    /// Defaults to 0.
    fn line_breaks(&self) -> usize {
        0
    }

    /// Upstream's internal `isHidden`.
    #[doc(hidden)]
    fn is_hidden(&self) -> bool {
        false
    }

    /// Upstream's internal `editable`.
    #[doc(hidden)]
    fn editable(&self) -> bool {
        false
    }
}

trait DynWidget: Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn compare(&self, other: &dyn DynWidget) -> bool;
    fn estimated_height(&self) -> f64;
    fn line_breaks(&self) -> usize;
    fn is_hidden(&self) -> bool;
    fn editable(&self) -> bool;
    fn type_name(&self) -> &'static str;
    fn to_dom(&self, document: &Document) -> Result<Element, div::Error>;
    fn update_dom(&self, dom: &Element, document: &Document, from: &dyn DynWidget) -> bool;
    fn destroy(&self, dom: &Element);
}

impl<W: WidgetType> DynWidget for W {
    fn as_any(&self) -> &dyn Any {
        self
    }

    // `this.constructor == other.constructor && this.eq(other)`.
    fn compare(&self, other: &dyn DynWidget) -> bool {
        other
            .as_any()
            .downcast_ref::<W>()
            .is_some_and(|other| std::ptr::eq(self, other) || WidgetType::eq(self, other))
    }

    fn estimated_height(&self) -> f64 {
        WidgetType::estimated_height(self)
    }

    fn line_breaks(&self) -> usize {
        WidgetType::line_breaks(self)
    }

    fn is_hidden(&self) -> bool {
        WidgetType::is_hidden(self)
    }

    fn editable(&self) -> bool {
        WidgetType::editable(self)
    }

    fn type_name(&self) -> &'static str {
        std::any::type_name::<W>()
    }

    fn to_dom(&self, document: &Document) -> Result<Element, div::Error> {
        WidgetType::to_dom(self, document)
    }

    fn update_dom(&self, dom: &Element, document: &Document, from: &dyn DynWidget) -> bool {
        from.as_any()
            .downcast_ref::<W>()
            .is_some_and(|from| WidgetType::update_dom(self, dom, document, from))
    }

    fn destroy(&self, dom: &Element) {
        WidgetType::destroy(self, dom)
    }
}

/// A shared [`WidgetType`] instance.
#[derive(Clone)]
pub struct Widget(Arc<dyn DynWidget>);

impl Widget {
    /// Share `widget`.
    pub fn new<W: WidgetType>(widget: W) -> Widget {
        Widget(Arc::new(widget))
    }

    /// Whether this is the same widget, or an equal one of the same type
    /// (`compare`).
    pub fn compare(&self, other: &Widget) -> bool {
        std::ptr::addr_eq(Arc::as_ptr(&self.0), Arc::as_ptr(&other.0)) || self.0.compare(&*other.0)
    }

    /// The widget, if it is a `W`.
    pub fn downcast_ref<W: WidgetType>(&self) -> Option<&W> {
        self.0.as_any().downcast_ref()
    }

    /// See [`WidgetType::estimated_height`].
    pub fn estimated_height(&self) -> f64 {
        self.0.estimated_height()
    }

    /// See [`WidgetType::line_breaks`].
    pub fn line_breaks(&self) -> usize {
        self.0.line_breaks()
    }

    #[doc(hidden)]
    pub fn is_hidden(&self) -> bool {
        self.0.is_hidden()
    }

    #[doc(hidden)]
    pub fn editable(&self) -> bool {
        self.0.editable()
    }

    /// Whether both share one instance (upstream's `==` on widgets).
    pub(crate) fn ptr_eq(&self, other: &Widget) -> bool {
        std::ptr::addr_eq(Arc::as_ptr(&self.0), Arc::as_ptr(&other.0))
    }

    /// Whether both are of one type (`a.constructor == b.constructor`).
    pub(crate) fn same_type(&self, other: &Widget) -> bool {
        self.0.as_any().type_id() == other.0.as_any().type_id()
    }

    pub(crate) fn to_dom(&self, document: &Document) -> Result<Element, div::Error> {
        self.0.to_dom(document)
    }

    /// `updateDOM` for `from`, which must be of this widget's type.
    pub(crate) fn update_dom(&self, dom: &Element, document: &Document, from: &Widget) -> bool {
        self.0.update_dom(dom, document, &*from.0)
    }

    pub(crate) fn destroy(&self, dom: &Element) {
        self.0.destroy(dom)
    }
}

impl fmt::Debug for Widget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Widget({})", self.0.type_name())
    }
}

fn widgets_eq(a: Option<&Widget>, b: Option<&Widget>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.compare(b),
        _ => false,
    }
}

/// How a mark decoration is drawn (`MarkDecorationSpec`).
#[derive(Clone, Default)]
pub struct MarkSpec {
    /// Whether the mark covers its start and end positions, so content
    /// inserted there becomes part of it. Defaults to false.
    pub inclusive: Option<bool>,
    /// Whether the start is inclusive. Overrides `inclusive`.
    pub inclusive_start: Option<bool>,
    /// Whether the end is inclusive. Overrides `inclusive`.
    pub inclusive_end: Option<bool>,
    /// Attributes for the elements that hold the marked text.
    pub attributes: Option<Attrs>,
    /// Shorthand for a `class` attribute.
    pub class: Option<String>,
    /// The wrapping element's tag. Defaults to `span`.
    pub tag_name: Option<String>,
    /// In `bidiIsolatedRanges` sets, the direction of the isolate; `None`
    /// means `dir=auto` (`bidiIsolate`).
    pub bidi_isolate: Option<Direction>,
    /// Extra data for the code that made the decoration.
    pub extra: Option<SpecData>,
}

/// A widget decoration (`WidgetDecorationSpec`).
#[derive(Clone)]
pub struct WidgetSpec {
    /// The widget to draw.
    pub widget: Widget,
    /// Which side of the position the widget is on: positive draws it
    /// after a cursor at the same position, otherwise before. Widgets at
    /// one position are ordered by side. Clamped to -10000..=10000.
    /// Defaults to 0.
    pub side: i32,
    /// For a block widget, order it among the inline widgets by side,
    /// instead of after (positive side) or before them.
    pub inline_order: bool,
    /// Whether this is a block widget, drawn between lines.
    pub block: bool,
    /// Extra data for the code that made the decoration.
    pub extra: Option<SpecData>,
}

impl WidgetSpec {
    /// An inline widget at side 0.
    pub fn new<W: WidgetType>(widget: W) -> WidgetSpec {
        WidgetSpec {
            widget: Widget::new(widget),
            side: 0,
            inline_order: false,
            block: false,
            extra: None,
        }
    }
}

/// A replace decoration (`ReplaceDecorationSpec`).
#[derive(Clone, Default)]
pub struct ReplaceSpec {
    /// The widget drawn in place of the content, if any.
    pub widget: Option<Widget>,
    /// Whether the range covers the positions on its sides. Defaults to
    /// false for inline and true for block replacements.
    pub inclusive: Option<bool>,
    /// Whether the start is inclusive. Overrides `inclusive`.
    pub inclusive_start: Option<bool>,
    /// Whether the end is inclusive. Overrides `inclusive`.
    pub inclusive_end: Option<bool>,
    /// Whether this is a block-level decoration.
    pub block: bool,
    /// Upstream's internal `isBlockGap`: a gap the viewport leaves
    /// undrawn.
    #[doc(hidden)]
    pub is_block_gap: bool,
    /// Extra data for the code that made the decoration.
    pub extra: Option<SpecData>,
}

/// A line decoration (`LineDecorationSpec`).
#[derive(Clone, Default)]
pub struct LineSpec {
    /// Attributes for the element wrapping the line.
    pub attributes: Option<Attrs>,
    /// Shorthand for a `class` attribute.
    pub class: Option<String>,
    /// Extra data for the code that made the decoration.
    pub extra: Option<SpecData>,
}

/// The spec a decoration was made from (`spec`).
#[derive(Clone)]
pub enum DecorationSpec {
    /// From [`Decoration::mark`].
    Mark(MarkSpec),
    /// From [`Decoration::widget`].
    Widget(WidgetSpec),
    /// From [`Decoration::replace`].
    Replace(ReplaceSpec),
    /// From [`Decoration::line`].
    Line(LineSpec),
}

// The sides (`Side` upstream) that order decorations at one position.
const NON_INC_END: i32 = -600_000_000;
const GAP_START: i32 = -500_000_000;
const BLOCK_BEFORE: i32 = -400_000_000;
const BLOCK_INC_START: i32 = -300_000_000;
const LINE: i32 = -200_000_000;
const INLINE_BEFORE: i32 = -100_000_000;
const INLINE_INC_START: i32 = -1;
const INLINE_INC_END: i32 = 1;
const INLINE_AFTER: i32 = 100_000_000;
const BLOCK_INC_END: i32 = 200_000_000;
const BLOCK_AFTER: i32 = 300_000_000;
const GAP_END: i32 = 400_000_000;
const NON_INC_START: i32 = 500_000_000;

/// The types of block in the view (`BlockType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlockType {
    /// A line of text.
    Text,
    /// A block widget associated with the position after it.
    WidgetBefore,
    /// A block widget associated with the position before it.
    WidgetAfter,
    /// A block widget replacing a range of content.
    WidgetRange,
}

/// Which kind of decoration this is: upstream's subclasses.
#[derive(Clone, Copy, Debug)]
pub enum DecorationKind<'a> {
    /// A mark (`MarkDecoration`), with the wrapping element's tag and
    /// attributes.
    Mark {
        /// The wrapping element's tag.
        tag_name: &'a str,
        /// The wrapping element's attributes.
        attrs: &'a Attrs,
    },
    /// A line decoration (`LineDecoration`).
    Line,
    /// A widget or replace decoration (`PointDecoration`).
    Point {
        /// Whether it is block-level.
        block: bool,
        /// Whether it replaces content.
        is_replace: bool,
    },
}

enum Kind {
    Mark { tag_name: String, attrs: Attrs },
    Line,
    Point { block: bool, is_replace: bool },
}

/// Information on how to draw or style a piece of content (`Decoration`).
/// Decorations are used as [`RangeSet`] values, shared in `Arc`s.
pub struct Decoration {
    start_side: i32,
    end_side: i32,
    widget: Option<Widget>,
    spec: DecorationSpec,
    kind: Kind,
}

/// A set of decorated ranges (`DecorationSet`).
pub type DecorationSet = RangeSet<Decoration>;

static NONE: LazyLock<DecorationSet> = LazyLock::new(RangeSet::empty);

fn get_inclusive(
    inclusive: Option<bool>,
    start: Option<bool>,
    end: Option<bool>,
    block: bool,
) -> (bool, bool) {
    (
        start.or(inclusive).unwrap_or(block),
        end.or(inclusive).unwrap_or(block),
    )
}

impl Decoration {
    /// A mark decoration, which styles the content in its range
    /// (`Decoration.mark`). Nested marks create nested elements; the
    /// decorations facet's precedence decides the nesting, with higher
    /// precedence inner. Marks are split at line boundaries and at the
    /// boundaries of lower-precedence decorations.
    pub fn mark(spec: MarkSpec) -> Arc<Decoration> {
        let (start, end) = get_inclusive(
            spec.inclusive,
            spec.inclusive_start,
            spec.inclusive_end,
            false,
        );
        let tag_name = spec.tag_name.clone().unwrap_or_else(|| "span".to_string());
        let attrs = match (&spec.class, &spec.attributes) {
            (Some(class), Some(attributes)) => combine_attrs(
                attributes,
                [("class", class.as_str())].into_iter().collect(),
            ),
            (Some(class), None) => [("class", class.as_str())].into_iter().collect(),
            (None, Some(attributes)) => attributes.clone(),
            (None, None) => Attrs::new(),
        };
        Arc::new(Decoration {
            start_side: if start {
                INLINE_INC_START
            } else {
                NON_INC_START
            },
            end_side: if end { INLINE_INC_END } else { NON_INC_END },
            widget: None,
            spec: DecorationSpec::Mark(spec),
            kind: Kind::Mark { tag_name, attrs },
        })
    }

    /// A widget decoration, which draws a widget at its position
    /// (`Decoration.widget`).
    pub fn widget(spec: WidgetSpec) -> Arc<Decoration> {
        let mut side = spec.side.clamp(-10000, 10000);
        let block = spec.block;
        side += if block && !spec.inline_order {
            if side > 0 {
                BLOCK_AFTER
            } else {
                BLOCK_BEFORE
            }
        } else if side > 0 {
            INLINE_AFTER
        } else {
            INLINE_BEFORE
        };
        let widget = Some(spec.widget.clone());
        Decoration::point(
            DecorationSpec::Widget(spec),
            side,
            side,
            block,
            widget,
            false,
        )
    }

    /// A replace decoration, which replaces its range with a widget or
    /// hides it (`Decoration.replace`).
    pub fn replace(spec: ReplaceSpec) -> Arc<Decoration> {
        let block = spec.block;
        let (start_side, end_side) = if spec.is_block_gap {
            (GAP_START, GAP_END)
        } else {
            let (start, end) = get_inclusive(
                spec.inclusive,
                spec.inclusive_start,
                spec.inclusive_end,
                block,
            );
            let start_side = match (start, block) {
                (true, true) => BLOCK_INC_START,
                (true, false) => INLINE_INC_START,
                (false, _) => NON_INC_START,
            } - 1;
            let end_side = match (end, block) {
                (true, true) => BLOCK_INC_END,
                (true, false) => INLINE_INC_END,
                (false, _) => NON_INC_END,
            } + 1;
            (start_side, end_side)
        };
        let widget = spec.widget.clone();
        Decoration::point(
            DecorationSpec::Replace(spec),
            start_side,
            end_side,
            block,
            widget,
            true,
        )
    }

    fn point(
        spec: DecorationSpec,
        start_side: i32,
        end_side: i32,
        block: bool,
        widget: Option<Widget>,
        is_replace: bool,
    ) -> Arc<Decoration> {
        Arc::new(Decoration {
            start_side,
            end_side,
            widget,
            spec,
            kind: Kind::Point { block, is_replace },
        })
    }

    /// A line decoration, which adds attributes to the line starting at its
    /// position (`Decoration.line`).
    pub fn line(spec: LineSpec) -> Arc<Decoration> {
        Arc::new(Decoration {
            start_side: LINE,
            end_side: LINE,
            widget: None,
            spec: DecorationSpec::Line(spec),
            kind: Kind::Line,
        })
    }

    /// A decoration set of `ranges`, which must be sorted unless `sort` is
    /// true (`Decoration.set`).
    pub fn set(ranges: Vec<Range<Decoration>>, sort: bool) -> DecorationSet {
        RangeSet::of(ranges, sort)
    }

    /// The empty decoration set (`Decoration.none`).
    pub fn none() -> DecorationSet {
        NONE.clone()
    }

    /// A range with this decoration (`range`). Marks may not be empty;
    /// line and widget decorations must be empty; a replacement's range
    /// may be empty only when its sides allow it.
    pub fn range(
        self: &Arc<Self>,
        from: usize,
        to: usize,
    ) -> Result<Range<Decoration>, RangeError> {
        let error = match self.kind {
            Kind::Mark { .. } if from >= to => Some("Mark decorations may not be empty"),
            Kind::Line if to != from => Some("Line decoration ranges must be zero-length"),
            Kind::Point {
                is_replace: true, ..
            } if from > to || (from == to && self.start_side > 0 && self.end_side <= 0) => {
                Some("Invalid range for replacement decoration")
            }
            Kind::Point {
                is_replace: false, ..
            } if to != from => Some("Widget decorations can only have zero-length ranges"),
            _ => None,
        };
        match error {
            Some(message) => Err(RangeError(message.to_string())),
            None => Ok(Range::new(from, to, self.clone())),
        }
    }

    /// A range at `pos`: `range(pos)` upstream.
    pub fn at(self: &Arc<Self>, pos: usize) -> Result<Range<Decoration>, RangeError> {
        self.range(pos, pos)
    }

    /// The spec the decoration was made from (`spec`).
    pub fn spec(&self) -> &DecorationSpec {
        &self.spec
    }

    /// The widget, for widget and replace decorations (`widget`).
    pub fn widget_type(&self) -> Option<&Widget> {
        self.widget.as_ref()
    }

    /// Which kind of decoration this is.
    pub fn kind(&self) -> DecorationKind<'_> {
        match &self.kind {
            Kind::Mark { tag_name, attrs } => DecorationKind::Mark { tag_name, attrs },
            Kind::Line => DecorationKind::Line,
            Kind::Point { block, is_replace } => DecorationKind::Point {
                block: *block,
                is_replace: *is_replace,
            },
        }
    }

    /// Whether this is a block-level widget or replace decoration
    /// (`block`).
    pub fn is_block(&self) -> bool {
        matches!(self.kind, Kind::Point { block: true, .. })
    }

    /// For a block decoration, which kind of block it is (`type` on
    /// `PointDecoration`).
    pub fn block_type(&self) -> BlockType {
        if self.start_side != self.end_side {
            BlockType::WidgetRange
        } else if self.start_side <= 0 {
            BlockType::WidgetBefore
        } else {
            BlockType::WidgetAfter
        }
    }

    /// Whether the decoration can change the height of the content
    /// (`heightRelevant`).
    pub fn height_relevant(&self) -> bool {
        match self.kind {
            Kind::Point { block, .. } => {
                block
                    || self.widget.as_ref().is_some_and(|widget| {
                        widget.estimated_height() >= 5.0 || widget.line_breaks() > 0
                    })
            }
            _ => false,
        }
    }

    /// Whether the widget has an estimated height (`hasHeight`).
    pub fn has_height(&self) -> bool {
        self.widget
            .as_ref()
            .is_some_and(|widget| widget.estimated_height() > -1.0)
    }
}

impl RangeValue for Decoration {
    fn range_eq(&self, other: &Self) -> bool {
        match (&self.kind, &other.kind) {
            (
                Kind::Mark { tag_name, attrs },
                Kind::Mark {
                    tag_name: other_tag,
                    attrs: other_attrs,
                },
            ) => {
                std::ptr::eq(self, other)
                    || tag_name == other_tag && attrs_eq(Some(attrs), Some(other_attrs), None)
            }
            (Kind::Line, Kind::Line) => match (&self.spec, &other.spec) {
                (DecorationSpec::Line(a), DecorationSpec::Line(b)) => {
                    a.class == b.class
                        && attrs_eq(a.attributes.as_ref(), b.attributes.as_ref(), None)
                }
                _ => false,
            },
            (
                Kind::Point { block, .. },
                Kind::Point {
                    block: other_block, ..
                },
            ) => {
                widgets_eq(self.widget.as_ref(), other.widget.as_ref())
                    && block == other_block
                    && self.start_side == other.start_side
                    && self.end_side == other.end_side
            }
            _ => false,
        }
    }

    fn start_side(&self) -> i32 {
        self.start_side
    }

    fn end_side(&self) -> i32 {
        self.end_side
    }

    fn map_mode(&self) -> MapMode {
        match self.kind {
            Kind::Mark { .. } => MapMode::TrackDel,
            Kind::Line => MapMode::TrackBefore,
            Kind::Point { block: false, .. } => MapMode::TrackDel,
            Kind::Point { .. } if self.start_side <= 0 => MapMode::TrackBefore,
            Kind::Point { .. } => MapMode::TrackAfter,
        }
    }

    fn point(&self) -> bool {
        !matches!(self.kind, Kind::Mark { .. })
    }
}

impl fmt::Debug for Decoration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match &self.kind {
            Kind::Mark { .. } => "mark",
            Kind::Line => "line",
            Kind::Point {
                is_replace: true, ..
            } => "replace",
            Kind::Point { .. } => "widget",
        };
        write!(
            f,
            "Decoration::{kind}({}, {}{})",
            self.start_side,
            self.end_side,
            if self.is_block() { ", block" } else { "" }
        )
    }
}

/// Add `from..to` to a flat list of ranges, merging it into the last range
/// when it starts within `margin` of its end (`addRange`).
pub fn add_range(from: usize, to: usize, ranges: &mut Vec<usize>, margin: usize) {
    match ranges.last_mut() {
        Some(last) if *last + margin >= from => *last = (*last).max(to),
        _ => ranges.extend([from, to]),
    }
}

/// How a block wrapper is built (`BlockWrapperSpec`).
#[derive(Clone, Debug)]
pub struct BlockWrapperSpec {
    /// The wrapping element's tag.
    pub tag_name: String,
    /// Attributes for the wrapping element.
    pub attributes: Option<Attrs>,
    /// Precedence among overlapping wrappers from one source, 0 to 100:
    /// lower ranks nest inside higher ones. Defaults to 50.
    pub rank: Option<f64>,
}

/// An element that wraps lines or other block wrappers at the top level
/// of the document (`BlockWrapper`). It wraps each line or block widget
/// that starts in its range, including at `from` but not at `to`.
#[derive(Debug)]
pub struct BlockWrapper {
    tag_name: String,
    attributes: Attrs,
    rank: f64,
}

impl BlockWrapper {
    /// A block wrapper (`BlockWrapper.create`).
    pub fn create(spec: BlockWrapperSpec) -> Arc<BlockWrapper> {
        Arc::new(BlockWrapper {
            tag_name: spec.tag_name,
            attributes: spec.attributes.unwrap_or_default(),
            rank: spec.rank.map_or(50.0, |rank| rank.clamp(0.0, 100.0)),
        })
    }

    /// A range set of block wrappers (`BlockWrapper.set`).
    pub fn set(ranges: Vec<Range<BlockWrapper>>, sort: bool) -> RangeSet<BlockWrapper> {
        RangeSet::of(ranges, sort)
    }

    /// The wrapping element's tag.
    pub fn tag_name(&self) -> &str {
        &self.tag_name
    }

    /// The wrapping element's attributes.
    pub fn attributes(&self) -> &Attrs {
        &self.attributes
    }

    /// The wrapper's rank.
    pub fn rank(&self) -> f64 {
        self.rank
    }
}

impl RangeValue for BlockWrapper {
    fn range_eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
            || self.tag_name == other.tag_name
                && attrs_eq(Some(&self.attributes), Some(&other.attributes), None)
    }

    fn start_side(&self) -> i32 {
        -1
    }

    fn end_side(&self) -> i32 {
        -1
    }
}
