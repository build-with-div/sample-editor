//! A port of `gutter.ts`: gutters beside the content, the line number
//! gutter and the active line's gutter highlight.
//!
//! Gutters are drawn by a view plugin, as upstream. Their DOM event
//! handlers (`domEventHandlers`) come with input handling in milestone M7.

use std::any::Any;
use std::sync::{Arc, LazyLock};

use div::Element;
use editor_state::{
    Callback, Dep, EditorState, Extension, Facet, FacetConfig, MapMode, Range, RangeCursor,
    RangeSet, RangeValue,
};

use crate::decoration::{BlockType, Widget};
use crate::docview::{escape, open_tag_with};
use crate::dom::{
    add_css_text, bounding_rect, create_element, create_text, set_attribute, set_style,
    sync_children, DomNode,
};
use crate::extension::{Margins, ViewUpdate, SCROLL_MARGINS};
use crate::heightmap::{BlockInfo, BlockKind};
use crate::plugin::{PluginSpec, PluginValue, ViewPlugin};
use crate::viewstate::{js, Viewport};
use crate::{Direction, EditorView};

/// A gutter marker (`GutterMarker`): a bit of information attached to a
/// line in a gutter. Shared as a [`GutterMarker`].
pub trait GutterMarkerType: Any + Send + Sync {
    /// Compare with another marker of the same type (`eq`). The default
    /// returns `false`.
    fn eq(&self, _other: &Self) -> bool
    where
        Self: Sized,
    {
        false
    }

    /// Whether the marker draws a DOM node, upstream's test for a
    /// `toDOM` method. Defaults to `false`.
    fn has_dom(&self) -> bool {
        false
    }

    /// Draw the marker's DOM node (`toDOM`), for a marker whose
    /// [`has_dom`](Self::has_dom) is `true`.
    fn to_dom(&self, _view: &EditorView) -> Result<DomNode, div::Error> {
        Err(div::Error::InvalidArgument(
            "the gutter marker draws no DOM",
        ))
    }

    /// CSS classes to add to the gutter element holding the marker
    /// (`elementClass`). Defaults to none.
    fn element_class(&self) -> &str {
        ""
    }

    /// Called when the marker's DOM node was removed from a gutter
    /// (`destroy`).
    fn destroy(&self, _dom: &DomNode) {}
}

trait DynMarker: Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn compare(&self, other: &dyn DynMarker) -> bool;
    fn has_dom(&self) -> bool;
    fn to_dom(&self, view: &EditorView) -> Result<DomNode, div::Error>;
    fn element_class(&self) -> &str;
    fn destroy(&self, dom: &DomNode);
}

impl<M: GutterMarkerType> DynMarker for M {
    fn as_any(&self) -> &dyn Any {
        self
    }

    // `this.constructor == other.constructor && this.eq(other)`.
    fn compare(&self, other: &dyn DynMarker) -> bool {
        other
            .as_any()
            .downcast_ref::<M>()
            .is_some_and(|other| GutterMarkerType::eq(self, other))
    }

    fn has_dom(&self) -> bool {
        GutterMarkerType::has_dom(self)
    }

    fn to_dom(&self, view: &EditorView) -> Result<DomNode, div::Error> {
        GutterMarkerType::to_dom(self, view)
    }

    fn element_class(&self) -> &str {
        GutterMarkerType::element_class(self)
    }

    fn destroy(&self, dom: &DomNode) {
        GutterMarkerType::destroy(self, dom)
    }
}

/// A shared [`GutterMarkerType`] instance (`GutterMarker`). As a range
/// value, it is a point that maps before insertions at its position, with
/// sides -1.
#[derive(Clone)]
pub struct GutterMarker(Arc<dyn DynMarker>);

impl GutterMarker {
    /// Share `marker`.
    pub fn new<M: GutterMarkerType>(marker: M) -> GutterMarker {
        GutterMarker(Arc::new(marker))
    }

    /// Whether this is the same marker, or an equal one of the same type
    /// (`compare`).
    pub fn compare(&self, other: &GutterMarker) -> bool {
        self.ptr_eq(other) || self.0.compare(&*other.0)
    }

    /// Whether both share one instance (upstream's `==` on markers).
    pub fn ptr_eq(&self, other: &GutterMarker) -> bool {
        std::ptr::addr_eq(Arc::as_ptr(&self.0), Arc::as_ptr(&other.0))
    }

    /// The marker at `pos` (`range`).
    pub fn range(&self, pos: usize) -> Range<GutterMarker> {
        Range::new(pos, pos, Arc::new(self.clone()))
    }

    /// The marker, if it is an `M`.
    pub fn downcast_ref<M: GutterMarkerType>(&self) -> Option<&M> {
        self.0.as_any().downcast_ref()
    }

