//! A port of `viewstate.ts`: the view's measurements and the viewport,
//! the part of the document the view draws, with the line gaps that hide
//! the parts of very long lines that are out of view.

use std::sync::Arc;

use div::{Document, Element};
use editor_state::{ChangeDesc, ChangeSet, EditorState, RangeSet, SpanIterator, Text, Touch};

use crate::bidi::Direction;
use crate::decoration::{BlockType, Decoration, DecorationSet, ReplaceSpec, Widget, WidgetType};
use crate::docview::DocView;
use crate::dom::{
    bounding_rect, get_scale, is_scrolled_to_bottom, scrollable_parent_y, set_style, Rect,
    WindowSize,
};
use crate::extension::{
    update_flag, ChangedRange, ScrollStrategy, ScrollTarget, ViewUpdate, CONTENT_ATTRIBUTES,
    DECORATIONS, NATIVE_SELECTION_HIDDEN, OUTER_DECORATIONS,
};
use crate::heightmap::{
    clear_height_change_flag, height_change_flag, height_relevant_deco_changes, BlockInfo,
    BlockKind, HeightMap, HeightOracle, MeasuredHeights, QueryType,
};

// `VP`.
const VP_MARGIN: f64 = 1000.0;
// `coveredBy` requires at least this many extra pixels to be covered.
const VP_MIN_COVER_MARGIN: f64 = 10.0;
const VP_MAX_COVER_MARGIN: f64 = VP_MARGIN / 4.0;
// Beyond this size, DOM layout starts to break down in browsers because
// they use fixed-precision numbers to store dimensions.
const VP_MAX_DOM_HEIGHT: f64 = 7e6;
const VP_MAX_HORIZ_GAP: f64 = 2e6;

// `LG`.
const LG_MARGIN: usize = 2000;
const LG_MARGIN_WRAP: usize = 10000;
const LG_SELECTION_MARGIN: usize = 10;

/// The part of the document the view draws (`Viewport`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    /// The start of the viewport.
    pub from: usize,
    /// Its end.
    pub to: usize,
}

impl Viewport {
    fn new(from: usize, to: usize) -> Viewport {
        Viewport { from, to }
    }
}

/// A placeholder that hides part of an overlong line in the viewport
/// (`LineGap`), as a kludge to keep the editor responsive when a
/// ridiculously long line is loaded into it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LineGap {
    from: usize,
    to: usize,
    size: f64,
    display_size: f64,
}

impl LineGap {
    fn same(a: &[LineGap], b: &[LineGap]) -> bool {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(a, b)| a.from == b.from && a.to == b.to && a.size == b.size)
    }

    fn draw(&self, scale_x: f64, scale_y: f64, wrapping: bool) -> editor_state::Range<Decoration> {
        Decoration::replace(ReplaceSpec {
            widget: Some(Widget::new(LineGapWidget {
                size: self.display_size * if wrapping { scale_y } else { scale_x },
                vertical: wrapping,
            })),
            ..Default::default()
        })
        .range(self.from, self.to)
        .expect("a line gap is a valid range")
    }
}

struct LineGapWidget {
    size: f64,
    vertical: bool,
}

impl WidgetType for LineGapWidget {
    fn eq(&self, other: &Self) -> bool {
        other.size == self.size && other.vertical == self.vertical
    }

    fn to_dom(&self, document: &Document) -> Result<Element, div::Error> {
        let elt = crate::dom::create_element(document, "div")?;
        if self.vertical {
            set_style(&elt, "height", &format!("{}px", js(self.size)))?;
        } else {
            set_style(&elt, "width", &format!("{}px", js(self.size)))?;
            set_style(&elt, "height", "2px")?;
            set_style(&elt, "display", "inline-block")?;
        }
        Ok(elt)
    }

    fn estimated_height(&self) -> f64 {
        if self.vertical {
            self.size
        } else {
            -1.0
        }
    }
}

