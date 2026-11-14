//! Port of `transaction.ts`: annotations, state effects, transaction specs
//! and transactions, with change filters, transaction filters and
//! transaction extenders.

use std::fmt;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, OnceLock};

use crate::change::{ChangeDesc, ChangeSet, ChangeSpec};
use crate::facet::{AnyValue, Callback, Extension};
use crate::selection::{check_selection, EditorSelection};
use crate::state::{facets, EditorState};
use crate::text::Text;
use crate::RangeError;

// Ids 0 to 3 are the annotation types `Transaction` defines.
static NEXT_ANNOTATION_ID: AtomicUsize = AtomicUsize::new(4);

/// A type of [`Annotation`] holding values of type `T`
/// (`AnnotationType`).
pub struct AnnotationType<T> {
    id: usize,
    _value: PhantomData<fn() -> T>,
}

impl<T> Clone for AnnotationType<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for AnnotationType<T> {}

impl<T> fmt::Debug for AnnotationType<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AnnotationType#{}", self.id)
    }
}

impl<T: Send + Sync + 'static> AnnotationType<T> {
    const fn reserved(id: usize) -> Self {
        AnnotationType {
            id,
            _value: PhantomData,
        }
    }

    /// Define a new type of annotation (`Annotation.define`).
    #[allow(clippy::new_without_default)]
    pub fn define() -> Self {
        AnnotationType::reserved(NEXT_ANNOTATION_ID.fetch_add(1, Ordering::Relaxed))
    }

    /// An annotation of this type (`of`).
    pub fn of(self, value: T) -> Annotation {
        Annotation {
            type_id: self.id,
            value: Arc::new(value),
        }
    }
}

/// A tagged value that adds metadata to a transaction, such as its time
/// stamp or the user event that caused it (`Annotation`).
#[derive(Clone)]
pub struct Annotation {
    type_id: usize,
    value: AnyValue,
}

impl Annotation {
    /// Whether this annotation has type `ty`.
    pub fn is<T>(&self, ty: AnnotationType<T>) -> bool {
        self.type_id == ty.id
    }

    /// This annotation's value, if it has type `ty`.
    pub fn value_of<T: 'static>(&self, ty: AnnotationType<T>) -> Option<&T> {
        if self.is(ty) {
            self.value.downcast_ref()
        } else {
            None
        }
    }
}

impl fmt::Debug for Annotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Annotation#{}", self.type_id)
    }
}

type ErasedMap = Box<dyn Fn(&AnyValue, &ChangeDesc) -> Option<AnyValue> + Send + Sync>;

pub(crate) struct EffectTypeInner {
    map: Option<ErasedMap>,
}

/// A type of [`StateEffect`] holding values of type `V`
/// (`StateEffectType`).
pub struct StateEffectType<V> {
    inner: Arc<EffectTypeInner>,
    _value: PhantomData<fn() -> V>,
}

impl<V> Clone for StateEffectType<V> {
    fn clone(&self) -> Self {
        StateEffectType {
            inner: self.inner.clone(),
            _value: PhantomData,
        }
    }
}

impl<V> fmt::Debug for StateEffectType<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StateEffectType({:p})", Arc::as_ptr(&self.inner))
    }
}

impl<V: Send + Sync + 'static> StateEffectType<V> {
    /// Define an effect type whose values are not mapped through changes
    /// (`StateEffect.define()`).
    pub fn define() -> Self {
        StateEffectType {
            inner: Arc::new(EffectTypeInner { map: None }),
            _value: PhantomData,
        }
    }

    /// Define an effect type whose values `map` maps through changes; it
    /// returns `None` when the mapping deletes the effect
    /// (`StateEffect.define({map})`).
    pub fn define_with_map(
        map: impl Fn(&V, &ChangeDesc) -> Option<V> + Send + Sync + 'static,
    ) -> Self {
        StateEffectType {
            inner: Arc::new(EffectTypeInner {
                map: Some(Box::new(move |value, mapping| {
                    let value = value.downcast_ref::<V>().expect("the effect's value type");
                    map(value, mapping).map(|v| Arc::new(v) as AnyValue)
                })),
            }),
            _value: PhantomData,
        }
    }

    /// An effect of this type (`of`).
    pub fn of(&self, value: V) -> StateEffect {
        StateEffect {
            ty: self.inner.clone(),
            value: Arc::new(value),
        }
    }
}