    /// See [`GutterMarkerType::element_class`].
    pub fn element_class(&self) -> &str {
        self.0.element_class()
    }

    /// See [`GutterMarkerType::has_dom`].
    pub fn has_dom(&self) -> bool {
        self.0.has_dom()
    }
}

impl RangeValue for GutterMarker {
    fn range_eq(&self, other: &Self) -> bool {
        self.compare(other)
    }

    fn start_side(&self) -> i32 {
        -1
    }

    fn end_side(&self) -> i32 {
        -1
    }

    fn map_mode(&self) -> MapMode {
        MapMode::TrackBefore
    }

    fn point(&self) -> bool {
        true
    }
}

/// The marker sets of a gutter (`markers`).
pub type MarkerSource = Callback<dyn Fn(&EditorView) -> Vec<RangeSet<GutterMarker>> + Send + Sync>;
/// A marker for every line, given the line's other markers (`lineMarker`).
pub type LineMarker = Callback<
    dyn Fn(&EditorView, &BlockInfo, &[GutterMarker]) -> Option<GutterMarker> + Send + Sync,
>;
/// A marker for a block widget (`widgetMarker`).
pub type WidgetMarker =
    Callback<dyn Fn(&EditorView, &Widget, &BlockInfo) -> Option<GutterMarker> + Send + Sync>;
/// Whether an update may change the line markers (`lineMarkerChange`).
pub type LineMarkerChange = Callback<dyn Fn(&ViewUpdate) -> bool + Send + Sync>;
/// The hidden spacer's marker (`initialSpacer`).
pub type InitialSpacer = Callback<dyn Fn(&EditorView) -> GutterMarker + Send + Sync>;
/// The spacer's marker after an update (`updateSpacer`).
pub type UpdateSpacer =
    Callback<dyn Fn(&GutterMarker, &ViewUpdate, &EditorView) -> GutterMarker + Send + Sync>;

/// Which side of the content a gutter is on (`side`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GutterSide {
    /// Before the content: to the left in a left-to-right layout.
    #[default]
    Before,
    /// After the content.
    After,
}

/// A gutter's configuration (`GutterConfig`). Options left `None` take
/// upstream's defaults: no markers, no line or widget markers, no
/// spacer.
#[derive(Clone, Default)]
pub struct GutterConfig {
    /// An extra class for the `cm-gutter` element (`class`).
    pub class: String,
    /// Whether to draw gutter elements without markers
    /// (`renderEmptyElements`).
    pub render_empty_elements: bool,
    /// The markers to show (`markers`).
    pub markers: Option<MarkerSource>,
    /// A marker to add to every line (`lineMarker`).
    pub line_marker: Option<LineMarker>,
    /// Markers for block widgets (`widgetMarker`).
    pub widget_marker: Option<WidgetMarker>,
    /// Whether an update may change the line or widget markers
    /// (`lineMarkerChange`).
    pub line_marker_change: Option<LineMarkerChange>,
    /// A hidden spacer element that gives the gutter its base width
    /// (`initialSpacer`).
    pub initial_spacer: Option<InitialSpacer>,
    /// Update the spacer after an update (`updateSpacer`).
    pub update_spacer: Option<UpdateSpacer>,
    /// The side of the content the gutter is on (`side`).
    pub side: GutterSide,
}

// A gutter configuration in the `activeGutters` facet, compared by
// identity as upstream's configuration objects are.
#[derive(Clone)]
struct ActiveGutter(Arc<GutterConfig>);