/// A number as JavaScript prints it in `n + "px"`.
pub(crate) fn js(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e21 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// Converts between height map heights and DOM heights (`YScaler`), when
/// the document is taller than the DOM can handle.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Scaler {
    /// No scaling (`IdScaler`).
    Id,
    /// `BigScaler`.
    Big(BigScaler),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BigScaler {
    scale: f64,
    viewports: Vec<ScaledViewport>,
}

#[derive(Clone, Debug, PartialEq)]
struct ScaledViewport {
    from: usize,
    to: usize,
    top: f64,
    bottom: f64,
    dom_top: f64,
    dom_bottom: f64,
}

impl Scaler {
    fn scale(&self) -> f64 {
        match self {
            Scaler::Id => 1.0,
            Scaler::Big(big) => big.scale,
        }
    }

    pub(crate) fn to_dom(&self, n: f64) -> f64 {
        let Scaler::Big(big) = self else {
            return n;
        };
        let (mut base, mut dom_base) = (0.0, 0.0);
        for vp in &big.viewports {
            if n < vp.top {
                return dom_base + (n - base) * big.scale;
            }
            if n <= vp.bottom {
                return vp.dom_top + (n - vp.top);
            }
            base = vp.bottom;
            dom_base = vp.dom_bottom;
        }
        dom_base + (n - base) * big.scale
    }

    // Upstream's `fromDOM`, a conversion by this scaler.
    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn from_dom(&self, n: f64) -> f64 {
        let Scaler::Big(big) = self else {
            return n;
        };
        let (mut base, mut dom_base) = (0.0, 0.0);
        for vp in &big.viewports {
            if n < vp.dom_top {
                return base + (n - dom_base) / big.scale;
            }
            if n <= vp.dom_bottom {
                return vp.top + (n - vp.dom_top);
            }
            base = vp.bottom;
            dom_base = vp.dom_bottom;
        }
        base + (n - dom_base) / big.scale
    }

    // `eq`: the identity scaler equals itself only; big scalers compare
    // their scale and viewports.
    fn same(&self, other: &Scaler) -> bool {
        match (self, other) {
            (Scaler::Id, Scaler::Id) => true,
            (Scaler::Big(a), Scaler::Big(b)) => {
                a.scale == b.scale
                    && a.viewports.len() == b.viewports.len()
                    && a.viewports
                        .iter()
                        .zip(&b.viewports)
                        .all(|(a, b)| a.from == b.from && a.to == b.to)
            }
            _ => false,
        }
    }

    fn big(oracle: &HeightOracle, height_map: &HeightMap, viewports: &[Viewport]) -> Scaler {
        let mut vp_height = 0.0;
        let mut vps: Vec<ScaledViewport> = viewports
            .iter()
            .map(|&Viewport { from, to }| {
                let top = height_map
                    .line_at(from as f64, QueryType::ByPos, oracle, 0.0, 0)
                    .top;
                let bottom = height_map
                    .line_at(to as f64, QueryType::ByPos, oracle, 0.0, 0)
                    .bottom();
                vp_height += bottom - top;
                ScaledViewport {
                    from,
                    to,
                    top,
                    bottom,
                    dom_top: 0.0,
                    dom_bottom: 0.0,
                }
            })
            .collect();
        if height_map.height() - vp_height < 10.0 {
            return Scaler::Id;
        }
        let scale = (VP_MAX_DOM_HEIGHT - vp_height) / (height_map.height() - vp_height);
        let (mut base, mut dom_base) = (0.0, 0.0);
        for obj in &mut vps {
            obj.dom_top = dom_base + (obj.top - base) * scale;
            obj.dom_bottom = obj.dom_top + (obj.bottom - obj.top);
            dom_base = obj.dom_bottom;
            base = obj.bottom;
        }
        Scaler::Big(BigScaler {
            scale,
            viewports: vps,
        })
    }

    fn scale_block(&self, block: BlockInfo) -> BlockInfo {
        if self.scale() == 1.0 {
            return block;
        }
        block.scaled(&|n| self.to_dom(n))
    }
}

/// What the view's measurement reads and writes: the view's DOM and its
/// content's drawing.
pub(crate) struct MeasureContext<'a> {
    pub editor_dom: &'a Element,
    pub scroll_dom: &'a Element,
    pub content_dom: &'a Element,
    pub body: &'a Element,
    pub window: WindowSize,
    pub doc_view: &'a mut DocView,
}

/// The view's measurements and viewport (`ViewState`).
pub(crate) struct ViewState {
    // These are contentDOM-local coordinates.
    pub pixel_viewport: Rect,
    pub in_view: bool,

    /// Padding above the document, scaled.
    pub padding_top: f64,
    /// Padding below the document, scaled.
    pub padding_bottom: f64,
    /// `contentDOM.getBoundingClientRect().width`.
    pub content_dom_width: f64,
    /// `contentDOM.getBoundingClientRect().height`.
    pub content_dom_height: f64,
    /// `scrollDOM.clientHeight`, unscaled.
    pub editor_height: f64,
    /// `scrollDOM.clientWidth`, unscaled.
    pub editor_width: f64,
    /// The CSS-transformation scale of the editor (transformed size /
    /// concrete size).
    pub scale_x: f64,
    pub scale_y: f64,

    /// The nearest vertically scrollable parent; `None` is the window.
    pub scroll_parent: Option<Element>,
    /// Last seen vertical offset of the element at the top of the scroll
    /// container, or the top of the window if there is no wrapping
    /// scroller.
    pub scroll_offset: f64,
    pub scrolled_to_bottom: bool,
    /// The vertical position (document-relative) to anchor the scroll
    /// position to; -1 anchors to the end of the document.
    pub scroll_anchor_pos: i64,
    /// The height at the anchor position, set by the DOM update phase; -1
    /// means no height is available.
    pub scroll_anchor_height: f64,

    pub height_oracle: HeightOracle,
    pub height_map: HeightMap,
    pub scaler: Scaler,

    pub scroll_target: Option<ScrollTarget>,
    // Briefly set when printing, to disable viewport limiting.
    pub printing: bool,
    /// Set when the content was redrawn, so that the next measure stage
    /// reads the DOM layout; `Refresh` also refreshes the estimates.
    pub must_measure_content: MustMeasure,

    pub state_deco: Vec<DecorationSet>,
    pub viewport_lines: Vec<BlockInfo>,
    pub default_text_direction: Direction,

    /// The main viewport, for the visible part of the document.
    pub viewport: Viewport,
    /// When the main selection starts or ends outside the main viewport,
    /// extra single-line viewports for those points, so that the DOM
    /// selection does not fall in a gap.
    pub viewports: Vec<Viewport>,
    pub visible_ranges: Vec<(usize, usize)>,
    pub line_gaps: Vec<LineGap>,
    pub line_gap_deco: DecorationSet,

    pub must_enforce_cursor_assoc: bool,

    pub state: EditorState,
}

/// `mustMeasureContent`: `false`, `true` or `"refresh"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MustMeasure {
    No,
    Yes,
    Refresh,
}