/// An effect attached to a transaction, often used to change custom
/// state fields (`StateEffect`).
#[derive(Clone)]
pub struct StateEffect {
    ty: Arc<EffectTypeInner>,
    value: AnyValue,
}

/// The effect types `StateEffect` defines.
pub mod effects {
    use super::*;

    /// Replaces the root extensions of the state, discarding appended
    /// ones but keeping the content of reconfigured compartments
    /// (`StateEffect.reconfigure`).
    pub static RECONFIGURE: LazyLock<StateEffectType<Extension>> =
        LazyLock::new(StateEffectType::define);

    /// Appends extensions to the top-level configuration
    /// (`StateEffect.appendConfig`).
    pub static APPEND_CONFIG: LazyLock<StateEffectType<Extension>> =
        LazyLock::new(StateEffectType::define);
}

impl StateEffect {
    /// Whether this effect has type `ty` (`is`).
    pub fn is<V>(&self, ty: &StateEffectType<V>) -> bool {
        Arc::ptr_eq(&self.ty, &ty.inner)
    }

    /// This effect's value, if it has type `ty` (`is` and `value`).
    pub fn value_of<V: 'static>(&self, ty: &StateEffectType<V>) -> Option<&V> {
        if self.is(ty) {
            self.value.downcast_ref()
        } else {
            None
        }
    }

    /// Map this effect through `mapping`; `None` when that deletes it
    /// (`map`).
    pub fn map(&self, mapping: &ChangeDesc) -> Option<StateEffect> {
        match &self.ty.map {
            None => Some(self.clone()),
            Some(map) => map(&self.value, mapping).map(|value| StateEffect {
                ty: self.ty.clone(),
                value,
            }),
        }
    }

    /// Map `effects` through `mapping`, dropping deleted ones
    /// (`mapEffects`).
    pub fn map_effects(effects: &[StateEffect], mapping: &ChangeDesc) -> Vec<StateEffect> {
        effects.iter().filter_map(|e| e.map(mapping)).collect()
    }

    /// An effect replacing the root extensions ([`effects::RECONFIGURE`]).
    pub fn reconfigure(extension: impl Into<Extension>) -> StateEffect {
        effects::RECONFIGURE.of(extension.into())
    }

    /// An effect appending extensions ([`effects::APPEND_CONFIG`]).
    pub fn append_config(extension: impl Into<Extension>) -> StateEffect {
        effects::APPEND_CONFIG.of(extension.into())
    }
}

impl fmt::Debug for StateEffect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StateEffect({:p})", Arc::as_ptr(&self.ty))
    }
}

/// Describes a transaction for [`EditorState::update`]
/// (`TransactionSpec`).
#[derive(Clone, Debug, Default)]
pub struct TransactionSpec {
    /// The document changes, in the coordinates of the start document.
    pub changes: Option<ChangeSpec>,
    /// An explicit new selection, in the coordinates of the new document.
    /// Upstream also accepts `{anchor, head}`; use
    /// [`EditorSelection::single`].
    pub selection: Option<EditorSelection>,
    /// Effects, with positions in the coordinates of the new document.
    pub effects: Vec<StateEffect>,
    /// Annotations.
    pub annotations: Vec<Annotation>,
    /// Shorthand for a [`Transaction::USER_EVENT`] annotation.
    pub user_event: Option<String>,
    /// Scroll the selection into view after the transaction.
    pub scroll_into_view: bool,
    /// `Some(false)` stops change and transaction filters from changing
    /// the transaction.
    pub filter: Option<bool>,
    /// Take `changes` to be in the coordinates of the document the specs
    /// before this one produce.
    pub sequential: bool,
}