impl PartialEq for ActiveGutter {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

static ACTIVE_GUTTERS: LazyLock<Facet<ActiveGutter>> = LazyLock::new(Facet::define);

static UNFIX_GUTTERS: LazyLock<Facet<bool, bool>> = LazyLock::new(|| {
    Facet::define_with(FacetConfig::new(|values: &[bool]| {
        values.iter().any(|x| *x)
    }))
});

/// Classes for all gutter elements of a line (`gutterLineClass`). The
/// markers should only have an element class, no DOM.
pub static GUTTER_LINE_CLASS: LazyLock<Facet<RangeSet<GutterMarker>>> =
    LazyLock::new(Facet::define);

/// Classes for the gutter elements beside a block widget
/// (`gutterWidgetClass`).
pub static GUTTER_WIDGET_CLASS: LazyLock<Facet<WidgetMarker>> = LazyLock::new(Facet::define);

/// A gutter (`gutter`). Gutters appear in the order of their extensions'
/// precedence.
pub fn gutter(config: GutterConfig) -> Extension {
    Extension::from(vec![
        gutters(GuttersConfig::default()),
        ACTIVE_GUTTERS.of(ActiveGutter(Arc::new(config))),
    ])
}

/// The configuration of the gutters' plugin (`gutters`).
#[derive(Clone, Copy, Debug)]
pub struct GuttersConfig {
    /// Whether the gutters stay in place when the content scrolls
    /// horizontally, with `position: sticky` (`fixed`). Defaults to
    /// `true`.
    pub fixed: bool,
}

impl Default for GuttersConfig {
    fn default() -> Self {
        GuttersConfig { fixed: true }
    }
}

/// The plugin that draws the gutters (`gutters`), added by any gutter.
pub fn gutters(config: GuttersConfig) -> Extension {
    let mut result = vec![GUTTER_VIEW.extension()];
    if !config.fixed {
        result.push(UNFIX_GUTTERS.of(true));
    }
    Extension::from(result)
}

static GUTTER_VIEW: LazyLock<ViewPlugin<GutterView>> = LazyLock::new(|| {
    ViewPlugin::define(
        GutterView::new,
        PluginSpec {
            provide: Some(Box::new(|plugin: &ViewPlugin<GutterView>| {
                let plugin = plugin.clone();
                SCROLL_MARGINS.of(Callback(Arc::new(move |view: &EditorView| {
                    let value = view.plugin(&plugin)?;
                    if value.gutters.is_empty() || !value.fixed {
                        return None;
                    }
                    let before = offset_width(&value.dom) * view.scale_x();
                    let after = value
                        .dom_after
                        .as_ref()
                        .map_or(0.0, |dom| offset_width(dom) * view.scale_x());
                    Some(if view.text_direction() == Direction::Ltr {
                        Margins {
                            left: Some(before),
                            right: Some(after),
                            ..Margins::default()
                        }
                    } else {
                        Margins {
                            right: Some(before),
                            left: Some(after),
                            ..Margins::default()
                        }
                    })
                })))
            })),
        },
    )
});

// div-gap: 27 `offsetWidth`. Div has no offset sizes; the bounding
// rectangle's width is the same without transforms, and the view takes
// its scale as 1 (`getScale`).
fn offset_width(dom: &Element) -> f64 {
    bounding_rect(dom).map_or(0.0, |rect| rect.width())
}

// `style.minHeight` for the gutters: the content's height.
fn min_height(view: &EditorView) -> String {
    format!("{}px", js(view.content_height() / view.scale_y()))
}

/// The gutters' plugin value (`gutterView`).
pub(crate) struct GutterView {
    gutters: Vec<SingleGutterView>,
    dom: Element,
    dom_after: Option<Element>,
    fixed: bool,
    prev_viewport: Viewport,
}

impl GutterView {
    fn new(view: &EditorView) -> Result<GutterView, div::Error> {
        let dom = create_element(view.document(), "div")?;
        set_attribute(&dom, "class", "cm-gutters cm-gutters-before")?;
        set_attribute(&dom, "aria-hidden", "true")?;
        set_style(&dom, "min-height", &min_height(view))?;
        let gutters = view
            .state()
            .facet(&ACTIVE_GUTTERS)
            .iter()
            .map(|config| SingleGutterView::new(view, config.clone()))
            .collect::<Result<Vec<_>, _>>()?;
        let fixed = !*view.state().facet(&UNFIX_GUTTERS);
        let mut this = GutterView {
            gutters,
            dom,
            dom_after: None,
            fixed,
            prev_viewport: view.viewport(),
        };
        for i in 0..this.gutters.len() {
            let gutter_dom = this.gutters[i].dom.clone();
            if this.gutters[i].config.0.side == GutterSide::After {
                this.dom_after(view)?.append_child(&gutter_dom)?;
            } else {
                this.dom.append_child(&gutter_dom)?;
            }
        }
        if this.fixed {
            set_style(&this.dom, "position", "sticky")?;
        }
        this.sync_gutters(view, false)?;
        view.scroll_dom()
            .insert_before(&this.dom, view.content_dom())?;
        Ok(this)
    }

    // `getDOMAfter`.
    fn dom_after(&mut self, view: &EditorView) -> Result<Element, div::Error> {
        if let Some(dom) = &self.dom_after {
            return Ok(dom.clone());
        }
        let dom = create_element(view.document(), "div")?;
        set_attribute(&dom, "class", "cm-gutters cm-gutters-after")?;
        set_attribute(&dom, "aria-hidden", "true")?;
        set_style(&dom, "min-height", &min_height(view))?;
        set_style(&dom, "position", if self.fixed { "sticky" } else { "" })?;
        view.scroll_dom().append_child(&dom)?;
        self.dom_after = Some(dom.clone());
        Ok(dom)
    }