impl ViewState {
    /// `new ViewState(view, state)`. `window_width` is the window's
    /// `innerWidth`.
    pub fn new(state: EditorState, window_width: f64) -> ViewState {
        let guess_wrapping = state
            .facet(&CONTENT_ATTRIBUTES)
            .iter()
            .any(|attrs| attrs.get("class") == Some("cm-lineWrapping"));
        let mut height_oracle = HeightOracle::new(guess_wrapping);
        let state_deco = static_deco(&state);
        let height_map = HeightMap::empty().apply_changes(
            &state_deco,
            &Text::empty(),
            height_oracle.set_doc(state.doc().clone()),
            &[ChangedRange::new(0, 0, 0, state.doc().len())],
        );
        let mut vs = ViewState {
            pixel_viewport: Rect {
                left: 0.0,
                right: window_width,
                top: 0.0,
                bottom: 0.0,
            },
            in_view: true,
            padding_top: 0.0,
            padding_bottom: 0.0,
            content_dom_width: 0.0,
            content_dom_height: 0.0,
            editor_height: 0.0,
            editor_width: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            scroll_parent: None,
            scroll_offset: 0.0,
            scrolled_to_bottom: false,
            scroll_anchor_pos: 0,
            scroll_anchor_height: -1.0,
            height_oracle,
            height_map,
            scaler: Scaler::Id,
            scroll_target: None,
            printing: false,
            must_measure_content: MustMeasure::Yes,
            state_deco,
            viewport_lines: Vec::new(),
            default_text_direction: Direction::Ltr,
            viewport: Viewport::new(0, 0),
            viewports: Vec::new(),
            visible_ranges: Vec::new(),
            line_gaps: Vec::new(),
            line_gap_deco: Decoration::none(),
            must_enforce_cursor_assoc: false,
            state,
        };
        for _ in 0..2 {
            vs.viewport = vs.get_viewport(0.0, None);
            if vs.update_for_viewport() == 0 {
                break;
            }
        }
        vs.update_viewport_lines();
        vs.line_gaps = vs.ensure_line_gaps(&[]);
        vs.line_gap_deco = Decoration::set(
            vs.line_gaps
                .iter()
                .map(|gap| gap.draw(vs.scale_x, vs.scale_y, false))
                .collect(),
            false,
        );
        // `this.scrollParent = view.scrollDOM` is set by the view, which
        // owns the element.
        vs.compute_visible_ranges(None);
        vs
    }

    pub fn update_for_viewport(&mut self) -> u32 {
        let mut viewports = vec![self.viewport];
        let main = self.state.selection().main();
        for i in 0..=1 {
            let pos = if i == 1 { main.head() } else { main.anchor() };
            if !viewports.iter().any(|vp| pos >= vp.from && pos <= vp.to) {
                let block = self.line_block_at(pos);
                viewports.push(Viewport::new(block.from, block.to()));
            }
        }
        viewports.sort_by_key(|vp| vp.from);
        self.viewports = viewports;
        self.update_scaler()
    }

    pub fn update_scaler(&mut self) -> u32 {
        let scaler = std::mem::replace(&mut self.scaler, Scaler::Id);
        self.scaler = if self.height_map.height() <= VP_MAX_DOM_HEIGHT {
            Scaler::Id
        } else {
            Scaler::big(&self.height_oracle, &self.height_map, &self.viewports)
        };
        if scaler.same(&self.scaler) {
            0
        } else {
            update_flag::HEIGHT
        }
    }

    pub fn update_viewport_lines(&mut self) {
        let mut lines = Vec::new();
        let scaler = self.scaler.clone();
        self.height_oracle.set_doc(self.state.doc().clone());
        self.height_map.for_each_line(
            self.viewport.from,
            self.viewport.to,
            &self.height_oracle,
            0.0,
            0,
            &mut |block| lines.push(scaler.scale_block(block)),
        );
        self.viewport_lines = lines;
    }

    /// Update for a view update (`update`). `line_wrapping` is the view's.
    pub fn update(
        &mut self,
        update: &mut ViewUpdate,
        scroll_target: Option<ScrollTarget>,
        line_wrapping: bool,
    ) {
        self.state = update.state.clone();
        let prev_deco = std::mem::take(&mut self.state_deco);
        self.state_deco = static_deco(&self.state);
        let content_changes = &update.changed_ranges;

        let height_changes = ChangedRange::extend_with_ranges(
            content_changes,
            &height_relevant_deco_changes(&prev_deco, &self.state_deco, update.changes.desc()),
        );
        let prev_height = self.height_map.height();
        let scroll_anchor = if self.scrolled_to_bottom {
            None
        } else {
            Some(self.scroll_anchor_at(self.scroll_offset))
        };
        clear_height_change_flag();
        let map = std::mem::replace(&mut self.height_map, HeightMap::empty());
        self.height_map = map.apply_changes(
            &self.state_deco,
            update.start_state.doc(),
            self.height_oracle.set_doc(self.state.doc().clone()),
            &height_changes,
        );
        if self.height_map.height() != prev_height || height_change_flag() {
            update.flags |= update_flag::HEIGHT;
        }
        match scroll_anchor {
            Some(anchor) => {
                self.scroll_anchor_pos = update.changes.map_pos(anchor.from, -1) as i64;
                self.scroll_anchor_height = anchor.top;
            }
            None => {
                self.scroll_anchor_pos = -1;
                self.scroll_anchor_height = prev_height;
            }
        }

        let mut viewport = if height_changes.is_empty() {
            self.viewport
        } else {
            self.map_viewport(self.viewport, update.changes.desc())
        };
        if scroll_target
            .as_ref()
            .is_some_and(|t| t.range.head() < viewport.from || t.range.head() > viewport.to)
            || !self.viewport_is_appropriate(viewport, 0.0)
        {
            viewport = self.get_viewport(0.0, scroll_target.as_ref());
        }

        let viewport_change = viewport != self.viewport;
        self.viewport = viewport;
        update.flags |= self.update_for_viewport();
        if viewport_change || !update.changes.is_empty() || update.flags & update_flag::HEIGHT != 0
        {
            self.update_viewport_lines();
        }

        if !self.line_gaps.is_empty() || self.viewport.to - self.viewport.from > LG_MARGIN << 1 {
            let mapped = self.map_line_gaps(&self.line_gaps, &update.changes);
            let gaps = self.ensure_line_gaps(&mapped);
            self.update_line_gaps(gaps);
        }
        update.flags |= self.compute_visible_ranges(Some(update.changes.desc()));

        if scroll_target.is_some() {
            self.scroll_target = scroll_target;
        }

        if !self.must_enforce_cursor_assoc
            && (update.selection_set() || update.focus_changed())
            && line_wrapping
        {
            let main = update.state.selection().main();
            if main.is_empty()
                && main.assoc() != 0
                && !*update.state.facet(&NATIVE_SELECTION_HIDDEN)
            {
                self.must_enforce_cursor_assoc = true;
            }
        }
    }

