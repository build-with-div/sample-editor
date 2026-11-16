//! A port of `editorview.ts`: the editor view, its DOM (`.cm-editor`,
//! `.cm-scroller`, `.cm-content`), updates and the measure cycle that
//! reads the layout and moves the viewport.
//!
//! Input, focus, the DOM selection and the coordinate queries come with
//! milestone M7; this part draws the state, runs the view plugins (the
//! gutters among them), keeps the drawing up to date and keeps the
//! viewport on the visible part of the document.

use std::cell::{Ref, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use div::{Document, Element};
use editor_state::{
    facets, Callback, ChangeSet, EditorSelection, EditorState, Extension, Facet, Line, Prec,
    RangeSet, SelectionRange, StateEffect, StateEffectType, Touch, Transaction,
};

use crate::attributes::{combine_attrs, Attrs};
use crate::bidi::{compute_order, trivial_order, BidiSpan, Isolate};
use crate::buildtile::BuildError;
use crate::cursor::{self, by_group, skip_atoms, ByChar};
use crate::decoration::{BlockWrapper, DecorationSet};
use crate::docview::{open_tag_with, DocView};
use crate::dom::{
    bounding_rect, create_element, is_scrolled_to_bottom, scroll_rect_into_view, set_attribute,
    update_attrs, Rect, WindowSize,
};
use crate::extension::ATOMIC_RANGES;
use crate::extension::{get_isolated_ranges, BIDI_ISOLATED_RANGES, PER_LINE_TEXT_DIRECTION};
use crate::extension::{
    get_scroll_margins, ScrollStrategy, ScrollTarget, ViewUpdate, BLOCK_WRAPPERS,
    CONTENT_ATTRIBUTES, CURSOR_SCROLL_MARGIN, DECORATIONS, EDITABLE, EDITOR_ATTRIBUTES,
    OUTER_DECORATIONS, SCROLL_INTO_VIEW,
};
use crate::heightmap::BlockInfo;
use crate::input::{InputState, Shared};
use crate::plugin::{
    log_exception, DomEventHandlers, PluginInstance, PluginSource, PluginSpec, PluginValue,
    ViewPlugin, VIEW_PLUGIN,
};
use crate::style_mod::{InlineRules, StyleSet, StyleSpec};
use crate::theme::{build_theme, ModuleRef, BASE_THEME, DARK_THEME, NAMES, STYLE_MODULE, THEME};
use crate::tile::TileKind;
use crate::tile::Tiles;
use crate::viewstate::{MeasureContext, MustMeasure, ViewState, Viewport};
use crate::Direction;

/// The configuration of an [`EditorView`] (`EditorViewConfig`).
pub struct EditorViewConfig {
    /// The view's initial state.
    pub state: EditorState,
    /// The element to append the editor to, if any.
    pub parent: Option<Element>,
    /// A [`EditorView::scroll_into_view`] effect to scroll the initial
    /// view with.
    pub scroll_to: Option<StateEffect>,
    /// The window's size (`innerWidth`, `innerHeight`).
    ///
    /// div-gap: 26 `window.innerWidth`/`innerHeight`: Div does not report
    /// its viewport size, so the host gives it.
    pub window: WindowSize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateState {
    // Not updating.
    Idle,
    // Reading the DOM layout.
    Measuring,
    // Updating the view's DOM.
    Updating,
}

/// A listener for view updates (`EditorView.updateListener`).
pub type UpdateListener = Callback<dyn Fn(&ViewUpdate) + Send + Sync>;

static UPDATE_LISTENER: LazyLock<Facet<UpdateListener>> = LazyLock::new(Facet::define);

static ANNOUNCE: LazyLock<StateEffectType<String>> = LazyLock::new(StateEffectType::define);

/// An editor view (`EditorView`): the user interface of an editor, drawn
/// on Div's DOM.
pub struct EditorView {
    document: Document,
    dom: Element,
    scroll_dom: Element,
    content_dom: Element,
    announce_dom: Element,
    view_state: ViewState,
    doc_view: Option<DocView>,
    editor_attrs: Option<Attrs>,
    content_attrs: Option<Attrs>,
    style_modules: Vec<ModuleRef>,
    style_set: StyleSet,
    inline_rules: Option<InlineRules>,
    update_state: UpdateState,
    measure_scheduled: bool,
    destroyed: bool,
    window: WindowSize,
    plugins: Vec<PluginInstance>,
    // The view's identity, for a stored multi-stroke key prefix.
    id: u64,
    input: InputState,
    bidi_cache: RefCell<Vec<CachedOrder>>,
}

impl EditorView {
    /// Create a view for `config.state`, drawn into new elements that
    /// `config.parent`, when given, receives (`new EditorView`).
    pub fn new(document: &Document, config: EditorViewConfig) -> Result<EditorView, BuildError> {
        let content_dom = create_element(document, "div")?;

        let scroll_dom = create_element(document, "div")?;
        set_attribute(&scroll_dom, "tabindex", "-1")?;
        set_attribute(&scroll_dom, "class", "cm-scroller")?;
        scroll_dom.append_child(&content_dom)?;

        let announce_dom = create_element(document, "div")?;
        set_attribute(&announce_dom, "class", "cm-announced")?;
        set_attribute(&announce_dom, "aria-live", "polite")?;

        let dom = create_element(document, "div")?;
        dom.append_child(&announce_dom)?;
        dom.append_child(&scroll_dom)?;

        if let Some(parent) = &config.parent {
            parent.append_child(&dom)?;
        }

        let mut view_state = ViewState::new(config.state.clone(), config.window.width);
        view_state.scroll_parent = Some(scroll_dom.clone());
        if let Some(effect) = &config.scroll_to {
            if let Some(target) = effect.value_of(&SCROLL_INTO_VIEW) {
                view_state.scroll_target = Some(target.clip(&view_state.state));
            }
        }
        let mut view = EditorView {
            document: document.clone(),
            dom,
            scroll_dom,
            content_dom,
            announce_dom,
            view_state,
            doc_view: None,
            editor_attrs: None,
            content_attrs: None,
            style_modules: Vec::new(),
            style_set: StyleSet::default(),
            inline_rules: None,
            update_state: UpdateState::Idle,
            measure_scheduled: false,
            destroyed: false,
            window: config.window,
            plugins: Vec::new(),
            id: crate::keymap::next_view_id(),
            input: InputState::new(),
            bidi_cache: RefCell::new(Vec::new()),
        };
        view.plugins = view
            .state()
            .facet(&VIEW_PLUGIN)
            .iter()
            .map(PluginInstance::new)
            .collect();
        for plugin in &view.plugins {
            plugin.update(&view);
        }
        view.ensure_handlers()?;
        let state = view.view_state.state.clone();
        view.doc_view = Some(DocView::new(
            document,
            view.content_dom.clone(),
            &state,
            &mut view.view_state,
        )?);

        view.mount_styles()?;
        view.update_attrs()?;
        view.update_state = UpdateState::Idle;

        view.request_measure();
        // `document.fonts.ready` would request a refreshing measure when
        // the fonts load (div-gap: 19 `document.fonts`); Div's fonts are
        // registered before they are used.
        view.run_requested_measure()?;
        Ok(view)
    }

    /// Create a view as [`new`](Self::new) does, shared so that its
    /// listeners can reach it: the view handles the DOM events on its
    /// content, as upstream's always does.
    ///
    /// A view made with [`new`](Self::new) has no listeners, as Div's
    /// listeners are `'static` closures with no other way to reach it.
    /// Events that arrive while the view is borrowed, because it is
    /// updating or its host holds it, run after its next update or
    /// measure, as upstream runs them after an update in progress.
    pub fn new_shared(
        document: &Document,
        config: EditorViewConfig,
    ) -> Result<Rc<RefCell<EditorView>>, BuildError> {
        let view = Rc::new(RefCell::new(EditorView::new(document, config)?));
        {
            let mut inner = view.borrow_mut();
            inner.input.shared = Some(Shared {
                view: Rc::downgrade(&view),
                pending: Rc::new(RefCell::new(VecDeque::new())),
            });
            inner.ensure_handlers()?;
        }
        Ok(view)
    }

    pub(crate) fn input(&self) -> &InputState {
        &self.input
    }

    pub(crate) fn input_mut(&mut self) -> &mut InputState {
        &mut self.input
    }

    // The plugins that have not crashed.
    pub(crate) fn plugin_sources(&self) -> Vec<PluginSource> {
        self.plugins
            .iter()
            .filter_map(PluginInstance::source)
            .collect()
    }

    pub(crate) fn doc_view_tiles(&self) -> &Tiles {
        &self
            .doc_view
            .as_ref()
            .expect("the view's content")
            .content
            .tiles
    }

    /// The current editor state (`state`).
    pub fn state(&self) -> &EditorState {
        &self.view_state.state
    }

    /// The document this view draws in.
    pub fn document(&self) -> &Document {
        &self.document
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    /// The editor's outer element (`dom`).
    pub fn dom(&self) -> &Element {
        &self.dom
    }

    /// The scrollable element (`scrollDOM`).
    pub fn scroll_dom(&self) -> &Element {
        &self.scroll_dom
    }

    /// The editable element holding the content (`contentDOM`).
    pub fn content_dom(&self) -> &Element {
        &self.content_dom
    }

    /// The element for screen reader announcements (`announceDOM`).
    pub fn announce_dom(&self) -> &Element {
        &self.announce_dom
    }

    /// The part of the document that is drawn (`viewport`).
    pub fn viewport(&self) -> Viewport {
        self.view_state.viewport
    }

    /// The drawn ranges that are not hidden by replacing decorations or
    /// line gaps (`visibleRanges`).
    pub fn visible_ranges(&self) -> &[(usize, usize)] {
        &self.view_state.visible_ranges
    }

    /// Whether the editor is in the window (`inView`).
    pub fn in_view(&self) -> bool {
        self.view_state.in_view
    }

    fn doc_view(&mut self) -> &mut DocView {
        self.doc_view.as_mut().expect("the view's content")
    }

    /// Update the view for a transaction (`dispatch`).
    pub fn dispatch(&mut self, tr: Transaction) -> Result<(), BuildError> {
        self.update(vec![tr])
    }

    /// Update the view for transactions that start from its state and
    /// follow each other (`update`).
    ///
    /// Panics, as upstream throws, when called during an update or with a
    /// transaction that does not start from the previous state.
    pub fn update(&mut self, transactions: Vec<Transaction>) -> Result<(), BuildError> {
        if self.update_state != UpdateState::Idle {
            panic!("Calls to EditorView.update are not allowed while an update is in progress");
        }
        let mut state = self.state().clone();
        for tr in &transactions {
            if !tr.start_state().ptr_eq(&state) {
                panic!(
                    "Trying to update state with a transaction that doesn't start from the \
                     previous state."
                );
            }
            state = tr.state().clone();
        }
        if self.destroyed {
            self.view_state.state = state;
            return Ok(());
        }

        // Focus changes and pending DOM changes come with the input
        // handling in milestone M7.

        // When the phrases change, redraw the editor.
        if state.facet(&facets::PHRASES) != self.state().facet(&facets::PHRASES) {
            return self.set_state(state);
        }

        let mut update = ViewUpdate::create(self.state(), &state, transactions);
        let mut scroll_target = self.view_state.scroll_target.clone();
        self.update_state = UpdateState::Updating;
        let result = (|| -> Result<(bool, bool), BuildError> {
            for tr in &update.transactions {
                if let Some(target) = &scroll_target {
                    scroll_target = Some(target.map(tr.changes().desc()));
                }
                if tr.scroll_into_view() {
                    let main = tr.state().selection().main();
                    let (x, y) = *self.state().facet(&CURSOR_SCROLL_MARGIN);
                    let range = if main.is_empty() {
                        main
                    } else {
                        EditorSelection::cursor_with(
                            main.head(),
                            if main.head() > main.anchor() { -1 } else { 1 },
                            None,
                            None,
                        )
                    };
                    scroll_target = Some(ScrollTarget::new(
                        range,
                        ScrollStrategy::Nearest,
                        ScrollStrategy::Nearest,
                        y,
                        x,
                    ));
                }
                for effect in tr.effects() {
                    if let Some(target) = effect.value_of(&SCROLL_INTO_VIEW) {
                        scroll_target = Some(target.clip(self.state()));
                    }
                }
            }
            let line_wrapping = self.line_wrapping();
            self.view_state
                .update(&mut update, scroll_target.clone(), line_wrapping);
            let cache = std::mem::take(self.bidi_cache.get_mut());
            *self.bidi_cache.get_mut() = CachedOrder::update(cache, &update.changes);
            if !update.empty() {
                self.update_plugins(&update);
                self.input.update(update.transactions.len());
            }
            let redrawn = {
                let vs = &mut self.view_state;
                self.doc_view
                    .as_mut()
                    .expect("the view's content")
                    .update(&update, vs)?
            };
            if *self.state().facet(&STYLE_MODULE) != self.style_modules {
                self.mount_styles()?;
            }
            let attrs_changed = self.update_attrs()?;
            self.apply_inline_rules()?;
            self.show_announcements(&update.transactions)?;
            Ok((redrawn, attrs_changed))
        })();
        self.update_state = UpdateState::Idle;
        let (redrawn, attrs_changed) = result?;

        if update.start_state.facet(&THEME) != update.state.facet(&THEME) {
            self.view_state.must_measure_content = MustMeasure::Yes;
        }
        if redrawn
            || attrs_changed
            || scroll_target.is_some()
            || self.view_state.must_enforce_cursor_assoc
            || self.view_state.must_measure_content != MustMeasure::No
        {
            self.request_measure();
        }
        if redrawn {
            self.doc_view_update();
        }
        if !update.empty() {
            for listener in self.state().facet(&UPDATE_LISTENER).clone() {
                listener(&update);
            }
        }
        let result = self.run_requested_measure();
        self.run_pending_events();
        result
    }

    /// Reset the view to `new_state`, redrawing everything (`setState`).
    pub fn set_state(&mut self, new_state: EditorState) -> Result<(), BuildError> {
        if self.update_state != UpdateState::Idle {
            panic!("Calls to EditorView.setState are not allowed while an update is in progress");
        }
        if self.destroyed {
            self.view_state.state = new_state;
            return Ok(());
        }
        self.update_state = UpdateState::Updating;
        let result = (|| -> Result<(), BuildError> {
            let mut view_state = ViewState::new(new_state.clone(), self.window.width);
            view_state.scroll_parent = Some(self.scroll_dom.clone());
            self.view_state = view_state;
            for plugin in &self.plugins {
                plugin.destroy();
            }
            self.plugins = new_state
                .facet(&VIEW_PLUGIN)
                .iter()
                .map(PluginInstance::new)
                .collect();
            for plugin in &self.plugins {
                plugin.update(self);
            }
            self.ensure_handlers()?;
            self.bidi_cache.get_mut().clear();
            if let Some(doc_view) = self.doc_view.take() {
                doc_view.destroy()?;
            }
            self.doc_view = Some(DocView::new(
                &self.document,
                self.content_dom.clone(),
                &new_state,
                &mut self.view_state,
            )?);
            self.mount_styles()?;
            self.update_attrs()?;
            Ok(())
        })();
        self.update_state = UpdateState::Idle;
        result?;
        self.request_measure();
        let result = self.run_requested_measure();
        self.run_pending_events();
        result
    }

    // `updatePlugins`: keep the plugins of the new state, create the new
    // ones, destroy those that left, and give the others the update.
    fn update_plugins(&mut self, update: &ViewUpdate) {
        let prev_specs = update.start_state.facet(&VIEW_PLUGIN).clone();
        let specs = update.state.facet(&VIEW_PLUGIN).clone();
        if prev_specs != specs {
            let mut old: Vec<Option<PluginInstance>> = std::mem::take(&mut self.plugins)
                .into_iter()
                .map(Some)
                .collect();
            let mut plugins = Vec::new();
            for spec in &specs {
                match prev_specs.iter().position(|prev| prev == spec) {
                    Some(found) if old.get(found).is_some_and(Option::is_some) => {
                        let plugin = old[found].take().expect("checked");
                        plugin.set_must_update(update);
                        plugins.push(plugin);
                    }
                    _ => plugins.push(PluginInstance::new(spec)),
                }
            }
            for plugin in old.into_iter().flatten() {
                plugin.destroy();
            }
            self.plugins = plugins;
        } else {
            for plugin in &self.plugins {
                plugin.set_must_update(update);
            }
        }
        for plugin in &self.plugins {
            plugin.update(self);
        }
        if prev_specs != specs {
            if let Err(error) = self.ensure_handlers() {
                log_exception("CodeMirror plugin crashed", &error);
            }
        }
    }

    // `docViewUpdate`: tell the plugins the content was redrawn.
    fn doc_view_update(&self) {
        for plugin in &self.plugins {
            plugin.doc_view_update(self);
        }
    }

    /// The value of `plugin` in this view, if it is there and did not
    /// crash (`plugin`). A plugin with a pending update is updated first.
    /// While a plugin's own update or creation runs, this returns `None`
    /// for it.
    pub fn plugin<V: PluginValue>(&self, plugin: &ViewPlugin<V>) -> Option<Ref<'_, V>> {
        let instance = self
            .plugins
            .iter()
            .find(|p| p.source().is_some_and(|source| plugin.is(&source)))?;
        instance.update(self);
        instance.get::<V>()
    }

    /// Schedule a layout measurement (`requestMeasure`).
    ///
    /// div-gap: 13 `requestAnimationFrame`. Upstream measures in the next
    /// animation frame; Div has none, so the view measures at the end of
    /// the constructor or update that requested it, before Div paints, and
    /// [`measure`](Self::measure) runs a measurement when the host knows
    /// the layout changed (after scrolling or resizing, which upstream
    /// observes with `scroll` events and `ResizeObserver`, gaps 11, 13).
    /// Measure requests with read and write callbacks come with view
    /// plugins.
    pub fn request_measure(&mut self) {
        self.measure_scheduled = true;
    }

    fn run_requested_measure(&mut self) -> Result<(), BuildError> {
        if self.measure_scheduled {
            self.measure()?;
        }
        Ok(())
    }

    /// Read the layout, update the viewport and redraw until they agree
    /// (`measure`).
    pub fn measure(&mut self) -> Result<(), BuildError> {
        if self.destroyed {
            return Ok(());
        }
        self.measure_scheduled = true;
        let result = self.measure_loop();
        self.update_state = UpdateState::Idle;
        self.measure_scheduled = false;
        let updated = result?;
        if let Some(updated) = updated {
            if !updated.empty() {
                for listener in self.state().facet(&UPDATE_LISTENER).clone() {
                    listener(&updated);
                }
            }
        }
        self.run_pending_events();
        Ok(())
    }

    fn measure_loop(&mut self) -> Result<Option<ViewUpdate>, BuildError> {
        let mut updated: Option<ViewUpdate> = None;
        let scroll = self.view_state.scroll_parent.clone();
        let mut scroll_offset = self
            .view_state
            .get_scroll_offset(&self.scroll_dom, &self.content_dom)?;
        let mut scroll_anchor_pos = self.view_state.scroll_anchor_pos;
        let mut scroll_anchor_height = self.view_state.scroll_anchor_height;
        let mut scroll_scale = self.view_state.scale_y;
        if (scroll_offset - self.view_state.scroll_offset).abs() > 1.0 {
            scroll_anchor_height = -1.0;
        }
        self.view_state.scroll_anchor_height = -1.0;

        let mut i = 0;
        loop {
            if scroll_anchor_height < 0.0 {
                if is_scrolled_to_bottom(scroll.as_ref())? {
                    scroll_anchor_pos = -1;
                    scroll_anchor_height =
                        self.view_state.height_map.height() / self.view_state.scale_y;
                } else {
                    let block = self.view_state.scroll_anchor_at(scroll_offset);
                    scroll_anchor_pos = block.from as i64;
                    scroll_anchor_height = block.top;
                }
                scroll_scale = self.view_state.scale_y;
            }
            self.update_state = UpdateState::Measuring;
            let body = self.document.body();
            let changed = self.view_state.measure(MeasureContext {
                editor_dom: &self.dom,
                scroll_dom: &self.scroll_dom,
                content_dom: &self.content_dom,
                body: &body,
                window: self.window,
                doc_view: self.doc_view.as_mut().expect("the view's content"),
            })?;
            if changed == 0 && self.view_state.scroll_target.is_none() {
                break;
            }
            if i > 5 {
                eprintln!("Viewport failed to stabilize");
                break;
            }
            let state = self.state().clone();
            let mut update = ViewUpdate::create(&state, &state, Vec::new());
            update.flags |= changed;
            match &mut updated {
                None => updated = Some(update.clone()),
                Some(updated) => updated.flags |= changed,
            }
            self.update_state = UpdateState::Updating;
            if !update.empty() {
                self.update_plugins(&update);
                self.update_attrs()?;
                let vs = &mut self.view_state;
                let redrawn = self
                    .doc_view
                    .as_mut()
                    .expect("the view's content")
                    .update(&update, vs)?;
                self.apply_inline_rules()?;
                if redrawn {
                    self.doc_view_update();
                }
            }
            if !update.viewport_changed() {
                if self.view_state.editor_height != 0.0 {
                    if let Some(target) = self.view_state.scroll_target.take() {
                        self.scroll_target_into_view(&target)?;
                        scroll_anchor_height = -1.0;
                        i += 1;
                        continue;
                    }
                    let new_anchor_height = if scroll_anchor_pos < 0 {
                        self.view_state.height_map.height()
                    } else {
                        self.view_state
                            .line_block_at(scroll_anchor_pos as usize)
                            .top
                    };
                    let diff = new_anchor_height / self.view_state.scale_y
                        - scroll_anchor_height / scroll_scale;
                    let scrolls_here = match &scroll {
                        Some(scroll) => scroll.is_same_node(&self.scroll_dom)?,
                        None => false,
                    };
                    // Upstream also corrects the scroll position after
                    // recent wheel and touch input, which comes with M7.
                    if !(-1.0..=1.0).contains(&diff) && (scrolls_here || self.has_focus()?) {
                        scroll_offset += diff;
                        match &scroll {
                            // `win.scrollBy` (div-gap: 26).
                            None => {}
                            Some(scroll) if scroll_anchor_pos < 0 => {
                                scroll.scroll_to(
                                    scroll.scroll_left()?,
                                    scroll.scroll_height()? as f64,
                                )?;
                            }
                            Some(scroll) => {
                                scroll.scroll_to(
                                    scroll.scroll_left()?,
                                    scroll.scroll_top()? + diff,
                                )?;
                            }
                        }
                        scroll_anchor_height = -1.0;
                        i += 1;
                        continue;
                    }
                }
                break;
            }
            i += 1;
        }
        let _ = scroll_offset;
        Ok(updated)
    }

    /// Whether the editor's content has focus (`hasFocus`).
    pub fn has_focus(&self) -> Result<bool, div::Error> {
        self.content_dom.has_focus()
    }

    /// The class names of the active themes (`themeClasses`).
    pub fn theme_classes(&self) -> String {
        let names = &*NAMES;
        format!(
            "{} {} {}",
            names.base,
            if *self.state().facet(&DARK_THEME) {
                &names.dark
            } else {
                &names.light
            },
            self.state().facet(&THEME)
        )
    }

    fn update_attrs(&mut self) -> Result<bool, div::Error> {
        let focused = self.has_focus()?;
        let mut editor_attrs = Attrs::new();
        editor_attrs.set(
            "class",
            format!(
                "cm-editor{}{}",
                if focused { " cm-focused " } else { " " },
                self.theme_classes()
            ),
        );
        let editor_attrs = attrs_from_facet(self.state(), &EDITOR_ATTRIBUTES, editor_attrs);
        let mut content_attrs = Attrs::new();
        content_attrs.set("spellcheck", "false");
        content_attrs.set("autocorrect", "off");
        content_attrs.set("autocapitalize", "off");
        content_attrs.set("writingsuggestions", "false");
        content_attrs.set("translate", "no");
        content_attrs.set(
            "contenteditable",
            if *self.state().facet(&EDITABLE) {
                "true"
            } else {
                "false"
            },
        );
        content_attrs.set("class", "cm-content");
        // `browser.tabSize` is `tab-size` in current browsers.
        content_attrs.set("style", format!("tab-size: {}", self.state().tab_size()));
        content_attrs.set("role", "textbox");
        content_attrs.set("aria-multiline", "true");
        if self.state().read_only() {
            content_attrs.set("aria-readonly", "true");
        }
        let content_attrs = attrs_from_facet(self.state(), &CONTENT_ATTRIBUTES, content_attrs);

        let changed_content = update_attrs(
            &self.content_dom,
            self.content_attrs.as_ref(),
            Some(&content_attrs),
        )?;
        let changed_editor =
            update_attrs(&self.dom, self.editor_attrs.as_ref(), Some(&editor_attrs))?;
        self.editor_attrs = Some(editor_attrs);
        self.content_attrs = Some(content_attrs);
        let changed = changed_content || changed_editor;
        if changed {
            self.apply_inline_rules()?;
        }
        Ok(changed)
    }

    fn show_announcements(&mut self, trs: &[Transaction]) -> Result<(), div::Error> {
        let mut first = true;
        for tr in trs {
            for effect in tr.effects() {
                if let Some(text) = effect.value_of(&ANNOUNCE) {
                    if first {
                        self.announce_dom.set_text("")?;
                        // Upstream resets the text to a no-break space
                        // after 200 ms, with a timer of the view's window,
                        // which comes with M7's input handling.
                        first = false;
                    }
                    let div = create_element(&self.document, "div")?;
                    div.set_text(text)?;
                    self.announce_dom.append_child(&div)?;
                }
            }
        }
        Ok(())
    }

    fn mount_styles(&mut self) -> Result<(), div::Error> {
        self.style_modules = self.state().facet(&STYLE_MODULE).clone();
        let mut modules: Vec<_> = self.style_modules.iter().map(|m| m.0.clone()).collect();
        modules.push(BASE_THEME.clone());
        modules.reverse();
        self.style_set.mount(&modules);
        self.inline_rules = Some(InlineRules::new(&self.style_set));
        self.apply_inline_rules()
    }

    // div-gap: 14a stylesheets. Apply the mounted rules as inline styles
    // (see `InlineRules`) to every element of the editor, after each
    // change to its DOM, where a browser would restyle.
    fn apply_inline_rules(&self) -> Result<(), div::Error> {
        if let Some(rules) = &self.inline_rules {
            let mut stack = vec![self.dom.clone()];
            while let Some(element) = stack.pop() {
                rules.apply(&element)?;
                let mut child = element.first_child()?;
                while let Some(next) = child {
                    child = next.next_sibling()?;
                    stack.push(next);
                }
            }
        }
        Ok(())
    }

    /// The text of the `<style>` element the mounted theme modules would
    /// fill (div-gap: 14b).
    pub fn mounted_style_text(&self) -> String {
        self.style_set.text()
    }

    /// The number of mounted rules and declarations that the inline
    /// scaffolding for stylesheets skips (div-gap: 14a): rules with
    /// selectors it cannot match, and declarations Div cannot parse.
    pub fn skipped_inline_rules(&self) -> (usize, usize) {
        self.inline_rules
            .as_ref()
            .map_or((0, 0), |r| (r.skipped_rules, r.skipped_declarations))
    }

    // `docView.scrollIntoView(target)`.
    fn scroll_target_into_view(&mut self, target: &ScrollTarget) -> Result<(), div::Error> {
        if target.is_snapshot {
            let block = self.view_state.line_block_at(target.range.head());
            self.scroll_dom
                .scroll_to(target.x_margin, block.top - target.y_margin)?;
            return Ok(());
        }
        // The `scrollHandler` facet comes with view plugins.
        let range = target.range;
        let side = if range.assoc() != 0 {
            range.assoc() as i32
        } else if range.head() > range.anchor() {
            -1
        } else {
            1
        };
        let Some(mut rect) = self.coords_at(range.head(), side)? else {
            return Ok(());
        };
        if !range.is_empty() {
            let side = if range.anchor() > range.head() { -1 } else { 1 };
            if let Some(other) = self.coords_at(range.anchor(), side)? {
                rect = Rect {
                    left: rect.left.min(other.left),
                    top: rect.top.min(other.top),
                    right: rect.right.max(other.right),
                    bottom: rect.bottom.max(other.bottom),
                };
            }
        }
        let (left, right, top, bottom) = get_scroll_margins(self);
        let target_rect = Rect {
            left: rect.left - left,
            top: rect.top - top,
            right: rect.right + right,
            bottom: rect.bottom + bottom,
        };
        // `offsetWidth`/`offsetHeight` (div-gap: 27): the bounding box,
        // which is the same without a transform.
        let scroll_rect = bounding_rect(&self.scroll_dom)?;
        let (offset_width, offset_height) = (scroll_rect.width(), scroll_rect.height());
        scroll_rect_into_view(
            &self.scroll_dom,
            &self.document.body(),
            self.window,
            target_rect,
            if range.head() < range.anchor() { -1 } else { 1 },
            target.x,
            target.y,
            target.x_margin.min(offset_width).max(-offset_width),
            target.y_margin.min(offset_height).max(-offset_height),
            self.text_direction() == Direction::Ltr,
        )?;
        // The visual viewport kludge for mobile browsers does not apply.
        Ok(())
    }

    /// The screen rectangle of the cursor position `pos`, on `side`
    /// (`docView.coordsAt`), or `None` when it is not drawn.
    ///
    /// div-gap: 5 Range geometry, 6 `caretPositionFromPoint`. Upstream
    /// measures the character with `Range.getClientRects`. Div cannot
    /// measure text, so this places the position in its drawn line, as a
    /// measured monospace cell (design section 7): at the line's content
    /// box left plus the column times the character width, with the
    /// text's height centered in the line box. It is wrong for wide
    /// characters, proportional fonts, bidi text and wrapped lines.
    fn coords_at(&mut self, pos: usize, side: i32) -> Result<Option<Rect>, div::Error> {
        let (tile, offset) = self.doc_view().block_at(pos, side);
        let doc_view = self.doc_view.as_ref().expect("the view's content");
        let tiles = &doc_view.content.tiles;
        let Some(element) = tiles[tile].dom.element().cloned() else {
            return Ok(None);
        };
        let rect = bounding_rect(&element)?;
        if !matches!(tiles[tile].kind, TileKind::Line(_)) {
            // A block widget or gap: its own box.
            return Ok(Some(rect));
        }
        let style = element.computed_style()?;
        let padding_left = style.padding_left();
        let padding_left = if padding_left.is_fixed() {
            padding_left.value() as f64
        } else {
            0.0
        };
        let line_start = pos - offset;
        let column = self
            .state()
            .doc()
            .slice_string(line_start, pos)
            .chars()
            .count();
        let oracle = &self.view_state.height_oracle;
        let x = rect.left + padding_left + column as f64 * oracle.char_width;
        let top = rect.top + (rect.height() - oracle.text_height) / 2.0;
        Ok(Some(Rect {
            left: x,
            right: x,
            top,
            bottom: top + oracle.text_height,
        }))
    }

    /// The top of the document in screen coordinates, the top of the
    /// first line below the padding (`documentTop`).
    pub fn document_top(&self) -> Result<f64, div::Error> {
        Ok(bounding_rect(&self.content_dom)?.top + self.view_state.padding_top)
    }

    /// The padding above and below the document (`documentPadding`).
    pub fn document_padding(&self) -> (f64, f64) {
        (self.view_state.padding_top, self.view_state.padding_bottom)
    }

    /// The horizontal scale of the editor's CSS transforms (`scaleX`).
    pub fn scale_x(&self) -> f64 {
        self.view_state.scale_x
    }

    /// The vertical scale (`scaleY`).
    pub fn scale_y(&self) -> f64 {
        self.view_state.scale_y
    }

    /// The block at `height` from the top of the document
    /// (`elementAtHeight`).
    pub fn element_at_height(&self, height: f64) -> BlockInfo {
        self.view_state.element_at_height(height)
    }

    /// The line block at `height` (`lineBlockAtHeight`).
    pub fn line_block_at_height(&self, height: f64) -> BlockInfo {
        self.view_state.line_block_at_height(height)
    }

    /// The line blocks in the viewport (`viewportLineBlocks`).
    pub fn viewport_line_blocks(&self) -> &[BlockInfo] {
        &self.view_state.viewport_lines
    }

    /// The line block around `pos` (`lineBlockAt`).
    pub fn line_block_at(&self, pos: usize) -> BlockInfo {
        self.view_state.line_block_at(pos)
    }

    /// The height of the content, with its padding (`contentHeight`).
    pub fn content_height(&self) -> f64 {
        self.view_state.content_height()
    }

    /// The default character width (`defaultCharacterWidth`).
    pub fn default_character_width(&self) -> f64 {
        self.view_state.height_oracle.char_width
    }

    /// The default line height (`defaultLineHeight`).
    pub fn default_line_height(&self) -> f64 {
        self.view_state.height_oracle.line_height
    }

    /// The content's text direction (`textDirection`).
    pub fn text_direction(&self) -> Direction {
        self.view_state.default_text_direction
    }

    /// The text direction of the block at `pos`, as CSS assigns it
    /// (`textDirectionAt`). Without
    /// [`per_line_text_direction`](Self::per_line_text_direction), or
    /// outside the viewport, this is [`text_direction`](Self::text_direction).
    ///
    /// Upstream first runs a pending measure (`readMeasured`). The port
    /// measures right after each update (gap 13), so only a measure
    /// requested with [`request_measure`](Self::request_measure) can wait.
    pub fn text_direction_at(&self, pos: usize) -> Direction {
        let viewport = self.viewport();
        if !*self.state().facet(&PER_LINE_TEXT_DIRECTION)
            || pos < viewport.from
            || pos > viewport.to
        {
            return self.text_direction();
        }
        let Some(doc_view) = &self.doc_view else {
            return self.text_direction();
        };
        // `DocView.textDirectionAt`.
        let (tile, _) = doc_view.block_at(pos, 1);
        let rtl = doc_view.content.tiles[tile]
            .dom
            .element()
            .and_then(|dom| dom.computed_style().ok())
            .is_some_and(|style| *style.direction() == div::typed_style::Direction::Rtl);
        if rtl {
            Direction::Rtl
        } else {
            Direction::Ltr
        }
    }

    /// The bidirectional structure of `line`, which must be in the current
    /// document, as spans in the order of the [text
    /// direction](Self::text_direction): leftmost first for left-to-right
    /// text, rightmost first otherwise (`bidiSpans`).
    pub fn bidi_spans(&self, line: &Line) -> Vec<BidiSpan> {
        if line.len() > MAX_BIDI_LINE && line.text.encode_utf16().count() > MAX_BIDI_LINE {
            return trivial_order(line.len());
        }
        let dir = self.text_direction_at(line.from);
        let mut isolates: Option<Vec<Isolate>> = None;
        for entry in self.bidi_cache.borrow().iter() {
            if entry.from == line.from
                && entry.dir == dir
                && (entry.fresh
                    || *isolates.get_or_insert_with(|| get_isolated_ranges(self.state(), line))
                        == entry.isolates)
            {
                return entry.order.clone();
            }
        }
        let isolates = isolates.unwrap_or_else(|| get_isolated_ranges(self.state(), line));
        let order = compute_order(line.text, dir, &isolates);
        self.bidi_cache.borrow_mut().push(CachedOrder {
            from: line.from,
            to: line.to,
            dir,
            isolates,
            fresh: true,
            order: order.clone(),
        });
        order
    }

    /// Move a cursor position by grapheme cluster (`moveByChar`).
    /// `forward` moves away from the line's start. Bidirectional text is
    /// traversed in visual order, by the [text
    /// direction](Self::text_direction) of the starting line. From the
    /// last position of a line, the result is across the line break; with
    /// no further line, it is `start`. The result skips atomic ranges.
    pub fn move_by_char(&self, start: &SelectionRange, forward: bool) -> SelectionRange {
        skip_atoms(
            self,
            start,
            cursor::move_by_char(self, start, forward, None),
        )
    }

    /// [`move_by_char`](Self::move_by_char), moving over more than one
    /// cluster (`moveByChar` with `by`): `by` is called with the first
    /// cluster and returns a predicate that tells, for each cluster after
    /// it, whether to move over it too.
    pub fn move_by_char_by<'a>(
        &'a self,
        start: &SelectionRange,
        forward: bool,
        by: impl FnOnce(&str) -> ByChar<'a> + 'a,
    ) -> SelectionRange {
        let moved = cursor::move_by_char(self, start, forward, Some(Box::new(by)));
        skip_atoms(self, start, moved)
    }

    /// Move a cursor position across the next group of word characters,
    /// or of other characters that are not whitespace, with the
    /// whitespace before it (`moveByGroup`).
    pub fn move_by_group(&self, start: &SelectionRange, forward: bool) -> SelectionRange {
        let head = start.head();
        let moved = cursor::move_by_char(
            self,
            start,
            forward,
            Some(Box::new(move |initial: &str| by_group(self, head, initial))),
        );
        skip_atoms(self, start, moved)
    }

    /// The cursor at the visual start or end of `line`
    /// (`visualLineSide`, deprecated upstream).
    pub fn visual_line_side(&self, line: &Line, end: bool) -> SelectionRange {
        if end {
            EditorSelection::cursor_with(line.to, 1, None, None)
        } else {
            EditorSelection::cursor_with(line.from, -1, None, None)
        }
    }

    /// Move to the next line boundary in a direction
    /// (`moveToLineBoundary`). With `include_wrap`, in a wrapped line,
    /// upstream stops at the next wrap point; the port goes to the line's
    /// start or end until Div offers the geometry that finds wrap points
    /// (gaps 5 and 6).
    pub fn move_to_line_boundary(
        &self,
        start: &SelectionRange,
        forward: bool,
        include_wrap: bool,
    ) -> SelectionRange {
        cursor::move_to_line_boundary(self, start, forward, include_wrap)
    }

    /// Whether lines wrap (`lineWrapping`).
    pub fn line_wrapping(&self) -> bool {
        self.view_state.height_oracle.line_wrapping
    }

    /// Remove the editor's DOM and stop updating (`destroy`).
    pub fn destroy(&mut self) -> Result<(), div::Error> {
        if self.destroyed {
            return Ok(());
        }
        for plugin in std::mem::take(&mut self.plugins) {
            plugin.destroy();
        }
        if let Some(doc_view) = self.doc_view.take() {
            doc_view.destroy()?;
        }
        self.dom.remove()?;
        self.destroyed = true;
        Ok(())
    }

    /// An effect that scrolls `range` into view (`scrollIntoView`), with
    /// the strategies and margins upstream defaults to: nearest, 5px.
    pub fn scroll_into_view(range: SelectionRange) -> StateEffect {
        EditorView::scroll_into_view_with(
            range,
            ScrollStrategy::Nearest,
            ScrollStrategy::Nearest,
            5.0,
            5.0,
        )
    }

    /// [`scroll_into_view`](Self::scroll_into_view) with the strategies
    /// and margins given.
    pub fn scroll_into_view_with(
        range: SelectionRange,
        y: ScrollStrategy,
        x: ScrollStrategy,
        y_margin: f64,
        x_margin: f64,
    ) -> StateEffect {
        SCROLL_INTO_VIEW.of(ScrollTarget::new(range, y, x, y_margin, x_margin))
    }

    /// A theme: styles for the editor, scoped to a class the editor gets
    /// (`theme`). `&` in a selector is the editor element. Fails, as
    /// upstream throws, on an unsupported `&` form.
    pub fn theme(spec: &StyleSpec, dark: bool) -> Result<Extension, String> {
        let names = &*NAMES;
        let _ = names;
        let prefix = crate::style_mod::StyleModule::new_name();
        let module = build_theme(&format!(".{prefix}"), spec, None)?;
        let mut result = vec![
            THEME.of(prefix),
            STYLE_MODULE.of(ModuleRef(Arc::new(module))),
        ];
        if dark {
            result.push(DARK_THEME.of(true));
        }
        Ok(Extension::from(result))
    }

    /// Styles added to the base theme (`baseTheme`), which may use
    /// `&light` and `&dark`.
    pub fn base_theme(spec: &StyleSpec) -> Result<Extension, String> {
        let names = &*NAMES;
        let light = format!(".{}", names.light);
        let dark = format!(".{}", names.dark);
        let module = build_theme(&format!(".{}", names.base), spec, Some((&light, &dark)))?;
        Ok(Prec::lowest(STYLE_MODULE.of(ModuleRef(Arc::new(module)))))
    }

    /// DOM event handlers for the content (`domEventHandlers`): each
    /// returns true when it handled the event, which then has its default
    /// prevented and is not given to later handlers. Handlers given with
    /// higher precedence run first.
    pub fn dom_event_handlers(handlers: DomEventHandlers) -> Extension {
        ViewPlugin::define(
            |_| Ok(NoValue),
            PluginSpec {
                event_handlers: handlers,
                ..PluginSpec::default()
            },
        )
        .extension()
    }

    /// DOM event observers for the content (`domEventObservers`): like
    /// handlers, but all run, before the handlers, and cannot prevent
    /// the event's default.
    pub fn dom_event_observers(observers: DomEventHandlers) -> Extension {
        ViewPlugin::define(
            |_| Ok(NoValue),
            PluginSpec {
                event_observers: observers,
                ..PluginSpec::default()
            },
        )
        .extension()
    }

    /// Ranges the cursor moves over as a unit (`atomicRanges`).
    pub fn atomic_ranges(
        ranges: impl Fn(&EditorView) -> DecorationSet + Send + Sync + 'static,
    ) -> Extension {
        ATOMIC_RANGES.of(Callback(Arc::new(ranges)))
    }

    /// Read the text direction of each line from its CSS, instead of
    /// using the editor's for all lines (`perLineTextDirection`).
    pub fn per_line_text_direction(enabled: bool) -> Extension {
        PER_LINE_TEXT_DIRECTION.of(enabled)
    }

    /// Ranges that the view orders as bidi isolates, as an element with
    /// `unicode-bidi: isolate` is (`bidiIsolatedRanges`). Each range is a
    /// mark decoration whose [`MarkSpec::bidi_isolate`](crate::MarkSpec::bidi_isolate) gives its
    /// direction. Upstream also takes functions of the view.
    pub fn bidi_isolated_ranges(set: DecorationSet) -> Extension {
        BIDI_ISOLATED_RANGES.of(set)
    }

    /// Decorations to draw (`decorations`).
    pub fn decorations(set: DecorationSet) -> Extension {
        DECORATIONS.of(set)
    }

    /// Decorations drawn around the others (`outerDecorations`).
    pub fn outer_decorations(set: DecorationSet) -> Extension {
        OUTER_DECORATIONS.of(set)
    }

    /// Block wrappers (`blockWrappers`).
    pub fn block_wrappers(set: RangeSet<BlockWrapper>) -> Extension {
        BLOCK_WRAPPERS.of(set)
    }

    /// Attributes for the content element (`contentAttributes`).
    pub fn content_attributes(attrs: Attrs) -> Extension {
        CONTENT_ATTRIBUTES.of(attrs)
    }

    /// Attributes for the editor element (`editorAttributes`).
    pub fn editor_attributes(attrs: Attrs) -> Extension {
        EDITOR_ATTRIBUTES.of(attrs)
    }

    /// Whether the content is editable (`editable`).
    pub fn editable(editable: bool) -> Extension {
        EDITABLE.of(editable)
    }

    /// Wrap lines (`lineWrapping`).
    pub fn line_wrapping_extension() -> Extension {
        let mut attrs = Attrs::new();
        attrs.set("class", "cm-lineWrapping");
        CONTENT_ATTRIBUTES.of(attrs)
    }

    /// Mark the theme dark (`darkTheme`).
    pub fn dark_theme(dark: bool) -> Extension {
        DARK_THEME.of(dark)
    }

    /// A listener for the view's updates (`updateListener`).
    pub fn update_listener(listener: impl Fn(&ViewUpdate) + Send + Sync + 'static) -> Extension {
        UPDATE_LISTENER.of(Callback(Arc::new(listener)))
    }

    /// The margin kept around the cursor when a transaction scrolls it
    /// into view (`cursorScrollMargin`).
    pub fn cursor_scroll_margin(x: f64, y: f64) -> Extension {
        CURSOR_SCROLL_MARGIN.of((x, y))
    }

    /// The effect that announces text to screen readers (`announce`).
    pub fn announce(text: impl Into<String>) -> StateEffect {
        ANNOUNCE.of(text.into())
    }

    /// The editor's `outerHTML`, read from Div's DOM, for the structural
    /// fidelity check.
    ///
    /// div-gap: 23 attribute enumeration, 24 node traversal, 25
    /// `nodeName`: the view's own elements are written with the
    /// attributes the view gives them, in the order it first set them,
    /// the content's blocks as [`crate::__test::ContentTiles`] writes
    /// them, and the gutters from the gutter plugin's own record of its
    /// elements.
    #[doc(hidden)]
    pub fn outer_html(&self) -> Result<String, div::Error> {
        let mut out = String::new();
        let names = |attrs: &Option<Attrs>| -> Vec<String> {
            attrs
                .as_ref()
                .map(|a| a.iter().map(|(n, _)| n.to_string()).collect())
                .unwrap_or_default()
        };
        let editor_names = names(&self.editor_attrs);
        let content_names = names(&self.content_attrs);
        fn refs(v: &[String]) -> Vec<&str> {
            v.iter().map(String::as_str).collect()
        }
        open_tag_with(&self.dom, &refs(&editor_names), &mut out)?;
        open_tag_with(&self.announce_dom, &["class", "aria-live"], &mut out)?;
        out.push_str("</div>");
        open_tag_with(&self.scroll_dom, &["tabindex", "class"], &mut out)?;
        let (before, after) = crate::gutter::gutters_html(self)?;
        out.push_str(&before);
        out.push_str(&self.content_html_with(&refs(&content_names))?);
        out.push_str(&after);
        out.push_str("</div></div>");
        Ok(out)
    }

    /// The `.cm-content` element's `outerHTML` (see
    /// [`outer_html`](Self::outer_html)).
    #[doc(hidden)]
    pub fn content_html(&self) -> Result<String, div::Error> {
        let names: Vec<String> = self
            .content_attrs
            .as_ref()
            .map(|a| a.iter().map(|(n, _)| n.to_string()).collect())
            .unwrap_or_default();
        self.content_html_with(&names.iter().map(String::as_str).collect::<Vec<_>>())
    }

    fn content_html_with(&self, names: &[&str]) -> Result<String, div::Error> {
        let mut out = String::new();
        open_tag_with(&self.content_dom, names, &mut out)?;
        let doc_view = self.doc_view.as_ref().expect("the view's content");
        for block in doc_view.content.block_html()? {
            out.push_str(&block);
        }
        out.push_str("</div>");
        Ok(out)
    }
}

// `attrsFromFacet`: combine the facet's attribute sets into `base`, the
// last source first.
fn attrs_from_facet(state: &EditorState, facet: &Facet<Attrs>, mut base: Attrs) -> Attrs {
    for source in state.facet(facet).iter().rev() {
        base = combine_attrs(source, base);
    }
    base
}

// The value of the plugins `dom_event_handlers` and `dom_event_observers`
// define (upstream's `{}`).
// The longest line, in UTF-16 code units, whose bidi structure is
// computed (`MaxBidiLine`).
const MAX_BIDI_LINE: usize = 4096;

// A line's bidi structure, kept between calls (`CachedOrder`).
pub(crate) struct CachedOrder {
    from: usize,
    to: usize,
    dir: Direction,
    isolates: Vec<Isolate>,
    fresh: bool,
    order: Vec<BidiSpan>,
}

impl CachedOrder {
    // Keep the last ten entries untouched by `changes`, mapped.
    fn update(cache: Vec<CachedOrder>, changes: &ChangeSet) -> Vec<CachedOrder> {
        if changes.is_empty() && !cache.iter().any(|entry| entry.fresh) {
            return cache;
        }
        let last_dir = cache.last().map_or(Direction::Ltr, |entry| entry.dir);
        let start = cache.len().saturating_sub(10);
        cache
            .into_iter()
            .skip(start)
            .filter(|entry| {
                entry.dir == last_dir && changes.touches_range(entry.from, entry.to) == Touch::No
            })
            .map(|entry| CachedOrder {
                from: changes.map_pos(entry.from, 1),
                to: changes.map_pos(entry.to, -1),
                fresh: false,
                ..entry
            })
            .collect()
    }
}

struct NoValue;

impl PluginValue for NoValue {}
