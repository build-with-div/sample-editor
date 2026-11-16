//! View plugins, the part of `extension.ts` that associates stateful
//! values with a view: [`ViewPlugin`], [`PluginValue`] and the instances
//! the view keeps.
//!
//! A plugin value lives in the view. Upstream hands it the view through
//! `update.view`; here [`PluginValue::update`] receives the view next to
//! the update, and [`EditorView::plugin`] borrows a value for reading.
//! A plugin's DOM event handlers and observers (`eventHandlers`,
//! `eventObservers`) receive the view and the event; a handler reaches the
//! plugin's own value, which upstream passes as `this`, through
//! [`EditorView::plugin`]. The `decorations` option and plugin arguments
//! (`of`) come later in milestone M7.

use std::any::Any;
use std::cell::{Ref, RefCell};
use std::marker::PhantomData;
use std::sync::{Arc, LazyLock, OnceLock};

use editor_state::{Callback, Extension, Facet, FacetConfig};

use crate::extension::ViewUpdate;
use crate::EditorView;

/// The value of a view plugin (`PluginValue`).
pub trait PluginValue: Any {
    /// Notifies the plugin of an update that happened in `view`, before
    /// the view updates its own DOM (`update`). It updates the plugin's
    /// state and writes the DOM for the changes in the update; it should
    /// not read the layout.
    ///
    /// An error drops the plugin from the view, as an exception does
    /// upstream.
    fn update(&mut self, _update: &ViewUpdate, _view: &EditorView) -> Result<(), div::Error> {
        Ok(())
    }

    /// Called when the view redrew its content (`docViewUpdate`).
    fn doc_view_update(&mut self, _view: &EditorView) -> Result<(), div::Error> {
        Ok(())
    }

    /// Called when the plugin is no longer used (`destroy`). It should
    /// revert the changes it made to the DOM.
    fn destroy(&mut self) -> Result<(), div::Error> {
        Ok(())
    }
}

type Create = dyn Fn(&EditorView) -> Result<Box<dyn PluginValue>, div::Error> + Send + Sync;

/// A DOM event handler or observer (`DOMEventHandlers`' functions): given
/// the view and the event, a handler returns true when it handled the
/// event, which then has its default prevented. An observer's result is
/// ignored.
pub type DomEventHandler = Callback<dyn Fn(&mut EditorView, &div::Event) -> bool + Send + Sync>;

/// DOM event handlers by event type, in the order given
/// (`DOMEventHandlers`).
pub type DomEventHandlers = Vec<(String, DomEventHandler)>;

/// Wrap `f` as a [`DomEventHandler`].
pub fn dom_event_handler(
    f: impl Fn(&mut EditorView, &div::Event) -> bool + Send + Sync + 'static,
) -> DomEventHandler {
    Callback(Arc::new(f))
}

struct PluginInner {
    create: Box<Create>,
    extension: OnceLock<Extension>,
    event_handlers: DomEventHandlers,
    event_observers: DomEventHandlers,
}

/// A view plugin's identity in the [`VIEW_PLUGIN`] facet
/// (`PluginSource`).
#[derive(Clone)]
pub(crate) struct PluginSource(Arc<PluginInner>);

impl PartialEq for PluginSource {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl PluginSource {
    pub(crate) fn event_handlers(&self) -> &DomEventHandlers {
        &self.0.event_handlers
    }

    pub(crate) fn event_observers(&self) -> &DomEventHandlers {
        &self.0.event_observers
    }
}

/// The plugins in a state, each once (`viewPlugin`).
pub(crate) static VIEW_PLUGIN: LazyLock<Facet<PluginSource, Vec<PluginSource>>> =
    LazyLock::new(|| {
        Facet::define_with(FacetConfig::new(|plugins: &[PluginSource]| {
            let mut out: Vec<PluginSource> = Vec::new();
            for plugin in plugins {
                if !out.contains(plugin) {
                    out.push(plugin.clone());
                }
            }
            out
        }))
    });

/// Extensions a plugin adds to a configuration it is in (`provide`).
pub type Provide<V> = Box<dyn FnOnce(&ViewPlugin<V>) -> Extension + Send>;

/// Options for [`ViewPlugin::define`] (`PluginSpec`).
pub struct PluginSpec<V: PluginValue> {
    /// Extensions the plugin adds to a configuration it is in
    /// (`provide`).
    pub provide: Option<Provide<V>>,
    /// DOM event handlers for the plugin (`eventHandlers`), run as
    /// [`EditorView::dom_event_handlers`] runs them.
    pub event_handlers: DomEventHandlers,
    /// DOM event observers for the plugin (`eventObservers`), run as
    /// [`EditorView::dom_event_observers`] runs them.
    pub event_observers: DomEventHandlers,
}

impl<V: PluginValue> Default for PluginSpec<V> {
    fn default() -> Self {
        PluginSpec {
            provide: None,
            event_handlers: Vec::new(),
            event_observers: Vec::new(),
        }
    }
}

/// A view plugin (`ViewPlugin`): a value of type `V`, created for each
/// view whose state includes [`extension`](Self::extension), notified of
/// the view's updates.
pub struct ViewPlugin<V: PluginValue> {
    inner: Arc<PluginInner>,
    value: PhantomData<fn() -> V>,
}

impl<V: PluginValue> Clone for ViewPlugin<V> {
    fn clone(&self) -> Self {
        ViewPlugin {
            inner: self.inner.clone(),
            value: PhantomData,
        }
    }
}

impl<V: PluginValue> ViewPlugin<V> {
    /// Define a plugin whose value `create` makes for a view (`define`,
    /// `fromClass`).
    pub fn define(
        create: impl Fn(&EditorView) -> Result<V, div::Error> + Send + Sync + 'static,
        spec: PluginSpec<V>,
    ) -> ViewPlugin<V> {
        let plugin = ViewPlugin {
            inner: Arc::new(PluginInner {
                create: Box::new(move |view| {
                    create(view).map(|value| Box::new(value) as Box<dyn PluginValue>)
                }),
                extension: OnceLock::new(),
                event_handlers: spec.event_handlers,
                event_observers: spec.event_observers,
            }),
            value: PhantomData,
        };
        let mut extensions = Vec::new();
        if let Some(provide) = spec.provide {
            extensions.push(provide(&plugin));
        }
        extensions.push(VIEW_PLUGIN.of(PluginSource(plugin.inner.clone())));
        let _ = plugin.inner.extension.set(Extension::from(extensions));
        plugin
    }