    // `syncGutters`: draw the gutter elements for the viewport's lines.
    fn sync_gutters(&mut self, view: &EditorView, detach: bool) -> Result<(), div::Error> {
        let after = self.dom.next_sibling()?;
        if detach {
            self.dom.detach()?;
            if let Some(dom) = &self.dom_after {
                dom.detach()?;
            }
        }
        let viewport = view.viewport();
        let classes: &Vec<RangeSet<GutterMarker>> = view.state().facet(&GUTTER_LINE_CLASS);
        let mut line_classes = RangeSet::iter_sets(classes, viewport.from);
        let mut class_set: Vec<GutterMarker> = Vec::new();
        let top = -view.document_padding().0;
        let mut contexts: Vec<UpdateContext> = self
            .gutters
            .iter_mut()
            .map(|gutter| UpdateContext::new(gutter, viewport, top))
            .collect();
        for line in view.viewport_line_blocks() {
            if !class_set.is_empty() {
                class_set = Vec::new();
            }
            match line.kind() {
                BlockKind::Blocks(blocks) => {
                    let mut first = true;
                    for block in blocks {
                        if first && matches!(block.kind(), BlockKind::Type(BlockType::Text)) {
                            advance_cursor(&mut line_classes, &mut class_set, block.from);
                            for cx in &mut contexts {
                                cx.line(view, block, &class_set)?;
                            }
                            first = false;
                        } else if block.widget().is_some() {
                            for cx in &mut contexts {
                                cx.widget(view, block)?;
                            }
                        }
                    }
                }
                BlockKind::Type(BlockType::Text) => {
                    advance_cursor(&mut line_classes, &mut class_set, line.from);
                    for cx in &mut contexts {
                        cx.line(view, line, &class_set)?;
                    }
                }
                _ if line.widget().is_some() => {
                    for cx in &mut contexts {
                        cx.widget(view, line)?;
                    }
                }
                _ => {}
            }
        }
        for cx in contexts {
            cx.finish()?;
        }
        if detach {
            match &after {
                Some(after) => view.scroll_dom().insert_before(&self.dom, after)?,
                None => view.scroll_dom().append_child(&self.dom)?,
            }
            if let Some(dom) = &self.dom_after {
                view.scroll_dom().append_child(dom)?;
            }
        }
        Ok(())
    }

    // `updateGutters`: whether the gutter elements must be redrawn.
    fn update_gutters(
        &mut self,
        update: &ViewUpdate,
        view: &EditorView,
    ) -> Result<bool, div::Error> {
        let prev = update.start_state.facet(&ACTIVE_GUTTERS).clone();
        let cur = update.state.facet(&ACTIVE_GUTTERS).clone();
        let viewport = view.viewport();
        let prev_classes: &Vec<RangeSet<GutterMarker>> =
            update.start_state.facet(&GUTTER_LINE_CLASS);
        let classes: &Vec<RangeSet<GutterMarker>> = update.state.facet(&GUTTER_LINE_CLASS);
        let mut change = update.doc_changed()
            || update.height_changed()
            || update.viewport_changed()
            || !RangeSet::eq(prev_classes, classes, viewport.from, Some(viewport.to));
        if prev == cur {
            for gutter in &mut self.gutters {
                if gutter.update(update, view)? {
                    change = true;
                }
            }
        } else {
            change = true;
            let mut old: Vec<Option<SingleGutterView>> = std::mem::take(&mut self.gutters)
                .into_iter()
                .map(Some)
                .collect();
            let mut gutters = Vec::new();
            for config in &cur {
                match prev.iter().position(|p| p == config) {
                    Some(known) if old[known].is_some() => {
                        let mut gutter = old[known].take().expect("checked");
                        gutter.update(update, view)?;
                        gutters.push(gutter);
                    }
                    _ => gutters.push(SingleGutterView::new(view, config.clone())?),
                }
            }
            for mut gutter in old.into_iter().flatten() {
                gutter.dom.detach()?;
                gutter.destroy()?;
                gutter.dom.remove()?;
            }
            for gutter in &gutters {
                gutter.dom.detach()?;
            }
            for gutter in &gutters {
                if gutter.config.0.side == GutterSide::After {
                    self.dom_after(view)?.append_child(&gutter.dom)?;
                } else {
                    self.dom.append_child(&gutter.dom)?;
                }
            }
            self.gutters = gutters;
        }
        Ok(change)
    }