/// A spec for [`EditorState::update_all`] or a transaction filter's
/// result: a [`TransactionSpec`] or a [`Transaction`], which upstream can
/// pass where a spec is expected.
#[derive(Clone, Debug)]
pub enum SpecItem {
    /// A transaction spec.
    Spec(TransactionSpec),
    /// An existing transaction, used as a spec.
    Transaction(Transaction),
}

impl From<TransactionSpec> for SpecItem {
    fn from(spec: TransactionSpec) -> SpecItem {
        SpecItem::Spec(spec)
    }
}

impl From<Transaction> for SpecItem {
    fn from(tr: Transaction) -> SpecItem {
        SpecItem::Transaction(tr)
    }
}

/// What a change filter decides about a transaction's changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeFilterResult {
    /// Keep the changes (`true`).
    Allow,
    /// Drop all the changes (`false`).
    Block,
    /// Drop the changes in these ranges, given as `from, to` pairs
    /// (an array of numbers).
    Suppress(Vec<usize>),
}

/// Effects and annotations a transaction extender adds.
#[derive(Clone, Debug, Default)]
pub struct TransactionExtension {
    /// Effects to add.
    pub effects: Vec<StateEffect>,
    /// Annotations to add.
    pub annotations: Vec<Annotation>,
}

/// A change filter ([`facets::CHANGE_FILTER`]).
pub type ChangeFilter = Callback<dyn Fn(&Transaction) -> ChangeFilterResult + Send + Sync>;
/// A transaction filter ([`facets::TRANSACTION_FILTER`]).
pub type TransactionFilter = Callback<dyn Fn(&Transaction) -> Vec<SpecItem> + Send + Sync>;
/// A transaction extender ([`facets::TRANSACTION_EXTENDER`]).
pub type TransactionExtender =
    Callback<dyn Fn(&Transaction) -> Option<TransactionExtension> + Send + Sync>;

impl ChangeFilter {
    /// Wrap a change filter function.
    pub fn new(f: impl Fn(&Transaction) -> ChangeFilterResult + Send + Sync + 'static) -> Self {
        Callback(Arc::new(f))
    }
}

impl TransactionFilter {
    /// Wrap a transaction filter function.
    pub fn new(f: impl Fn(&Transaction) -> Vec<SpecItem> + Send + Sync + 'static) -> Self {
        Callback(Arc::new(f))
    }
}

impl TransactionExtender {
    /// Wrap a transaction extender function.
    pub fn new(
        f: impl Fn(&Transaction) -> Option<TransactionExtension> + Send + Sync + 'static,
    ) -> Self {
        Callback(Arc::new(f))
    }
}

struct TransactionInner {
    start_state: EditorState,
    changes: ChangeSet,
    selection: Option<EditorSelection>,
    effects: Vec<StateEffect>,
    annotations: Vec<Annotation>,
    scroll_into_view: bool,
    doc: OnceLock<Text>,
    state: OnceLock<EditorState>,
}

/// A group of document changes, a selection change and other effects
/// (`Transaction`). Create one with [`EditorState::update`]; its new
/// state is computed on demand.
#[derive(Clone)]
pub struct Transaction(Arc<TransactionInner>);

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_millis() as f64)
}

impl Transaction {
    /// The time stamp of the transaction, in milliseconds since the Unix
    /// epoch, added to every transaction (`Transaction.time`).
    pub const TIME: AnnotationType<f64> = AnnotationType::reserved(0);
    /// The user interface event that caused the transaction, in a
    /// dot-separated format such as `"input.type"` or `"select.pointer"`
    /// (`Transaction.userEvent`).
    pub const USER_EVENT: AnnotationType<String> = AnnotationType::reserved(1);
    /// Whether the transaction should be added to the undo history
    /// (`Transaction.addToHistory`).
    pub const ADD_TO_HISTORY: AnnotationType<bool> = AnnotationType::reserved(2);
    /// Whether the transaction is another actor's change, as in
    /// collaborative editing (`Transaction.remote`).
    pub const REMOTE: AnnotationType<bool> = AnnotationType::reserved(3);