    /// Read the DOM layout (`measure`). Returns the update flags.
    pub fn measure(&mut self, ctx: MeasureContext<'_>) -> Result<u32, div::Error> {
        let dom = ctx.content_dom;
        let style = dom.computed_style()?;
        let white_space = white_space_name(*style.white_space());
        self.default_text_direction = match *style.direction() {
            div::typed_style::Direction::Rtl => Direction::Rtl,
            _ => Direction::Ltr,
        };

        let mut refresh = self.height_oracle.must_refresh_for_wrapping(white_space)
            || self.must_measure_content == MustMeasure::Refresh;
        let dom_rect = bounding_rect(dom)?;
        let mut measure_content = refresh
            || self.must_measure_content != MustMeasure::No
            || self.content_dom_height != dom_rect.height();
        self.content_dom_height = dom_rect.height();
        self.must_measure_content = MustMeasure::No;
        let mut result = 0;
        let mut bias = 0.0;

        if dom_rect.width() != 0.0 && dom_rect.height() != 0.0 {
            let (scale_x, scale_y) = get_scale(dom, &dom_rect);
            if scale_x > 0.005 && (self.scale_x - scale_x).abs() > 0.005
                || scale_y > 0.005 && (self.scale_y - scale_y).abs() > 0.005
            {
                self.scale_x = scale_x;
                self.scale_y = scale_y;
                result |= update_flag::GEOMETRY;
                refresh = true;
                measure_content = true;
            }
        }

        // Vertical padding (`parseInt` of the computed value).
        let padding_top = px(style.padding_top()).trunc() * self.scale_y;
        let padding_bottom = px(style.padding_bottom()).trunc() * self.scale_y;
        if self.padding_top != padding_top || self.padding_bottom != padding_bottom {
            self.padding_top = padding_top;
            self.padding_bottom = padding_bottom;
            result |= update_flag::GEOMETRY | update_flag::HEIGHT;
        }
        let client_width = ctx.scroll_dom.client_width()? as f64;
        if self.editor_width != client_width {
            if self.height_oracle.line_wrapping {
                measure_content = true;
            }
            self.editor_width = client_width;
            result |= update_flag::GEOMETRY;
        }
        let scroll_parent = scrollable_parent_y(dom, ctx.body)?;
        let same_parent = match (&scroll_parent, &self.scroll_parent) {
            (Some(a), Some(b)) => a.is_same_node(b)?,
            (None, None) => true,
            _ => false,
        };
        if !same_parent {
            self.scroll_parent = scroll_parent;
            self.scroll_anchor_height = -1.0;
            self.scroll_offset = 0.0;
        }
        let scroll_offset = self.get_scroll_offset(ctx.scroll_dom, ctx.content_dom)?;
        if self.scroll_offset != scroll_offset {
            self.scroll_anchor_height = -1.0;
            self.scroll_offset = scroll_offset;
        }
        self.scrolled_to_bottom = is_scrolled_to_bottom(self.scroll_parent.as_ref())?;

        // Pixel viewport.
        let pixel_viewport = if self.printing {
            full_pixel_range(dom, self.padding_top)?
        } else {
            visible_pixel_range(dom, ctx.body, ctx.window, self.padding_top)?
        };
        let d_top = pixel_viewport.top - self.pixel_viewport.top;
        let d_bottom = pixel_viewport.bottom - self.pixel_viewport.bottom;
        self.pixel_viewport = pixel_viewport;
        let in_view = self.pixel_viewport.bottom > self.pixel_viewport.top
            && self.pixel_viewport.right > self.pixel_viewport.left;
        if in_view != self.in_view {
            self.in_view = in_view;
            if in_view {
                measure_content = true;
            }
        }
        if !self.in_view && self.scroll_target.is_none() && !in_window(ctx.editor_dom, ctx.window)?
        {
            return Ok(0);
        }

        let content_width = dom_rect.width();
        let editor_height = ctx.scroll_dom.client_height()? as f64;
        if self.content_dom_width != content_width || self.editor_height != editor_height {
            self.content_dom_width = dom_rect.width();
            self.editor_height = editor_height;
            result |= update_flag::GEOMETRY;
        }

        if measure_content {
            let content_client_width = dom.client_width()? as f64;
            let line_heights = ctx.doc_view.measure_visible_line_heights(
                self.viewport,
                content_client_width,
                client_width,
                self.default_text_direction == Direction::Ltr,
            )?;
            if self.height_oracle.must_refresh_for_heights(&line_heights) {
                refresh = true;
            }
            if refresh
                || self.height_oracle.line_wrapping
                    && (content_width - self.content_dom_width).abs()
                        > self.height_oracle.char_width
            {
                let (line_height, char_width, text_height) = ctx.doc_view.measure_text_size()?;
                refresh = line_height > 0.0
                    && self.height_oracle.refresh(
                        white_space,
                        line_height,
                        char_width,
                        text_height,
                        (content_width / char_width).max(5.0),
                        &line_heights,
                    );
                if refresh {
                    ctx.doc_view.min_width = 0.0;
                    result |= update_flag::GEOMETRY;
                }
            }

            if d_top > 0.0 && d_bottom > 0.0 {
                bias = d_top.max(d_bottom);
            } else if d_top < 0.0 && d_bottom < 0.0 {
                bias = d_top.min(d_bottom);
            }

            clear_height_change_flag();
            for vp in self.viewports.clone() {
                let heights = if vp.from == self.viewport.from {
                    line_heights.clone()
                } else {
                    let content_client_width = dom.client_width()? as f64;
                    ctx.doc_view.measure_visible_line_heights(
                        vp,
                        content_client_width,
                        client_width,
                        self.default_text_direction == Direction::Ltr,
                    )?
                };
                let map = if refresh {
                    HeightMap::empty().apply_changes(
                        &self.state_deco,
                        &Text::empty(),
                        &mut self.height_oracle,
                        &[ChangedRange::new(0, 0, 0, self.state.doc().len())],
                    )
                } else {
                    std::mem::replace(&mut self.height_map, HeightMap::empty())
                };
                let mut measured = MeasuredHeights::new(vp.from, heights);
                self.height_map =
                    map.update_height(&self.height_oracle, 0, refresh, Some(&mut measured));
            }
            if height_change_flag() {
                result |= update_flag::HEIGHT;
            }
        }

        let viewport_change = !self.viewport_is_appropriate(self.viewport, bias)
            || self.scroll_target.as_ref().is_some_and(|t| {
                t.range.head() < self.viewport.from || t.range.head() > self.viewport.to
            });
        if viewport_change {
            if result & update_flag::HEIGHT != 0 {
                result |= self.update_scaler();
            }
            let target = self.scroll_target.clone();
            self.viewport = self.get_viewport(bias, target.as_ref());
            result |= self.update_for_viewport();
        }
        if result & update_flag::HEIGHT != 0 || viewport_change {
            self.update_viewport_lines();
        }

        if !self.line_gaps.is_empty() || self.viewport.to - self.viewport.from > LG_MARGIN << 1 {
            let current = if refresh {
                Vec::new()
            } else {
                self.line_gaps.clone()
            };
            let gaps = self.ensure_line_gaps(&current);
            self.update_line_gaps(gaps);
        }
        result |= self.compute_visible_ranges(None);

        if self.must_enforce_cursor_assoc {
            // `view.docView.enforceCursorAssoc()` moves the DOM selection,
            // which comes with the selection in milestone M7.
            self.must_enforce_cursor_assoc = false;
        }

        Ok(result)
    }