    // The gutters as `outerHTML`, before and after the content (see
    // `EditorView::outer_html`).
    fn html(&self) -> Result<(String, String), div::Error> {
        let mut before = String::new();
        open_tag_with(&self.dom, &["class", "aria-hidden", "style"], &mut before)?;
        let mut after = String::new();
        if let Some(dom) = &self.dom_after {
            open_tag_with(dom, &["class", "aria-hidden", "style"], &mut after)?;
        }
        for gutter in &self.gutters {
            let out = match gutter.config.0.side {
                GutterSide::Before => &mut before,
                GutterSide::After => &mut after,
            };
            gutter.html(out)?;
        }
        before.push_str("</div>");
        if self.dom_after.is_some() {
            after.push_str("</div>");
        }
        Ok((before, after))
    }
}

impl PluginValue for GutterView {
    fn update(&mut self, update: &ViewUpdate, view: &EditorView) -> Result<(), div::Error> {
        if self.update_gutters(update, view)? {
            // Detach during sync when the viewport changed significantly
            // (such as during scrolling), since for large updates that is
            // faster.
            let (a, b) = (self.prev_viewport, view.viewport());
            let overlap = a.to.min(b.to) as f64 - a.from.max(b.from) as f64;
            self.sync_gutters(view, overlap < (b.to - b.from) as f64 * 0.8)?;
        }
        if update.geometry_changed() {
            let min = min_height(view);
            set_style(&self.dom, "min-height", &min)?;
            if let Some(dom) = &self.dom_after {
                set_style(dom, "min-height", &min)?;
            }
        }
        // `facet(unfixGutters) != !fixed`.
        if *view.state().facet(&UNFIX_GUTTERS) == self.fixed {
            self.fixed = !self.fixed;
            let position = if self.fixed { "sticky" } else { "" };
            set_style(&self.dom, "position", position)?;
            if let Some(dom) = &self.dom_after {
                set_style(dom, "position", position)?;
            }
        }
        self.prev_viewport = view.viewport();
        Ok(())
    }

    fn destroy(&mut self) -> Result<(), div::Error> {
        for gutter in &mut self.gutters {
            gutter.destroy()?;
        }
        self.dom.remove()?;
        if let Some(dom) = &self.dom_after {
            dom.remove()?;
        }
        Ok(())
    }
}

/// The gutters' `outerHTML` before and after the content, empty without
/// gutters (see [`EditorView::outer_html`]).
///
/// div-gap: 23 attribute enumeration, 24 node traversal: written from
/// the gutter plugin's record of its elements and marker nodes.
pub(crate) fn gutters_html(view: &EditorView) -> Result<(String, String), div::Error> {
    match view.plugin(&GUTTER_VIEW) {
        Some(gutters) => gutters.html(),
        None => Ok((String::new(), String::new())),
    }
}

// `advanceCursor`: collect the markers at `pos`, skipping those before.
fn advance_cursor(
    cursor: &mut RangeCursor<GutterMarker>,
    collect: &mut Vec<GutterMarker>,
    pos: usize,
) {
    while let Some(value) = cursor.value() {
        if cursor.from() > pos {
            break;
        }
        if cursor.from() == pos {
            collect.push((**value).clone());
        }
        cursor.next();
    }
}

// `UpdateContext`: walks one gutter's elements while its lines are drawn.
struct UpdateContext<'a> {
    gutter: &'a mut SingleGutterView,
    cursor: RangeCursor<GutterMarker>,
    i: usize,
    height: f64,
}

impl<'a> UpdateContext<'a> {
    fn new(gutter: &'a mut SingleGutterView, viewport: Viewport, height: f64) -> Self {
        let cursor = RangeSet::iter_sets(&gutter.markers, viewport.from);
        UpdateContext {
            gutter,
            cursor,
            i: 0,
            height,
        }
    }

    fn add_element(
        &mut self,
        view: &EditorView,
        block: &BlockInfo,
        markers: Vec<GutterMarker>,
    ) -> Result<(), div::Error> {
        let above = (block.top - self.height) / view.scale_y();
        let height = block.height / view.scale_y();
        if self.i == self.gutter.elements.len() {
            let element = GutterElement::new(view, height, above, markers)?;
            self.gutter.dom.append_child(&element.dom)?;
            self.gutter.elements.push(element);
        } else {
            self.gutter.elements[self.i].update(view, height, above, markers)?;
        }
        self.height = block.bottom();
        self.i += 1;
        Ok(())
    }

    fn line(
        &mut self,
        view: &EditorView,
        line: &BlockInfo,
        extra_markers: &[GutterMarker],
    ) -> Result<(), div::Error> {
        let mut local_markers = Vec::new();
        advance_cursor(&mut self.cursor, &mut local_markers, line.from);
        local_markers.extend(extra_markers.iter().cloned());
        let config = self.gutter.config.0.clone();
        if let Some(line_marker) = &config.line_marker {
            if let Some(marker) = line_marker(view, line, &local_markers) {
                local_markers.insert(0, marker);
            }
        }
        if local_markers.is_empty() && !config.render_empty_elements {
            return Ok(());
        }
        self.add_element(view, line, local_markers)
    }