    pub(crate) fn create(
        start_state: &EditorState,
        changes: ChangeSet,
        selection: Option<EditorSelection>,
        effects: Vec<StateEffect>,
        mut annotations: Vec<Annotation>,
        scroll_into_view: bool,
    ) -> Result<Transaction, RangeError> {
        if let Some(selection) = &selection {
            check_selection(selection, changes.new_len())?;
        }
        if !annotations.iter().any(|a| a.is(Transaction::TIME)) {
            annotations.push(Transaction::TIME.of(now_ms()));
        }
        Ok(Transaction(Arc::new(TransactionInner {
            start_state: start_state.clone(),
            changes,
            selection,
            effects,
            annotations,
            scroll_into_view,
            doc: OnceLock::new(),
            state: OnceLock::new(),
        })))
    }

    /// The state the transaction starts from.
    pub fn start_state(&self) -> &EditorState {
        &self.0.start_state
    }

    /// The document changes.
    pub fn changes(&self) -> &ChangeSet {
        &self.0.changes
    }

    /// The selection the transaction sets explicitly, if any.
    pub fn selection(&self) -> Option<&EditorSelection> {
        self.0.selection.as_ref()
    }

    /// The effects.
    pub fn effects(&self) -> &[StateEffect] {
        &self.0.effects
    }

    /// The annotations.
    pub fn annotations(&self) -> &[Annotation] {
        &self.0.annotations
    }

    /// Whether the selection should be scrolled into view.
    pub fn scroll_into_view(&self) -> bool {
        self.0.scroll_into_view
    }

    /// The new document, computed without computing the whole new state
    /// (`newDoc`).
    pub fn new_doc(&self) -> &Text {
        self.0
            .doc
            .get_or_init(|| self.0.changes.apply(self.0.start_state.doc()))
    }

    /// The new selection: the explicit one, or the start selection mapped
    /// through the changes (`newSelection`).
    pub fn new_selection(&self) -> EditorSelection {
        match &self.0.selection {
            Some(selection) => selection.clone(),
            None => self
                .0
                .start_state
                .selection()
                .map(self.0.changes.desc(), -1),
        }
    }

    /// The new state, computed on demand and kept (`state`).
    ///
    /// # Panics
    ///
    /// When the transaction's reconfiguration effects use a compartment
    /// twice ("Duplicate use of compartment in extensions"), or a field
    /// or facet depends on itself.
    pub fn state(&self) -> &EditorState {
        if self.0.state.get().is_none() {
            self.0.start_state.apply_transaction(self);
        }
        self.0
            .state
            .get()
            .expect("applying the transaction sets its state")
    }

    pub(crate) fn set_state(&self, state: EditorState) {
        let _ = self.0.state.set(state);
    }

    /// The value of the annotation of type `ty`, if any (`annotation`).
    pub fn annotation<T: 'static>(&self, ty: AnnotationType<T>) -> Option<&T> {
        self.0.annotations.iter().find_map(|a| a.value_of(ty))
    }

    /// Whether the transaction changes the document (`docChanged`).
    pub fn doc_changed(&self) -> bool {
        !self.0.changes.is_empty()
    }

    /// Whether the transaction reconfigures the state (`reconfigured`).
    pub fn reconfigured(&self) -> bool {
        !self.0.start_state.same_config(self.state())
    }

    /// Whether the user event annotation is `event` or a more specific
    /// event under it, so `"select"` matches `"select.pointer"`
    /// (`isUserEvent`).
    pub fn is_user_event(&self, event: &str) -> bool {
        self.annotation(Transaction::USER_EVENT).is_some_and(|e| {
            e == event
                || e.len() > event.len()
                    && e.starts_with(event)
                    && e.as_bytes()[event.len()] == b'.'
        })
    }
}

impl fmt::Debug for Transaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Transaction")
            .field("changes", &self.0.changes)
            .field("selection", &self.0.selection)
            .field("effects", &self.0.effects)
            .field("annotations", &self.0.annotations)
            .field("scroll_into_view", &self.0.scroll_into_view)
            .finish()
    }
}