    pub fn visible_top(&self) -> f64 {
        self.scaler.from_dom(self.pixel_viewport.top)
    }

    pub fn visible_bottom(&self) -> f64 {
        self.scaler.from_dom(self.pixel_viewport.bottom)
    }

    pub fn get_viewport(&self, bias: f64, scroll_target: Option<&ScrollTarget>) -> Viewport {
        // This divides VP.Margin between the top and the bottom,
        // depending on the bias (the change in viewport position since the
        // last update). It holds a number between 0 and 1.
        let margin_top = 0.5 - (-0.5f64).max(0.5f64.min(bias / VP_MARGIN / 2.0));
        let map = &self.height_map;
        let oracle = &self.height_oracle;
        let (visible_top, visible_bottom) = (self.visible_top(), self.visible_bottom());
        let mut viewport = Viewport::new(
            map.line_at(
                visible_top - margin_top * VP_MARGIN,
                QueryType::ByHeight,
                oracle,
                0.0,
                0,
            )
            .from,
            map.line_at(
                visible_bottom + (1.0 - margin_top) * VP_MARGIN,
                QueryType::ByHeight,
                oracle,
                0.0,
                0,
            )
            .to(),
        );
        // If a scroll target is given, make sure the viewport includes it.
        if let Some(target) = scroll_target {
            let head = target.range.head();
            if head < viewport.from || head > viewport.to {
                let view_height = self
                    .editor_height
                    .min(self.pixel_viewport.bottom - self.pixel_viewport.top);
                let block = map.line_at(head as f64, QueryType::ByPos, oracle, 0.0, 0);
                let top_pos = if target.y == ScrollStrategy::Center {
                    (block.top + block.bottom()) / 2.0 - view_height / 2.0
                } else if target.y == ScrollStrategy::Start
                    || target.y == ScrollStrategy::Nearest && head < viewport.from
                {
                    block.top
                } else {
                    block.bottom() - view_height
                };
                viewport = Viewport::new(
                    map.line_at(
                        top_pos - VP_MARGIN / 2.0,
                        QueryType::ByHeight,
                        oracle,
                        0.0,
                        0,
                    )
                    .from,
                    map.line_at(
                        top_pos + view_height + VP_MARGIN / 2.0,
                        QueryType::ByHeight,
                        oracle,
                        0.0,
                        0,
                    )
                    .to(),
                );
            }
        }
        viewport
    }

    pub fn map_viewport(&self, viewport: Viewport, changes: &ChangeDesc) -> Viewport {
        let from = changes.map_pos(viewport.from, -1);
        let to = changes.map_pos(viewport.to, 1);
        Viewport::new(
            self.height_map
                .line_at(from as f64, QueryType::ByPos, &self.height_oracle, 0.0, 0)
                .from,
            self.height_map
                .line_at(to as f64, QueryType::ByPos, &self.height_oracle, 0.0, 0)
                .to(),
        )
    }

    /// Whether `viewport` covers the visible part of the document and not
    /// too much beyond it (`viewportIsAppropriate`).
    pub fn viewport_is_appropriate(&self, Viewport { from, to }: Viewport, bias: f64) -> bool {
        if !self.in_view {
            return true;
        }
        let top = self
            .height_map
            .line_at(from as f64, QueryType::ByPos, &self.height_oracle, 0.0, 0)
            .top;
        let bottom = self
            .height_map
            .line_at(to as f64, QueryType::ByPos, &self.height_oracle, 0.0, 0)
            .bottom();
        let (visible_top, visible_bottom) = (self.visible_top(), self.visible_bottom());
        (from == 0
            || top <= visible_top - VP_MIN_COVER_MARGIN.max((-bias).min(VP_MAX_COVER_MARGIN)))
            && (to == self.state.doc().len()
                || bottom
                    >= visible_bottom + VP_MIN_COVER_MARGIN.max(bias.min(VP_MAX_COVER_MARGIN)))
            && (top > visible_top - 2.0 * VP_MARGIN && bottom < visible_bottom + 2.0 * VP_MARGIN)
    }