    fn widget(&mut self, view: &EditorView, block: &BlockInfo) -> Result<(), div::Error> {
        let widget = block.widget().expect("a widget block");
        let config = self.gutter.config.0.clone();
        let mut markers: Option<Vec<GutterMarker>> = config
            .widget_marker
            .as_ref()
            .and_then(|marker| marker(view, widget, block))
            .map(|marker| vec![marker]);
        for class in view.state().facet(&GUTTER_WIDGET_CLASS).iter() {
            if let Some(marker) = class(view, widget, block) {
                markers.get_or_insert_with(Vec::new).push(marker);
            }
        }
        match markers {
            Some(markers) => self.add_element(view, block, markers),
            None => Ok(()),
        }
    }

    fn finish(self) -> Result<(), div::Error> {
        while self.gutter.elements.len() > self.i {
            let mut last = self.gutter.elements.pop().expect("an element");
            last.dom.detach()?;
            last.destroy()?;
            last.dom.remove()?;
        }
        Ok(())
    }
}

// `SingleGutterView`: one gutter's element and its gutter elements.
struct SingleGutterView {
    dom: Element,
    elements: Vec<GutterElement>,
    markers: Vec<RangeSet<GutterMarker>>,
    spacer: Option<GutterElement>,
    config: ActiveGutter,
}

impl SingleGutterView {
    fn new(view: &EditorView, config: ActiveGutter) -> Result<SingleGutterView, div::Error> {
        let dom = create_element(view.document(), "div")?;
        let class = if config.0.class.is_empty() {
            "cm-gutter".to_string()
        } else {
            format!("cm-gutter {}", config.0.class)
        };
        set_attribute(&dom, "class", &class)?;
        let markers = markers_of(&config.0, view);
        let mut spacer = None;
        if let Some(initial_spacer) = &config.0.initial_spacer {
            let element = GutterElement::new(view, 0.0, 0.0, vec![initial_spacer(view)])?;
            dom.append_child(&element.dom)?;
            add_css_text(&element.dom, "visibility: hidden; pointer-events: none")?;
            spacer = Some(element);
        }
        Ok(SingleGutterView {
            dom,
            elements: Vec::new(),
            markers,
            spacer,
            config,
        })
    }

    fn update(&mut self, update: &ViewUpdate, view: &EditorView) -> Result<bool, div::Error> {
        let config = self.config.0.clone();
        let prev_markers = std::mem::replace(&mut self.markers, markers_of(&config, view));
        if let (Some(spacer), Some(update_spacer)) = (&mut self.spacer, &config.update_spacer) {
            let current = spacer.markers[0].clone();
            let updated = update_spacer(&current, update, view);
            if !updated.ptr_eq(&current) {
                spacer.update(view, 0.0, 0.0, vec![updated])?;
            }
        }
        let viewport = view.viewport();
        Ok(!RangeSet::eq(
            &self.markers,
            &prev_markers,
            viewport.from,
            Some(viewport.to),
        ) || config
            .line_marker_change
            .as_ref()
            .is_some_and(|change| change(update)))
    }

    fn destroy(&mut self) -> Result<(), div::Error> {
        for element in &mut self.elements {
            element.destroy()?;
        }
        Ok(())
    }

    fn html(&self, out: &mut String) -> Result<(), div::Error> {
        open_tag_with(&self.dom, &["class"], out)?;
        for element in self.spacer.iter().chain(&self.elements) {
            element.html(out)?;
        }
        out.push_str("</div>");
        Ok(())
    }
}

// `asArray(config.markers(view))`, with upstream's default of no markers.
fn markers_of(config: &GutterConfig, view: &EditorView) -> Vec<RangeSet<GutterMarker>> {
    config
        .markers
        .as_ref()
        .map_or_else(|| vec![RangeSet::empty()], |markers| markers(view))
}

// `GutterElement`: the element for one line or widget in a gutter.
struct GutterElement {
    dom: Element,
    height: f64,
    above: f64,
    markers: Vec<GutterMarker>,
    // div-gap: 24 node traversal over text nodes. The DOM nodes of the
    // markers that draw one, in order, which upstream reaches through
    // `firstChild` and `nextSibling`.
    nodes: Vec<DomNode>,
}

impl GutterElement {
    fn new(
        view: &EditorView,
        height: f64,
        above: f64,
        markers: Vec<GutterMarker>,
    ) -> Result<GutterElement, div::Error> {
        let dom = create_element(view.document(), "div")?;
        set_attribute(&dom, "class", "cm-gutterElement")?;
        let mut element = GutterElement {
            dom,
            height: -1.0,
            above: 0.0,
            markers: Vec::new(),
            nodes: Vec::new(),
        };
        element.update(view, height, above, markers)?;
        Ok(element)
    }