fn join_ranges(a: &[usize], b: &[usize]) -> Vec<usize> {
    let mut result: Vec<usize> = Vec::new();
    let (mut ia, mut ib) = (0, 0);
    loop {
        let (from, to);
        if ia < a.len() && (ib == b.len() || b[ib] >= a[ia]) {
            from = a[ia];
            to = a[ia + 1];
            ia += 2;
        } else if ib < b.len() {
            from = b[ib];
            to = b[ib + 1];
            ib += 2;
        } else {
            return result;
        }
        match result.last_mut() {
            Some(last) if *last >= from => {
                if *last < to {
                    *last = to;
                }
            }
            _ => result.extend([from, to]),
        }
    }
}

struct ResolvedSpec {
    changes: ChangeSet,
    selection: Option<EditorSelection>,
    effects: Vec<StateEffect>,
    annotations: Vec<Annotation>,
    scroll_into_view: bool,
}

impl ResolvedSpec {
    fn of(tr: &Transaction) -> ResolvedSpec {
        ResolvedSpec {
            changes: tr.0.changes.clone(),
            selection: tr.0.selection.clone(),
            effects: tr.0.effects.clone(),
            annotations: tr.0.annotations.clone(),
            scroll_into_view: tr.0.scroll_into_view,
        }
    }
}

fn merge_transaction(a: ResolvedSpec, b: ResolvedSpec, sequential: bool) -> ResolvedSpec {
    let (map_for_a, map_for_b, changes) = if sequential {
        (
            b.changes.desc().clone(),
            ChangeSet::empty(b.changes.len()).desc().clone(),
            a.changes.compose(&b.changes),
        )
    } else {
        let map_for_a = b.changes.map(a.changes.desc(), false);
        let map_for_b = a.changes.desc().map_desc(b.changes.desc(), true);
        let changes = a.changes.compose(&map_for_a);
        (map_for_a.desc().clone(), map_for_b, changes)
    };
    let selection = match b.selection {
        Some(selection) => Some(selection.map(&map_for_b, -1)),
        None => a.selection.map(|s| s.map(&map_for_a, -1)),
    };
    let mut effects = StateEffect::map_effects(&a.effects, &map_for_a);
    effects.extend(StateEffect::map_effects(&b.effects, &map_for_b));
    let annotations = if a.annotations.is_empty() {
        b.annotations
    } else {
        let mut annotations = a.annotations;
        annotations.extend(b.annotations);
        annotations
    };
    ResolvedSpec {
        changes,
        selection,
        effects,
        annotations,
        scroll_into_view: a.scroll_into_view || b.scroll_into_view,
    }
}

fn resolve_transaction_inner(
    state: &EditorState,
    spec: &SpecItem,
    doc_size: usize,
) -> Result<ResolvedSpec, RangeError> {
    let spec = match spec {
        SpecItem::Transaction(tr) => return Ok(ResolvedSpec::of(tr)),
        SpecItem::Spec(spec) => spec,
    };
    let mut annotations = spec.annotations.clone();
    if let Some(event) = &spec.user_event {
        annotations.push(Transaction::USER_EVENT.of(event.clone()));
    }
    let changes = match &spec.changes {
        Some(ChangeSpec::Set(set)) => set.clone(),
        Some(changes) => ChangeSet::of(changes.clone(), doc_size, state.line_separator())?,
        None => ChangeSet::of(
            ChangeSpec::List(Vec::new()),
            doc_size,
            state.line_separator(),
        )?,
    };
    Ok(ResolvedSpec {
        changes,
        selection: spec.selection.clone(),
        effects: spec.effects.clone(),
        annotations,
        scroll_into_view: spec.scroll_into_view,
    })
}

fn filter_disabled(spec: &SpecItem) -> bool {
    matches!(
        spec,
        SpecItem::Spec(TransactionSpec {
            filter: Some(false),
            ..
        })
    )
}

fn is_sequential(spec: &SpecItem) -> bool {
    matches!(
        spec,
        SpecItem::Spec(TransactionSpec {
            sequential: true,
            ..
        })
    )
}