    fn map_line_gaps(&self, gaps: &[LineGap], changes: &ChangeSet) -> Vec<LineGap> {
        if gaps.is_empty() || changes.is_empty() {
            return gaps.to_vec();
        }
        gaps.iter()
            .filter(|gap| matches!(changes.touches_range(gap.from, gap.to), Touch::No))
            .map(|gap| LineGap {
                from: changes.map_pos(gap.from, -1),
                to: changes.map_pos(gap.to, -1),
                size: gap.size,
                display_size: gap.display_size,
            })
            .collect()
    }

    /// The positions in the viewport where the start or end of a line is
    /// hidden, reusing current gaps where they still fit
    /// (`ensureLineGaps`). Uses crude character counting for the positions
    /// and sizes, since actual DOM coordinates are not always available,
    /// and relies on generous margins to hide the artifacts.
    ///
    /// Upstream, when the measure phase calls this for wrapped lines, it
    /// snaps a gap's end to the start of a visual line with
    /// `moveToLineBoundary`, which comes with the view's geometry in
    /// milestone M7.
    pub fn ensure_line_gaps(&self, current: &[LineGap]) -> Vec<LineGap> {
        let wrapping = self.height_oracle.line_wrapping;
        let margin = if wrapping { LG_MARGIN_WRAP } else { LG_MARGIN };
        let double_margin = margin << 1;
        // The non-wrapping logic won't work at all in predominantly
        // right-to-left text.
        if self.default_text_direction != Direction::Ltr && !wrapping {
            return Vec::new();
        }
        let mut gaps = Vec::new();
        let sel = self.state.selection().main();
        let mut avoid = vec![sel.from()];
        if !sel.is_empty() {
            avoid.push(sel.to());
        }

        let check_line = |line: &BlockInfo, gaps: &mut Vec<LineGap>| {
            if line.length < double_margin
                || !matches!(line.kind(), BlockKind::Type(BlockType::Text))
            {
                return;
            }
            let structure = line_structure(line.from, line.to(), &self.state_deco);
            if structure.total < double_margin {
                return;
            }
            let target = self.scroll_target.as_ref().map(|t| t.range.head());
            let (view_from, view_to);
            if wrapping {
                let margin_height = (margin as f64 / self.height_oracle.line_length)
                    * self.height_oracle.line_height;
                let (top, bot);
                if let Some(target) = target {
                    let target_frac = find_fraction(&structure, target);
                    let space_frac = ((self.visible_bottom() - self.visible_top()) / 2.0
                        + margin_height)
                        / line.height;
                    top = target_frac - space_frac;
                    bot = target_frac + space_frac;
                } else {
                    top = (self.visible_top() - line.top - margin_height) / line.height;
                    bot = (self.visible_bottom() - line.top + margin_height) / line.height;
                }
                view_from = find_position(&structure, top);
                view_to = find_position(&structure, bot);
            } else {
                let char_width = self.height_oracle.char_width;
                let total_width = structure.total as f64 * char_width;
                let margin_width = margin as f64 * char_width;
                let mut horiz_offset = 0.0;
                if total_width > VP_MAX_HORIZ_GAP {
                    for old in current {
                        if old.from >= line.from
                            && old.from < line.to()
                            && old.size != old.display_size
                            && old.from as f64 * char_width + horiz_offset
                                < self.pixel_viewport.left
                        {
                            horiz_offset = old.size - old.display_size;
                        }
                    }
                }
                let px_left = self.pixel_viewport.left + horiz_offset;
                let px_right = self.pixel_viewport.right + horiz_offset;
                let (left, right);
                if let Some(target) = target {
                    let target_frac = find_fraction(&structure, target);
                    let space_frac = ((px_right - px_left) / 2.0 + margin_width) / total_width;
                    left = target_frac - space_frac;
                    right = target_frac + space_frac;
                } else {
                    left = (px_left - margin_width) / total_width;
                    right = (px_right + margin_width) / total_width;
                }
                view_from = find_position(&structure, left);
                view_to = find_position(&structure, right);
            }
            if view_from > line.from {
                self.add_gap(
                    line.from, view_from, line, &structure, current, &avoid, gaps,
                );
            }
            if view_to < line.to() {
                self.add_gap(view_to, line.to(), line, &structure, current, &avoid, gaps);
            }
        };

        for line in &self.viewport_lines {
            match line.kind() {
                BlockKind::Blocks(blocks) => {
                    for block in blocks {
                        check_line(block, &mut gaps);
                    }
                }
                BlockKind::Type(_) => check_line(line, &mut gaps),
            }
        }
        gaps
    }