    fn update(
        &mut self,
        view: &EditorView,
        height: f64,
        above: f64,
        markers: Vec<GutterMarker>,
    ) -> Result<(), div::Error> {
        if self.height != height {
            self.height = height;
            set_style(&self.dom, "height", &format!("{}px", js(height)))?;
        }
        if self.above != above {
            self.above = above;
            let value = if above != 0.0 {
                format!("{}px", js(above))
            } else {
                String::new()
            };
            set_style(&self.dom, "margin-top", &value)?;
        }
        if !same_markers(&self.markers, &markers) {
            self.set_markers(Some(view), markers)?;
        }
        Ok(())
    }

    // `setMarkers`: keep the DOM of markers that stay, draw new ones and
    // remove the others, then set the element's classes.
    fn set_markers(
        &mut self,
        view: Option<&EditorView>,
        markers: Vec<GutterMarker>,
    ) -> Result<(), div::Error> {
        let mut class = String::from("cm-gutterElement");
        let old_nodes = std::mem::take(&mut self.nodes);
        let mut dom_pos = 0;
        let mut nodes = Vec::new();
        let (mut i_new, mut i_old) = (0, 0);
        loop {
            let mut skip_to = i_old;
            let marker = markers.get(i_new).cloned();
            let mut matched = false;
            if let Some(marker) = &marker {
                i_new += 1;
                let c = marker.element_class();
                if !c.is_empty() {
                    class.push(' ');
                    class.push_str(c);
                }
                for i in i_old..self.markers.len() {
                    if self.markers[i].compare(marker) {
                        skip_to = i;
                        matched = true;
                        break;
                    }
                }
            } else {
                skip_to = self.markers.len();
            }
            while i_old < skip_to {
                let next = &self.markers[i_old];
                i_old += 1;
                if next.has_dom() {
                    let node = &old_nodes[dom_pos];
                    next.0.destroy(node);
                    node.remove()?;
                    dom_pos += 1;
                }
            }
            let Some(marker) = marker else {
                break;
            };
            if marker.has_dom() {
                if matched {
                    nodes.push(old_nodes[dom_pos].clone());
                    dom_pos += 1;
                } else {
                    let view = view.expect("a view to draw the marker");
                    nodes.push(marker.0.to_dom(view)?);
                }
            }
            if matched {
                i_old += 1;
            }
        }
        sync_children(&self.dom, &nodes)?;
        set_attribute(&self.dom, "class", &class)?;
        self.nodes = nodes;
        self.markers = markers;
        Ok(())
    }

    fn destroy(&mut self) -> Result<(), div::Error> {
        self.set_markers(None, Vec::new())
    }

    fn html(&self, out: &mut String) -> Result<(), div::Error> {
        open_tag_with(&self.dom, &["class", "style"], out)?;
        for node in &self.nodes {
            match node {
                DomNode::Text(_) => escape(&node.text_node()?.data()?, false, out),
                // div-gap: 25 `nodeName`: a marker's element is written
                // as a `div` with its class and style.
                DomNode::Element(element) => {
                    open_tag_with(element, &["class", "style"], out)?;
                    out.push_str("</div>");
                }
            }
        }
        out.push_str("</div>");
        Ok(())
    }
}

fn same_markers(a: &[GutterMarker], b: &[GutterMarker]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.compare(b))
}

/// How the line number gutter formats a number (`formatNumber`).
pub type FormatNumber = Callback<dyn Fn(usize, &EditorState) -> String + Send + Sync>;

/// The configuration of the line number gutter (`LineNumberConfig`).
#[derive(Clone, Default, PartialEq)]
pub struct LineNumberConfig {
    /// How to display line numbers (`formatNumber`). Defaults to the
    /// number in decimal.
    pub format_number: Option<FormatNumber>,
}

static LINE_NUMBER_CONFIG: LazyLock<Facet<LineNumberConfig, LineNumberConfig>> =
    LazyLock::new(|| {
        // `combineConfig`: a field given twice with different values is a
        // conflict.
        Facet::define_with(FacetConfig::new(|values: &[LineNumberConfig]| {
            let mut format_number: Option<FormatNumber> = None;
            for value in values {
                if let Some(format) = &value.format_number {
                    match &format_number {
                        None => format_number = Some(format.clone()),
                        Some(previous) if previous == format => {}
                        Some(_) => panic!("Config merge conflict for field formatNumber"),
                    }
                }
            }
            LineNumberConfig { format_number }
        }))
    });

/// Markers for the line number gutter (`lineNumberMarkers`).
pub static LINE_NUMBER_MARKERS: LazyLock<Facet<RangeSet<GutterMarker>>> =
    LazyLock::new(Facet::define);

/// Markers in the line number gutter beside block widgets
/// (`lineNumberWidgetMarker`).
pub static LINE_NUMBER_WIDGET_MARKER: LazyLock<Facet<WidgetMarker>> = LazyLock::new(Facet::define);

// `NumberMarker`: a line number, drawn as a text node.
struct NumberMarker {
    number: String,
}