pub(crate) fn resolve_transaction(
    state: &EditorState,
    specs: &[SpecItem],
    mut filter: bool,
) -> Result<Transaction, RangeError> {
    let empty = SpecItem::Spec(TransactionSpec::default());
    let mut s =
        resolve_transaction_inner(state, specs.first().unwrap_or(&empty), state.doc().len())?;
    if specs.first().is_some_and(filter_disabled) {
        filter = false;
    }
    for spec in specs.iter().skip(1) {
        if filter_disabled(spec) {
            filter = false;
        }
        let seq = is_sequential(spec);
        let doc_size = if seq {
            s.changes.new_len()
        } else {
            state.doc().len()
        };
        s = merge_transaction(s, resolve_transaction_inner(state, spec, doc_size)?, seq);
    }
    let tr = Transaction::create(
        state,
        s.changes,
        s.selection,
        s.effects,
        s.annotations,
        s.scroll_into_view,
    )?;
    let tr = if filter { filter_transaction(tr)? } else { tr };
    extend_transaction(tr)
}

fn filter_transaction(mut tr: Transaction) -> Result<Transaction, RangeError> {
    let state = tr.start_state().clone();

    // Change filters
    let mut result = ChangeFilterResult::Allow;
    for filter in state.facet(&facets::CHANGE_FILTER).iter() {
        match filter(&tr) {
            ChangeFilterResult::Allow => {}
            ChangeFilterResult::Block => {
                result = ChangeFilterResult::Block;
                break;
            }
            ChangeFilterResult::Suppress(ranges) => {
                result = match result {
                    ChangeFilterResult::Suppress(current) => {
                        ChangeFilterResult::Suppress(join_ranges(&current, &ranges))
                    }
                    _ => ChangeFilterResult::Suppress(ranges),
                };
            }
        }
    }
    if result != ChangeFilterResult::Allow {
        let (changes, back) = match result {
            ChangeFilterResult::Suppress(ranges) => {
                let (changes, filtered) = tr.changes().filter(&ranges);
                let back = filtered.map_desc(changes.desc(), false).inverted_desc();
                (changes, back)
            }
            _ => (
                ChangeSet::empty(state.doc().len()),
                tr.changes().desc().inverted_desc(),
            ),
        };
        tr = Transaction::create(
            &state,
            changes,
            tr.selection().map(|s| s.map(&back, -1)),
            StateEffect::map_effects(tr.effects(), &back),
            tr.annotations().to_vec(),
            tr.scroll_into_view(),
        )?;
    }

    // Transaction filters
    let filters = state.facet(&facets::TRANSACTION_FILTER);
    for filter in filters.iter().rev() {
        let mut filtered = filter(&tr);
        tr = match filtered.as_slice() {
            [SpecItem::Transaction(_)] => match filtered.pop() {
                Some(SpecItem::Transaction(next)) => next,
                _ => unreachable!(),
            },
            _ => resolve_transaction(&state, &filtered, false)?,
        };
    }
    Ok(tr)
}

fn extend_transaction(tr: Transaction) -> Result<Transaction, RangeError> {
    let state = tr.start_state().clone();
    let extenders = state.facet(&facets::TRANSACTION_EXTENDER);
    let mut spec: Option<ResolvedSpec> = None;
    for extender in extenders.iter().rev() {
        let Some(extension) = extender(&tr) else {
            continue;
        };
        if extension.effects.is_empty() && extension.annotations.is_empty() {
            continue;
        }
        let extension = SpecItem::Spec(TransactionSpec {
            effects: extension.effects,
            annotations: extension.annotations,
            ..TransactionSpec::default()
        });
        let current = spec.take().unwrap_or_else(|| ResolvedSpec::of(&tr));
        spec = Some(merge_transaction(
            current,
            resolve_transaction_inner(&state, &extension, tr.changes().new_len())?,
            true,
        ));
    }
    match spec {
        None => Ok(tr),
        Some(spec) => Transaction::create(
            &state,
            tr.changes().clone(),
            tr.selection().cloned(),
            spec.effects,
            spec.annotations,
            spec.scroll_into_view,
        ),
    }
}