    #[allow(clippy::too_many_arguments)]
    fn add_gap(
        &self,
        from: usize,
        to: usize,
        line: &BlockInfo,
        structure: &LineStructure,
        current: &[LineGap],
        avoid: &[usize],
        gaps: &mut Vec<LineGap>,
    ) {
        let wrapping = self.height_oracle.line_wrapping;
        let half_margin = if wrapping { LG_MARGIN_WRAP } else { LG_MARGIN } >> 1;
        if to < from + half_margin {
            return;
        }
        for &pos in avoid {
            if pos > from && pos < to {
                self.add_gap(
                    from,
                    pos.saturating_sub(LG_SELECTION_MARGIN),
                    line,
                    structure,
                    current,
                    avoid,
                    gaps,
                );
                self.add_gap(
                    pos + LG_SELECTION_MARGIN,
                    to,
                    line,
                    structure,
                    current,
                    avoid,
                    gaps,
                );
                return;
            }
        }
        let found = current.iter().find(|gap| {
            gap.from >= line.from
                && gap.to <= line.to()
                && gap.from.abs_diff(from) < half_margin
                && gap.to.abs_diff(to) < half_margin
                && !avoid.iter().any(|&pos| gap.from < pos && gap.to > pos)
        });
        let gap = match found {
            Some(gap) => gap.clone(),
            None => {
                let size = self.gap_size(line, from, to, structure);
                let display_size = if wrapping || size < VP_MAX_HORIZ_GAP {
                    size
                } else {
                    VP_MAX_HORIZ_GAP
                };
                LineGap {
                    from,
                    to,
                    size,
                    display_size,
                }
            }
        };
        gaps.push(gap);
    }

    fn gap_size(&self, line: &BlockInfo, from: usize, to: usize, structure: &LineStructure) -> f64 {
        let fraction = find_fraction(structure, to) - find_fraction(structure, from);
        if self.height_oracle.line_wrapping {
            line.height * fraction
        } else {
            structure.total as f64 * self.height_oracle.char_width * fraction
        }
    }

    fn update_line_gaps(&mut self, gaps: Vec<LineGap>) {
        if !LineGap::same(&gaps, &self.line_gaps) {
            let wrapping = self.height_oracle.line_wrapping;
            self.line_gap_deco = Decoration::set(
                gaps.iter()
                    .map(|gap| gap.draw(self.scale_x, self.scale_y, wrapping))
                    .collect(),
                false,
            );
            self.line_gaps = gaps;
        }
    }

    pub fn compute_visible_ranges(&mut self, changes: Option<&ChangeDesc>) -> u32 {
        let mut deco = self.state_deco.clone();
        if !self.line_gaps.is_empty() {
            deco.push(self.line_gap_deco.clone());
        }
        let mut collect = SpanRanges(Vec::new());
        RangeSet::spans(
            &deco,
            self.viewport.from,
            self.viewport.to,
            &mut collect,
            20,
        );
        let ranges = collect.0;
        let mut changed = 0;
        if ranges.len() != self.visible_ranges.len() {
            changed = update_flag::VIEWPORT_MOVED | update_flag::VIEWPORT;
        } else {
            for (old, new) in self.visible_ranges.iter().zip(&ranges) {
                if changed & update_flag::VIEWPORT_MOVED != 0 {
                    break;
                }
                if old != new {
                    changed |= update_flag::VIEWPORT;
                    if !changes.is_some_and(|changes| {
                        changes.map_pos(old.0, -1) == new.0 && changes.map_pos(old.1, 1) == new.1
                    }) {
                        changed |= update_flag::VIEWPORT_MOVED;
                    }
                }
            }
        }
        self.visible_ranges = ranges;
        changed
    }

    pub fn line_block_at(&self, pos: usize) -> BlockInfo {
        if pos >= self.viewport.from && pos <= self.viewport.to {
            if let Some(block) = self
                .viewport_lines
                .iter()
                .find(|b| b.from <= pos && b.to() >= pos)
            {
                return block.clone();
            }
        }
        self.scaler.scale_block(self.height_map.line_at(
            pos as f64,
            QueryType::ByPos,
            &self.height_oracle,
            0.0,
            0,
        ))
    }

    pub fn line_block_at_height(&self, height: f64) -> BlockInfo {
        if let (Some(first), Some(last)) = (self.viewport_lines.first(), self.viewport_lines.last())
        {
            if height >= first.top && height <= last.bottom() {
                if let Some(line) = self
                    .viewport_lines
                    .iter()
                    .find(|l| l.top <= height && l.bottom() >= height)
                {
                    return line.clone();
                }
            }
        }
        self.scaler.scale_block(self.height_map.line_at(
            self.scaler.from_dom(height),
            QueryType::ByHeight,
            &self.height_oracle,
            0.0,
            0,
        ))
    }

    pub fn get_scroll_offset(
        &self,
        scroll_dom: &Element,
        content_dom: &Element,
    ) -> Result<f64, div::Error> {
        Ok(match &self.scroll_parent {
            Some(parent) if parent.is_same_node(scroll_dom)? => parent.scroll_top()? * self.scale_y,
            Some(parent) => bounding_rect(parent)?.top - bounding_rect(content_dom)?.top,
            None => 0.0 - bounding_rect(content_dom)?.top,
        })
    }

    pub fn scroll_anchor_at(&self, scroll_offset: f64) -> BlockInfo {
        let block = self.line_block_at_height(scroll_offset + 12.0 - self.padding_top);
        match self.viewport_lines.first() {
            Some(first)
                if !(block.from >= self.viewport.from || first.top - scroll_offset > 200.0) =>
            {
                first.clone()
            }
            _ => block,
        }
    }

    pub fn element_at_height(&self, height: f64) -> BlockInfo {
        self.scaler.scale_block(self.height_map.block_at(
            self.scaler.from_dom(height),
            &self.height_oracle,
            0.0,
            0,
        ))
    }

    pub fn doc_height(&self) -> f64 {
        self.scaler.to_dom(self.height_map.height())
    }

    pub fn content_height(&self) -> f64 {
        self.doc_height() + self.padding_top + self.padding_bottom
    }
}

// Collects the spans of `RangeSet.spans`.
struct SpanRanges(Vec<(usize, usize)>);

impl SpanIterator<Decoration> for SpanRanges {
    fn span(&mut self, from: usize, to: usize, _active: &[Arc<Decoration>], _open: usize) {
        self.0.push((from, to));
    }

    fn point(
        &mut self,
        _from: usize,
        _to: usize,
        _value: &Arc<Decoration>,
        _active: &[Arc<Decoration>],
        _open: usize,
        _index: usize,
    ) {
    }
}