impl GutterMarkerType for NumberMarker {
    fn eq(&self, other: &Self) -> bool {
        self.number == other.number
    }

    fn has_dom(&self) -> bool {
        true
    }

    fn to_dom(&self, view: &EditorView) -> Result<DomNode, div::Error> {
        Ok(DomNode::Text(create_text(view.document(), &self.number)?))
    }
}

fn format_number(state: &EditorState, number: usize) -> String {
    match &state.facet(&LINE_NUMBER_CONFIG).format_number {
        Some(format) => format(number, state),
        None => number.to_string(),
    }
}

fn number_marker(state: &EditorState, number: usize) -> GutterMarker {
    GutterMarker::new(NumberMarker {
        number: format_number(state, number),
    })
}

// `maxLineNumber`: the widest number with as many digits as `lines`.
fn max_line_number(lines: usize) -> usize {
    let mut last = 9;
    while last < lines {
        last = last * 10 + 9;
    }
    last
}

static LINE_NUMBER_GUTTER: LazyLock<Extension> = LazyLock::new(|| {
    ACTIVE_GUTTERS.compute([Dep::from(&*LINE_NUMBER_CONFIG)], |_state| {
        ActiveGutter(Arc::new(GutterConfig {
            class: "cm-lineNumbers".to_string(),
            render_empty_elements: false,
            markers: Some(Callback(Arc::new(|view: &EditorView| {
                view.state().facet(&LINE_NUMBER_MARKERS).clone()
            }))),
            line_marker: Some(Callback(Arc::new(
                |view: &EditorView, line: &BlockInfo, others: &[GutterMarker]| {
                    if others.iter().any(GutterMarker::has_dom) {
                        return None;
                    }
                    let state = view.state();
                    Some(number_marker(state, state.doc().line_at(line.from).number))
                },
            ))),
            widget_marker: Some(Callback(Arc::new(
                |view: &EditorView, widget: &Widget, block: &BlockInfo| {
                    view.state()
                        .facet(&LINE_NUMBER_WIDGET_MARKER)
                        .iter()
                        .find_map(|marker| marker(view, widget, block))
                },
            ))),
            line_marker_change: Some(Callback(Arc::new(|update: &ViewUpdate| {
                update.start_state.facet(&LINE_NUMBER_CONFIG)
                    != update.state.facet(&LINE_NUMBER_CONFIG)
            }))),
            initial_spacer: Some(Callback(Arc::new(|view: &EditorView| {
                let state = view.state();
                number_marker(state, max_line_number(state.doc().lines()))
            }))),
            update_spacer: Some(Callback(Arc::new(
                |spacer: &GutterMarker, _update: &ViewUpdate, view: &EditorView| {
                    let state = view.state();
                    let max = format_number(state, max_line_number(state.doc().lines()));
                    let same = spacer
                        .downcast_ref::<NumberMarker>()
                        .is_some_and(|marker| marker.number == max);
                    if same {
                        spacer.clone()
                    } else {
                        GutterMarker::new(NumberMarker { number: max })
                    }
                },
            ))),
            side: GutterSide::Before,
        }))
    })
});

/// A line number gutter (`lineNumbers`).
pub fn line_numbers(config: LineNumberConfig) -> Extension {
    Extension::from(vec![
        LINE_NUMBER_CONFIG.of(config),
        gutters(GuttersConfig::default()),
        LINE_NUMBER_GUTTER.clone(),
    ])
}

// `activeLineGutterMarker`.
struct ActiveLineGutterMarker;

impl GutterMarkerType for ActiveLineGutterMarker {
    fn element_class(&self) -> &str {
        "cm-activeLineGutter"
    }
}

static ACTIVE_LINE_GUTTER_MARKER: LazyLock<GutterMarker> =
    LazyLock::new(|| GutterMarker::new(ActiveLineGutterMarker));

static ACTIVE_LINE_GUTTER_HIGHLIGHTER: LazyLock<Extension> = LazyLock::new(|| {
    GUTTER_LINE_CLASS.compute([Dep::SELECTION], |state| {
        let mut marks = Vec::new();
        let mut last = None;
        for range in state.selection().ranges() {
            let line_pos = state.doc().line_at(range.head()).from;
            if last.is_none_or(|last| line_pos > last) {
                last = Some(line_pos);
                marks.push(ACTIVE_LINE_GUTTER_MARKER.range(line_pos));
            }
        }
        RangeSet::of(marks, false)
    })
});

/// Add a `cm-activeLineGutter` class to the gutter elements on the
/// lines with a cursor or selection head (`highlightActiveLineGutter`).
pub fn highlight_active_line_gutter() -> Extension {
    ACTIVE_LINE_GUTTER_HIGHLIGHTER.clone()
}