    /// The extension that adds the plugin to a state (`extension`).
    pub fn extension(&self) -> Extension {
        self.inner.extension.get().expect("defined").clone()
    }

    pub(crate) fn is(&self, source: &PluginSource) -> bool {
        Arc::ptr_eq(&self.inner, &source.0)
    }
}

/// A plugin in a view (`PluginInstance`).
pub(crate) struct PluginInstance {
    // `None` once the plugin crashed.
    spec: RefCell<Option<PluginSource>>,
    // `None` until the first `update` creates it.
    value: RefCell<Option<Box<dyn PluginValue>>>,
    // The update the plugin has yet to see, set when an update starts.
    must_update: RefCell<Option<ViewUpdate>>,
}

// `logException`: report an error from a plugin or listener.
pub(crate) fn log_exception(context: &str, error: &div::Error) {
    eprintln!("{context}: {error}");
}

impl PluginInstance {
    pub(crate) fn new(spec: &PluginSource) -> PluginInstance {
        PluginInstance {
            spec: RefCell::new(Some(spec.clone())),
            value: RefCell::new(None),
            must_update: RefCell::new(None),
        }
    }

    pub(crate) fn source(&self) -> Option<PluginSource> {
        self.spec.borrow().clone()
    }

    pub(crate) fn set_must_update(&self, update: &ViewUpdate) {
        *self.must_update.borrow_mut() = Some(update.clone());
    }

    /// Create the value, or give it the pending update (`update`). A
    /// plugin that is updating already, and asks for itself through
    /// [`EditorView::plugin`], is left as it is.
    pub(crate) fn update(&self, view: &EditorView) {
        let Ok(mut value) = self.value.try_borrow_mut() else {
            return;
        };
        match value.as_mut() {
            None => {
                let spec = self.spec.borrow().clone();
                if let Some(spec) = spec {
                    match (spec.0.create)(view) {
                        Ok(created) => *value = Some(created),
                        Err(error) => {
                            log_exception("CodeMirror plugin crashed", &error);
                            *self.spec.borrow_mut() = None;
                        }
                    }
                }
            }
            Some(plugin) => {
                let update = self.must_update.borrow_mut().take();
                if let Some(update) = update {
                    if let Err(error) = plugin.update(&update, view) {
                        log_exception("CodeMirror plugin crashed", &error);
                        let _ = plugin.destroy();
                        *value = None;
                        *self.spec.borrow_mut() = None;
                    }
                }
            }
        }
    }

    pub(crate) fn doc_view_update(&self, view: &EditorView) {
        if let Ok(mut value) = self.value.try_borrow_mut() {
            if let Some(plugin) = value.as_mut() {
                if let Err(error) = plugin.doc_view_update(view) {
                    log_exception("doc view update listener", &error);
                }
            }
        }
    }

    pub(crate) fn destroy(&self) {
        if let Some(plugin) = self.value.borrow_mut().as_mut() {
            if let Err(error) = plugin.destroy() {
                log_exception("CodeMirror plugin crashed", &error);
            }
        }
    }

    /// The value, if it is a `V` and not being updated.
    pub(crate) fn get<V: PluginValue>(&self) -> Option<Ref<'_, V>> {
        let value = self.value.try_borrow().ok()?;
        Ref::filter_map(value, |value| {
            value
                .as_deref()
                .and_then(|value| (value as &dyn Any).downcast_ref::<V>())
        })
        .ok()
    }
}