struct LineStructure {
    total: usize,
    ranges: Vec<(usize, usize)>,
}

fn line_structure(from: usize, to: usize, state_deco: &[DecorationSet]) -> LineStructure {
    struct Points {
        pos: usize,
        total: usize,
        ranges: Vec<(usize, usize)>,
    }
    impl SpanIterator<Decoration> for Points {
        fn span(&mut self, _: usize, _: usize, _: &[Arc<Decoration>], _: usize) {}
        fn point(
            &mut self,
            from: usize,
            to: usize,
            _: &Arc<Decoration>,
            _: &[Arc<Decoration>],
            _: usize,
            _: usize,
        ) {
            if from > self.pos {
                self.ranges.push((self.pos, from));
                self.total += from - self.pos;
            }
            self.pos = to;
        }
    }
    let mut points = Points {
        pos: from,
        total: 0,
        ranges: Vec::new(),
    };
    // Only collapsed ranges of a significant size matter.
    RangeSet::spans(state_deco, from, to, &mut points, 20);
    if points.pos < to {
        points.ranges.push((points.pos, to));
        points.total += to - points.pos;
    }
    LineStructure {
        total: points.total,
        ranges: points.ranges,
    }
}

fn find_position(structure: &LineStructure, ratio: f64) -> usize {
    if ratio <= 0.0 {
        return structure.ranges[0].0;
    }
    if ratio >= 1.0 {
        return structure.ranges[structure.ranges.len() - 1].1;
    }
    let mut dist = (structure.total as f64 * ratio).floor() as usize;
    for &(from, to) in &structure.ranges {
        let size = to - from;
        if dist <= size {
            return from + dist;
        }
        dist -= size;
    }
    structure.ranges[structure.ranges.len() - 1].1
}

fn find_fraction(structure: &LineStructure, pos: usize) -> f64 {
    let mut counted = 0i64;
    for &(from, to) in &structure.ranges {
        if pos <= to {
            counted += pos as i64 - from as i64;
            break;
        }
        counted += (to - from) as i64;
    }
    counted as f64 / structure.total as f64
}

/// The decorations the height map and the viewport consider
/// (`staticDeco`): the plain decoration sets, with the outer ones joined.
pub(crate) fn static_deco(state: &EditorState) -> Vec<DecorationSet> {
    let mut deco: Vec<DecorationSet> = state.facet(&DECORATIONS).clone();
    let outer = state.facet(&OUTER_DECORATIONS);
    if !outer.is_empty() {
        deco.push(RangeSet::join(outer));
    }
    deco
}

fn px(length: &div::typed_style::Length) -> f64 {
    // div-gap: 22. `getComputedStyle` reports used lengths in pixels;
    // Div's computed style keeps percentages, which this reads as 0.
    if length.is_fixed() {
        length.value() as f64
    } else {
        0.0
    }
}

fn white_space_name(white_space: div::typed_style::WhiteSpace) -> &'static str {
    use div::typed_style::WhiteSpace;
    match white_space {
        WhiteSpace::Normal => "normal",
        WhiteSpace::Nowrap => "nowrap",
        WhiteSpace::Pre => "pre",
        WhiteSpace::PreWrap => "pre-wrap",
        WhiteSpace::PreLine => "pre-line",
        WhiteSpace::BreakSpaces => "break-spaces",
    }
}

fn is_overflow_visible(overflow: div::typed_style::Overflow) -> bool {
    overflow == div::typed_style::Overflow::Visible
}

// The visible part of `dom`, in its own coordinates below its top padding
// (`visiblePixelRange`).
fn visible_pixel_range(
    dom: &Element,
    body: &Element,
    window: WindowSize,
    padding_top: f64,
) -> Result<Rect, div::Error> {
    let rect = bounding_rect(dom)?;
    let mut left = rect.left.max(0.0);
    let mut right = window.width.min(rect.right);
    let mut top = rect.top.max(0.0);
    let mut bottom = window.height.min(rect.bottom);
    let dom_parent = dom.parent()?;
    let mut parent = dom_parent.clone();
    while let Some(node) = parent {
        if node.is_same_node(body)? {
            break;
        }
        let style = node.computed_style()?;
        let overflow_visible =
            is_overflow_visible(*style.overflow_x()) && is_overflow_visible(*style.overflow_y());
        if (node.scroll_height()? > node.client_height()?
            || node.scroll_width()? > node.client_width()?)
            && !overflow_visible
        {
            let parent_rect = bounding_rect(&node)?;
            left = left.max(parent_rect.left);
            right = right.min(parent_rect.right);
            top = top.max(parent_rect.top);
            let is_dom_parent = match &dom_parent {
                Some(p) => p.is_same_node(&node)?,
                None => false,
            };
            bottom = if is_dom_parent { window.height } else { bottom }.min(parent_rect.bottom);
        }
        // `offsetParent` for absolute and fixed elements (div-gap 27):
        // Div does not report it, so this walks the parent chain.
        parent = node.parent()?;
    }
    Ok(Rect {
        left: left - rect.left,
        right: left.max(right) - rect.left,
        top: top - (rect.top + padding_top),
        bottom: top.max(bottom) - (rect.top + padding_top),
    })
}

fn in_window(elt: &Element, window: WindowSize) -> Result<bool, div::Error> {
    let rect = bounding_rect(elt)?;
    Ok(rect.left < window.width
        && rect.right > 0.0
        && rect.top < window.height
        && rect.bottom > 0.0)
}

fn full_pixel_range(dom: &Element, padding_top: f64) -> Result<Rect, div::Error> {
    let rect = bounding_rect(dom)?;
    Ok(Rect {
        left: 0.0,
        right: rect.right - rect.left,
        top: padding_top,
        bottom: rect.bottom - (rect.top + padding_top),
    })
}
