//! Port of `facet.ts`: facets, state fields, extensions, precedence,
//! compartments and the configuration they resolve to.
//!
//! CodeMirror keeps facet inputs, facet outputs and field values as plain
//! JavaScript values in one array per state. This port keeps them as
//! [`AnyValue`]s (`Arc<dyn Any>`), and the typed handles ([`Facet`],
//! [`FacetReader`], [`StateField`]) cast them back. Where CodeMirror
//! compares values with `===`, the port uses `PartialEq`; functions, which
//! JavaScript compares by identity, are wrapped in [`Callback`], whose
//! `PartialEq` compares pointers. The identity checks CodeMirror makes on
//! extension objects, providers, facet outputs and field values use
//! pointers too.

use std::any::Any;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::marker::PhantomData;
use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, OnceLock};

use crate::state::EditorState;
use crate::transaction::{StateEffect, StateEffectType, Transaction};
use crate::RangeError;

/// A type-erased value: a facet input or output, a field value, or an
/// effect or annotation value.
pub type AnyValue = Arc<dyn Any + Send + Sync>;

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn next_id() -> usize {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

pub(crate) fn same_value(a: &AnyValue, b: &AnyValue) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}

fn cast<T: 'static>(value: &AnyValue) -> &T {
    value
        .downcast_ref::<T>()
        .expect("a value of the type its slot was defined with")
}

/// A function value, compared by identity as JavaScript compares
/// functions. Facets whose inputs are functions (change filters,
/// transaction filters, language data providers) take `Callback`s.
pub struct Callback<F: ?Sized>(pub Arc<F>);

impl<F: ?Sized> Clone for Callback<F> {
    fn clone(&self) -> Self {
        Callback(self.0.clone())
    }
}

impl<F: ?Sized> PartialEq for Callback<F> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::addr_eq(Arc::as_ptr(&self.0), Arc::as_ptr(&other.0))
    }
}

impl<F: ?Sized> Deref for Callback<F> {
    type Target = F;
    fn deref(&self) -> &F {
        &self.0
    }
}

impl<F: ?Sized> fmt::Debug for Callback<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Callback({:p})", Arc::as_ptr(&self.0) as *const ())
    }
}

// --- Facets -------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ProviderKind {
    Static,
    Single,
    Multi,
}

type ErasedCombine = Box<dyn Fn(&[(ProviderKind, &AnyValue)]) -> AnyValue + Send + Sync>;
type ErasedCompare = Box<dyn Fn(&AnyValue, &AnyValue) -> bool + Send + Sync>;
type Combine<I, O> = Box<dyn Fn(&[I]) -> O + Send + Sync>;
type Compare<T> = Box<dyn Fn(&T, &T) -> bool + Send + Sync>;
type CompareInput<I> = Arc<dyn Fn(&I, &I) -> bool + Send + Sync>;
type Enables<I, O> = Box<dyn FnOnce(&Facet<I, O>) -> Extension + Send>;

pub(crate) struct FacetInner {
    pub(crate) id: usize,
    combine: ErasedCombine,
    compare: ErasedCompare,
    // An `Arc<dyn Fn(&I, &I) -> bool + Send + Sync>`, cast back by the
    // typed methods that build providers.
    compare_input: Box<dyn Any + Send + Sync>,
    is_static: bool,
    pub(crate) default: AnyValue,
    pub(crate) extensions: OnceLock<Extension>,
}

impl FacetInner {
    pub(crate) fn combine(&self, values: &[(ProviderKind, &AnyValue)]) -> AnyValue {
        (self.combine)(values)
    }

    pub(crate) fn compare(&self, a: &AnyValue, b: &AnyValue) -> bool {
        (self.compare)(a, b)
    }
}

/// How a facet combines and compares its values (`FacetConfig`).
pub struct FacetConfig<I, O> {
    combine: Combine<I, O>,
    compare: Compare<O>,
    compare_input: CompareInput<I>,
    is_static: bool,
    enables: Option<Enables<I, O>>,
}

impl<I: PartialEq + 'static, O: PartialEq + 'static> FacetConfig<I, O> {
    /// A facet whose output is `combine` applied to its inputs, compared
    /// with `==`. `combine` is called once with no inputs, when the facet
    /// is defined, for the facet's default value.
    pub fn new(combine: impl Fn(&[I]) -> O + Send + Sync + 'static) -> Self {
        FacetConfig::with_compare(combine, |a: &O, b: &O| a == b)
    }
}

impl<I: PartialEq + 'static, O: 'static> FacetConfig<I, O> {
    /// Like [`new`](Self::new), for outputs compared with `compare`.
    pub fn with_compare(
        combine: impl Fn(&[I]) -> O + Send + Sync + 'static,
        compare: impl Fn(&O, &O) -> bool + Send + Sync + 'static,
    ) -> Self {
        FacetConfig {
            combine: Box::new(combine),
            compare: Box::new(compare),
            compare_input: Arc::new(|a: &I, b: &I| a == b),
            is_static: false,
            enables: None,
        }
    }

    /// Compare inputs with `compare` to avoid recomputing the output when
    /// no input changed (by default `==`).
    pub fn compare_input(
        mut self,
        compare: impl Fn(&I, &I) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.compare_input = Arc::new(compare);
        self
    }

    /// Forbid dynamic inputs to the facet (`static`).
    pub fn static_facet(mut self) -> Self {
        self.is_static = true;
        self
    }

    /// Add `extension` to any state where the facet is provided.
    pub fn enables(self, extension: impl Into<Extension>) -> Self {
        let extension = extension.into();
        self.enables_with(move |_| extension)
    }

    /// Add the extension `f` returns for the facet to any state where the
    /// facet is provided.
    pub fn enables_with(
        mut self,
        f: impl FnOnce(&Facet<I, O>) -> Extension + Send + 'static,
    ) -> Self {
        self.enables = Some(Box::new(f));
        self
    }
}

impl<I: Clone + PartialEq + 'static> FacetConfig<I, Vec<I>> {
    /// A facet whose output is the array of its inputs, the default
    /// upstream.
    pub fn array() -> Self {
        FacetConfig::new(|values: &[I]| values.to_vec())
    }
}

/// Reads a facet's output from a state (`FacetReader`), without the
/// ability to provide values for it. A [`Facet`] dereferences to its
/// reader, so `state.facet(&facet)` works for both.
pub struct FacetReader<O> {
    pub(crate) inner: Arc<FacetInner>,
    _output: PhantomData<fn() -> O>,
}

impl<O> Clone for FacetReader<O> {
    fn clone(&self) -> Self {
        FacetReader {
            inner: self.inner.clone(),
            _output: PhantomData,
        }
    }
}

impl<O> fmt::Debug for FacetReader<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Facet#{}", self.inner.id)
    }
}

impl<O: 'static> FacetReader<O> {
    /// The output when the facet has no inputs.
    pub fn default_value(&self) -> &O {
        cast(&self.inner.default)
    }
}

/// A labeled value associated with an editor state. It takes inputs from
/// any number of extensions and combines them into one output value.
pub struct Facet<I, O = Vec<I>> {
    reader: FacetReader<O>,
    _input: PhantomData<fn(I)>,
}

impl<I, O> Clone for Facet<I, O> {
    fn clone(&self) -> Self {
        Facet {
            reader: self.reader.clone(),
            _input: PhantomData,
        }
    }
}

impl<I, O> Deref for Facet<I, O> {
    type Target = FacetReader<O>;
    fn deref(&self) -> &FacetReader<O> {
        &self.reader
    }
}

impl<I, O> fmt::Debug for Facet<I, O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.reader, f)
    }
}

impl<I: Clone + PartialEq + Send + Sync + 'static> Facet<I, Vec<I>> {
    /// Define a facet whose output is the array of its inputs
    /// (`Facet.define()` without a config).
    pub fn define() -> Self {
        Facet::define_with(FacetConfig::array())
    }
}

impl<I, O> Facet<I, O>
where
    I: Clone + Send + Sync + 'static,
    O: Send + Sync + 'static,
{
    /// Define a facet (`Facet.define`).
    pub fn define_with(config: FacetConfig<I, O>) -> Self {
        let FacetConfig {
            combine,
            compare,
            compare_input,
            is_static,
            enables,
        } = config;
        let default: AnyValue = Arc::new(combine(&[]));
        let combine: ErasedCombine = Box::new(move |values| {
            let mut inputs: Vec<I> = Vec::with_capacity(values.len());
            for (kind, value) in values {
                if *kind == ProviderKind::Multi {
                    inputs.extend(cast::<Vec<I>>(value).iter().cloned());
                } else {
                    inputs.push(cast::<I>(value).clone());
                }
            }
            Arc::new(combine(&inputs))
        });
        let compare: ErasedCompare = Box::new(move |a, b| compare(cast(a), cast(b)));
        let facet = Facet {
            reader: FacetReader {
                inner: Arc::new(FacetInner {
                    id: next_id(),
                    combine,
                    compare,
                    compare_input: Box::new(compare_input),
                    is_static,
                    default,
                    extensions: OnceLock::new(),
                }),
                _output: PhantomData,
            },
            _input: PhantomData,
        };
        if let Some(enables) = enables {
            let extension = enables(&facet);
            let _ = facet.inner.extensions.set(extension);
        }
        facet
    }

    /// The reader for this facet (`reader`).
    pub fn reader(&self) -> FacetReader<O> {
        self.reader.clone()
    }

    fn compare_input_fn(&self) -> CompareInput<I> {
        self.inner
            .compare_input
            .downcast_ref::<CompareInput<I>>()
            .expect("the facet's input type")
            .clone()
    }

    /// An extension that adds `value` to this facet (`of`).
    pub fn of(&self, value: I) -> Extension {
        Extension::provider(ProviderInner {
            id: next_id(),
            dependencies: Vec::new(),
            facet: self.inner.clone(),
            kind: ProviderKind::Static,
            value: ProviderValue::Static(Arc::new(value)),
        })
    }

    /// An extension that computes a value for the facet from a state
    /// (`compute`). `get` runs again for a new state only when one of
    /// `deps` changed.
    ///
    /// # Panics
    ///
    /// For a static facet: "Can't compute a static facet".
    pub fn compute(
        &self,
        deps: impl IntoIterator<Item = Dep>,
        get: impl Fn(&EditorState) -> I + Send + Sync + 'static,
    ) -> Extension {
        let compare = self.compare_input_fn();
        self.dynamic(
            deps,
            ProviderKind::Single,
            Box::new(move |state| Arc::new(get(state))),
            Box::new(move |a, b| compare(cast(a), cast(b))),
        )
    }

    /// An extension that computes zero or more values for the facet from
    /// a state (`computeN`).
    ///
    /// # Panics
    ///
    /// For a static facet: "Can't compute a static facet".
    pub fn compute_n(
        &self,
        deps: impl IntoIterator<Item = Dep>,
        get: impl Fn(&EditorState) -> Vec<I> + Send + Sync + 'static,
    ) -> Extension {
        let compare = self.compare_input_fn();
        self.dynamic(
            deps,
            ProviderKind::Multi,
            Box::new(move |state| Arc::new(get(state))),
            Box::new(move |a, b| {
                let (a, b) = (cast::<Vec<I>>(a), cast::<Vec<I>>(b));
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| compare(a, b))
            }),
        )
    }

    fn dynamic(
        &self,
        deps: impl IntoIterator<Item = Dep>,
        kind: ProviderKind,
        get: ErasedGet,
        same: ErasedCompare,
    ) -> Extension {
        if self.inner.is_static {
            panic!("Can't compute a static facet");
        }
        Extension::provider(ProviderInner {
            id: next_id(),
            dependencies: deps.into_iter().collect(),
            facet: self.inner.clone(),
            kind,
            value: ProviderValue::Dynamic { get, same },
        })
    }

    /// An extension that takes the facet's input from `field` (`from`).
    pub fn from(&self, field: &StateField<I>) -> Extension
    where
        I: PartialEq,
    {
        self.from_with(field, I::clone)
    }

    /// An extension that takes the facet's input from `field` through
    /// `get` (`from` with a getter).
    pub fn from_with<T: Send + Sync + 'static>(
        &self,
        field: &StateField<T>,
        get: impl Fn(&T) -> I + Send + Sync + 'static,
    ) -> Extension {
        let reader = field.clone();
        self.compute([Dep::from(field)], move |state| get(state.field(&reader)))
    }
}

/// A part of the state a computed facet value depends on (`Slot`).
#[derive(Clone)]
pub struct Dep(pub(crate) DepKind);

#[derive(Clone)]
pub(crate) enum DepKind {
    Doc,
    Selection,
    Facet(Arc<FacetInner>),
    Field(Arc<FieldInner>),
}

impl Dep {
    /// The document (`"doc"`).
    pub const DOC: Dep = Dep(DepKind::Doc);
    /// The selection (`"selection"`).
    pub const SELECTION: Dep = Dep(DepKind::Selection);
}

impl<O> From<&FacetReader<O>> for Dep {
    fn from(facet: &FacetReader<O>) -> Dep {
        Dep(DepKind::Facet(facet.inner.clone()))
    }
}

impl<I, O> From<&Facet<I, O>> for Dep {
    fn from(facet: &Facet<I, O>) -> Dep {
        Dep(DepKind::Facet(facet.inner.clone()))
    }
}

impl<V> From<&StateField<V>> for Dep {
    fn from(field: &StateField<V>) -> Dep {
        Dep(DepKind::Field(field.inner.clone()))
    }
}

type ErasedGet = Box<dyn Fn(&EditorState) -> AnyValue + Send + Sync>;

pub(crate) enum ProviderValue {
    Static(AnyValue),
    Dynamic { get: ErasedGet, same: ErasedCompare },
}

pub(crate) struct ProviderInner {
    pub(crate) id: usize,
    dependencies: Vec<Dep>,
    pub(crate) facet: Arc<FacetInner>,
    pub(crate) kind: ProviderKind,
    value: ProviderValue,
}

impl ProviderInner {
    fn static_value(&self) -> &AnyValue {
        match &self.value {
            ProviderValue::Static(value) => value,
            ProviderValue::Dynamic { .. } => unreachable!("a static provider"),
        }
    }

    fn get(&self, state: &EditorState) -> AnyValue {
        match &self.value {
            ProviderValue::Dynamic { get, .. } => get(state),
            ProviderValue::Static(_) => unreachable!("a dynamic provider"),
        }
    }

    fn same(&self, a: &AnyValue, b: &AnyValue) -> bool {
        match &self.value {
            ProviderValue::Dynamic { same, .. } => same(a, b),
            ProviderValue::Static(_) => unreachable!("a dynamic provider"),
        }
    }
}

fn same_providers(a: &[Arc<ProviderInner>], b: &[Arc<ProviderInner>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| Arc::ptr_eq(a, b))
}

// --- State fields -------------------------------------------------------

type ErasedCreate = Arc<dyn Fn(&EditorState) -> AnyValue + Send + Sync>;
type ErasedUpdate = Box<dyn Fn(&AnyValue, &Transaction) -> AnyValue + Send + Sync>;
type Update<V> = Box<dyn Fn(&V, &Transaction) -> V + Send + Sync>;
type Provide<V> = Box<dyn FnOnce(&StateField<V>) -> Extension>;

pub(crate) struct FieldInner {
    pub(crate) id: usize,
    create: ErasedCreate,
    update: ErasedUpdate,
    compare: ErasedCompare,
    provides: OnceLock<Extension>,
}

/// How a [`StateField`] creates, updates and compares its value
/// (`StateFieldSpec`).
pub struct StateFieldSpec<V> {
    create: Box<dyn Fn(&EditorState) -> V + Send + Sync>,
    update: Update<V>,
    compare: Compare<V>,
    provide: Option<Provide<V>>,
}

impl<V: PartialEq + 'static> StateFieldSpec<V> {
    /// A field created by `create` and updated by `update`, whose values
    /// are compared with `==`.
    pub fn new(
        create: impl Fn(&EditorState) -> V + Send + Sync + 'static,
        update: impl Fn(&V, &Transaction) -> V + Send + Sync + 'static,
    ) -> Self {
        StateFieldSpec::with_compare(create, update, |a: &V, b: &V| a == b)
    }
}

impl<V: 'static> StateFieldSpec<V> {
    /// Like [`new`](Self::new), for values compared with `compare`.
    pub fn with_compare(
        create: impl Fn(&EditorState) -> V + Send + Sync + 'static,
        update: impl Fn(&V, &Transaction) -> V + Send + Sync + 'static,
        compare: impl Fn(&V, &V) -> bool + Send + Sync + 'static,
    ) -> Self {
        StateFieldSpec {
            create: Box::new(create),
            update: Box::new(update),
            compare: Box::new(compare),
            provide: None,
        }
    }

    /// Provide extensions based on the field, usually facet inputs taken
    /// from it (`provide`). `f` runs once, when the field is defined.
    pub fn provide(mut self, f: impl FnOnce(&StateField<V>) -> Extension + 'static) -> Self {
        self.provide = Some(Box::new(f));
        self
    }
}

/// Extra information stored in an editor state, kept in sync with the
/// rest of it (`StateField`).
pub struct StateField<V> {
    pub(crate) inner: Arc<FieldInner>,
    _value: PhantomData<fn() -> V>,
}

impl<V> Clone for StateField<V> {
    fn clone(&self) -> Self {
        StateField {
            inner: self.inner.clone(),
            _value: PhantomData,
        }
    }
}

impl<V> fmt::Debug for StateField<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StateField#{}", self.inner.id)
    }
}

impl<V: Send + Sync + 'static> StateField<V> {
    /// Define a state field (`StateField.define`).
    pub fn define(spec: StateFieldSpec<V>) -> Self {
        let StateFieldSpec {
            create,
            update,
            compare,
            provide,
        } = spec;
        let field = StateField {
            inner: Arc::new(FieldInner {
                id: next_id(),
                create: Arc::new(move |state| Arc::new(create(state))),
                update: Box::new(move |value, tr| Arc::new(update(cast(value), tr))),
                compare: Box::new(move |a, b| compare(cast(a), cast(b))),
                provides: OnceLock::new(),
            }),
            _value: PhantomData,
        };
        if let Some(provide) = provide {
            let extension = provide(&field);
            let _ = field.inner.provides.set(extension);
        }
        field
    }

    /// An extension that enables this field and creates its first value
    /// with `create` instead of the spec's function (`init`).
    pub fn init(&self, create: impl Fn(&EditorState) -> V + Send + Sync + 'static) -> Extension {
        let init = InitField(Arc::new(InitInner {
            field: self.inner.clone(),
            create: Arc::new(move |state| Arc::new(create(state))),
        }));
        Extension::from(vec![self.into(), INIT_FIELD.of(init)])
    }
}

struct InitInner {
    field: Arc<FieldInner>,
    create: ErasedCreate,
}

// Compared by identity, as upstream compares the `{field, create}`
// objects.
#[derive(Clone)]
struct InitField(Arc<InitInner>);

impl PartialEq for InitField {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

static INIT_FIELD: LazyLock<Facet<InitField>> =
    LazyLock::new(|| Facet::define_with(FacetConfig::array().static_facet()));

impl FieldInner {
    fn init_for(&self, state: &EditorState) -> Option<InitField> {
        state
            .facet(&INIT_FIELD)
            .iter()
            .find(|init| std::ptr::eq(Arc::as_ptr(&init.0.field), self))
            .cloned()
    }

    fn create(&self, state: &EditorState) -> AnyValue {
        match self.init_for(state) {
            Some(init) => (init.0.create)(state),
            None => (self.create)(state),
        }
    }
}

// --- Extensions ---------------------------------------------------------

/// Configuration and behaviour attached to a state: facet inputs, state
/// fields, and lists of extensions, possibly with a precedence or in a
/// compartment (`Extension`). Nested lists are flattened when a state is
/// configured, and an extension value that appears more than once is
/// included once.
///
/// Upstream accepts any object with an `extension` property; here such
/// types implement `Into<Extension>`.
#[derive(Clone)]
pub struct Extension(ExtKind);

#[derive(Clone)]
enum ExtKind {
    List(Arc<Vec<Extension>>),
    Provider(Arc<ProviderInner>),
    Field(Arc<FieldInner>),
    Prec(Arc<(Extension, usize)>),
    Compartment(Arc<(Compartment, Extension)>),
}

impl Extension {
    fn provider(provider: ProviderInner) -> Extension {
        Extension(ExtKind::Provider(Arc::new(provider)))
    }

    /// No extensions (`[]`).
    pub fn none() -> Extension {
        Extension::from(Vec::new())
    }

    // The identity CodeMirror uses to deduplicate extension values.
    fn key(&self) -> *const () {
        match &self.0 {
            ExtKind::List(a) => Arc::as_ptr(a) as *const (),
            ExtKind::Provider(a) => Arc::as_ptr(a) as *const (),
            ExtKind::Field(a) => Arc::as_ptr(a) as *const (),
            ExtKind::Prec(a) => Arc::as_ptr(a) as *const (),
            ExtKind::Compartment(a) => Arc::as_ptr(a) as *const (),
        }
    }

    /// Whether `self` and `other` are the same extension value, as
    /// JavaScript's `===` compares them.
    pub fn ptr_eq(&self, other: &Extension) -> bool {
        self.key() == other.key()
    }

    // The elements of a list, or the extension itself (`asArray`).
    pub(crate) fn as_list(&self) -> Vec<Extension> {
        match &self.0 {
            ExtKind::List(list) => list.as_ref().clone(),
            _ => vec![self.clone()],
        }
    }
}

impl Default for Extension {
    fn default() -> Self {
        Extension::none()
    }
}

impl fmt::Debug for Extension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            ExtKind::List(list) => f.debug_list().entries(list.iter()).finish(),
            ExtKind::Provider(p) => write!(f, "Provider#{}(Facet#{})", p.id, p.facet.id),
            ExtKind::Field(field) => write!(f, "StateField#{}", field.id),
            ExtKind::Prec(p) => write!(f, "Prec({}, {:?})", p.1, p.0),
            ExtKind::Compartment(c) => write!(f, "Compartment#{}({:?})", c.0.id, c.1),
        }
    }
}

impl From<Vec<Extension>> for Extension {
    fn from(list: Vec<Extension>) -> Extension {
        Extension(ExtKind::List(Arc::new(list)))
    }
}

impl<const N: usize> From<[Extension; N]> for Extension {
    fn from(list: [Extension; N]) -> Extension {
        Extension::from(Vec::from(list))
    }
}

impl<V> From<&StateField<V>> for Extension {
    fn from(field: &StateField<V>) -> Extension {
        Extension(ExtKind::Field(field.inner.clone()))
    }
}

impl<V> From<StateField<V>> for Extension {
    fn from(field: StateField<V>) -> Extension {
        Extension(ExtKind::Field(field.inner))
    }
}

const PREC_HIGHEST: usize = 0;
const PREC_HIGH: usize = 1;
const PREC_DEFAULT: usize = 2;
const PREC_LOW: usize = 3;
const PREC_LOWEST: usize = 4;

/// Precedence levels for extensions (`Prec`). Extensions are ordered by
/// precedence first and by their position in the flattened configuration
/// second. An extension without a precedence takes that of its nearest
/// parent that has one, or the default.
pub struct Prec;

impl Prec {
    fn wrap(extension: impl Into<Extension>, prec: usize) -> Extension {
        Extension(ExtKind::Prec(Arc::new((extension.into(), prec))))
    }

    /// The highest precedence, for extensions that should come first.
    pub fn highest(extension: impl Into<Extension>) -> Extension {
        Prec::wrap(extension, PREC_HIGHEST)
    }

    /// A higher-than-default precedence.
    pub fn high(extension: impl Into<Extension>) -> Extension {
        Prec::wrap(extension, PREC_HIGH)
    }

    /// The default precedence.
    pub fn default(extension: impl Into<Extension>) -> Extension {
        Prec::wrap(extension, PREC_DEFAULT)
    }

    /// A lower-than-default precedence.
    pub fn low(extension: impl Into<Extension>) -> Extension {
        Prec::wrap(extension, PREC_LOW)
    }

    /// The lowest precedence, for extensions that should come last.
    pub fn lowest(extension: impl Into<Extension>) -> Extension {
        Prec::wrap(extension, PREC_LOWEST)
    }
}

/// A part of the configuration that transactions can replace
/// (`Compartment`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Compartment {
    id: usize,
}

pub(crate) static COMPARTMENT_RECONFIGURE: LazyLock<StateEffectType<(Compartment, Extension)>> =
    LazyLock::new(StateEffectType::define);

impl Compartment {
    /// A new compartment (`new Compartment`).
    #[allow(clippy::new_without_default)]
    pub fn new() -> Compartment {
        Compartment { id: next_id() }
    }

    /// An extension holding `extension` in this compartment (`of`).
    pub fn of(&self, extension: impl Into<Extension>) -> Extension {
        Extension(ExtKind::Compartment(Arc::new((*self, extension.into()))))
    }

    /// An effect that replaces this compartment's content
    /// (`reconfigure`).
    pub fn reconfigure(&self, content: impl Into<Extension>) -> StateEffect {
        COMPARTMENT_RECONFIGURE.of((*self, content.into()))
    }

    /// The compartment's content in `state`, if it is present (`get`).
    pub fn get<'a>(&self, state: &'a EditorState) -> Option<&'a Extension> {
        state.config().compartments.get(self)
    }
}

// --- Configuration ------------------------------------------------------

pub(crate) const STATUS_UNRESOLVED: u8 = 0;
pub(crate) const STATUS_CHANGED: u8 = 1;
pub(crate) const STATUS_COMPUTED: u8 = 2;
pub(crate) const STATUS_COMPUTING: u8 = 4;

/// How a dynamic slot's value is produced while a state is built.
pub(crate) enum DynamicSlot {
    Field {
        field: Arc<FieldInner>,
    },
    Provider {
        provider: Arc<ProviderInner>,
        dep_doc: bool,
        dep_sel: bool,
        dep_addrs: Vec<usize>,
    },
    Facet {
        facet: Arc<FacetInner>,
        providers: Vec<Arc<ProviderInner>>,
        provider_addrs: Vec<usize>,
        dynamic: Vec<usize>,
    },
}

/// A slot's new value, and whether it changed.
pub(crate) type SlotResult = (AnyValue, bool);

fn ensure_all(state: &EditorState, addrs: &[usize]) -> bool {
    let mut changed = false;
    for &addr in addrs {
        if state.ensure_addr(addr) & STATUS_CHANGED != 0 {
            changed = true;
        }
    }
    changed
}

fn facet_value(
    state: &EditorState,
    facet: &FacetInner,
    providers: &[Arc<ProviderInner>],
    provider_addrs: &[usize],
) -> AnyValue {
    let values: Vec<(ProviderKind, &AnyValue)> = providers
        .iter()
        .zip(provider_addrs)
        .map(|(p, &addr)| (p.kind, state.get_addr(addr)))
        .collect();
    facet.combine(&values)
}

impl DynamicSlot {
    pub(crate) fn create(&self, state: &EditorState) -> SlotResult {
        match self {
            DynamicSlot::Field { field } => (field.create(state), true),
            DynamicSlot::Provider { provider, .. } => (provider.get(state), true),
            DynamicSlot::Facet {
                facet,
                providers,
                provider_addrs,
                ..
            } => {
                for &addr in provider_addrs {
                    state.ensure_addr(addr);
                }
                (facet_value(state, facet, providers, provider_addrs), true)
            }
        }
    }

    pub(crate) fn update(
        &self,
        state: &EditorState,
        old: &AnyValue,
        tr: &Transaction,
    ) -> SlotResult {
        match self {
            DynamicSlot::Field { field } => {
                let value = (field.update)(old, tr);
                if (field.compare)(old, &value) {
                    (old.clone(), false)
                } else {
                    (value, true)
                }
            }
            DynamicSlot::Provider {
                provider,
                dep_doc,
                dep_sel,
                dep_addrs,
            } => {
                if (*dep_doc && tr.doc_changed())
                    || (*dep_sel && (tr.doc_changed() || tr.selection().is_some()))
                    || ensure_all(state, dep_addrs)
                {
                    let value = provider.get(state);
                    if !provider.same(&value, old) {
                        return (value, true);
                    }
                }
                (old.clone(), false)
            }
            DynamicSlot::Facet {
                facet,
                providers,
                provider_addrs,
                dynamic,
            } => {
                if !ensure_all(state, dynamic) {
                    return (old.clone(), false);
                }
                let value = facet_value(state, facet, providers, provider_addrs);
                if facet.compare(&value, old) {
                    (old.clone(), false)
                } else {
                    (value, true)
                }
            }
        }
    }

    pub(crate) fn reconfigure(&self, state: &EditorState, old_state: &EditorState) -> SlotResult {
        match self {
            DynamicSlot::Field { field } => {
                let init = field.init_for(state);
                if let Some(init) = init {
                    if field.init_for(old_state).as_ref() != Some(&init) {
                        return ((init.0.create)(state), true);
                    }
                }
                if old_state.config().address.contains_key(&field.id) {
                    return (
                        old_state.field_value(field).expect("present").clone(),
                        false,
                    );
                }
                (field.create(state), true)
            }
            DynamicSlot::Provider { provider, .. } => {
                let Some(&old_addr) = old_state.config().address.get(&provider.id) else {
                    return (provider.get(state), true);
                };
                let old_value = old_state.get_addr(old_addr).clone();
                let deps_same = provider.dependencies.iter().all(|dep| match &dep.0 {
                    DepKind::Facet(facet) => {
                        same_value(old_state.facet_value(facet), state.facet_value(facet))
                    }
                    DepKind::Field(field) => {
                        match (old_state.field_value(field), state.field_value(field)) {
                            (Some(a), Some(b)) => same_value(a, b),
                            (None, None) => true,
                            _ => false,
                        }
                    }
                    DepKind::Doc | DepKind::Selection => true,
                });
                if deps_same {
                    return (old_value, false);
                }
                let value = provider.get(state);
                if provider.same(&value, &old_value) {
                    (old_value, false)
                } else {
                    (value, true)
                }
            }
            DynamicSlot::Facet {
                facet,
                providers,
                provider_addrs,
                ..
            } => {
                let dep_changed = ensure_all(state, provider_addrs);
                let old_value = old_state.facet_value(facet).clone();
                if let Some(old_providers) = old_state.config().facets.get(&facet.id) {
                    if !dep_changed && same_providers(providers, old_providers) {
                        return (old_value, false);
                    }
                }
                let value = facet_value(state, facet, providers, provider_addrs);
                if facet.compare(&value, &old_value) {
                    (old_value, false)
                } else {
                    (value, true)
                }
            }
        }
    }
}

/// A resolved configuration: the flattened extensions and the address of
/// every field, facet and provider value.
pub(crate) struct Configuration {
    pub(crate) base: Extension,
    pub(crate) compartments: HashMap<Compartment, Extension>,
    pub(crate) dynamic_slots: Vec<DynamicSlot>,
    // Even addresses are dynamic slots (index << 1), odd ones static
    // values ((index << 1) | 1), as upstream.
    pub(crate) address: HashMap<usize, usize>,
    pub(crate) static_values: Vec<AnyValue>,
    // Ordered by facet id, as upstream iterates an object's integer keys.
    pub(crate) facets: BTreeMap<usize, Vec<Arc<ProviderInner>>>,
}

enum Flat {
    Provider(Arc<ProviderInner>),
    Field(Arc<FieldInner>),
}

impl Flat {
    fn key(&self) -> *const () {
        match self {
            Flat::Provider(p) => Arc::as_ptr(p) as *const (),
            Flat::Field(f) => Arc::as_ptr(f) as *const (),
        }
    }
}

impl Configuration {
    pub(crate) fn static_facet<'a, O: 'static>(&'a self, facet: &'a FacetReader<O>) -> &'a O {
        match self.address.get(&facet.inner.id) {
            None => facet.default_value(),
            Some(addr) => cast(&self.static_values[addr >> 1]),
        }
    }

    pub(crate) fn resolve(
        base: Extension,
        compartments: &HashMap<Compartment, Extension>,
        old_state: Option<&EditorState>,
    ) -> Result<Configuration, RangeError> {
        let mut fields: Vec<Arc<FieldInner>> = Vec::new();
        let mut facets: BTreeMap<usize, Vec<Arc<ProviderInner>>> = BTreeMap::new();
        let mut new_compartments = HashMap::new();

        for ext in flatten(&base, compartments, &mut new_compartments)? {
            match ext {
                Flat::Field(field) => fields.push(field),
                Flat::Provider(p) => facets.entry(p.facet.id).or_default().push(p),
            }
        }

        let mut address: HashMap<usize, usize> = HashMap::new();
        let mut static_values: Vec<AnyValue> = Vec::new();
        enum Pending {
            Field(Arc<FieldInner>),
            Provider(Arc<ProviderInner>),
            Facet(Arc<FacetInner>, Vec<Arc<ProviderInner>>),
        }
        let mut pending: Vec<Pending> = Vec::new();

        for field in fields {
            address.insert(field.id, pending.len() << 1);
            pending.push(Pending::Field(field));
        }

        let old_facets = old_state.map(|s| &s.config().facets);
        for providers in facets.values() {
            let facet = providers[0].facet.clone();
            let old_providers: &[Arc<ProviderInner>] = old_facets
                .and_then(|f| f.get(&facet.id))
                .map_or(&[], Vec::as_slice);
            if providers.iter().all(|p| p.kind == ProviderKind::Static) {
                address.insert(facet.id, (static_values.len() << 1) | 1);
                if same_providers(old_providers, providers) {
                    let old = old_state.expect("old providers come from an old state");
                    static_values.push(old.facet_value(&facet).clone());
                } else {
                    let inputs: Vec<(ProviderKind, &AnyValue)> = providers
                        .iter()
                        .map(|p| (ProviderKind::Static, p.static_value()))
                        .collect();
                    let value = facet.combine(&inputs);
                    let value = match old_state {
                        Some(old) if facet.compare(&value, old.facet_value(&facet)) => {
                            old.facet_value(&facet).clone()
                        }
                        _ => value,
                    };
                    static_values.push(value);
                }
            } else {
                for p in providers {
                    if p.kind == ProviderKind::Static {
                        address.insert(p.id, (static_values.len() << 1) | 1);
                        static_values.push(p.static_value().clone());
                    } else {
                        address.insert(p.id, pending.len() << 1);
                        pending.push(Pending::Provider(p.clone()));
                    }
                }
                address.insert(facet.id, pending.len() << 1);
                pending.push(Pending::Facet(facet, providers.clone()));
            }
        }

        let dynamic_slots = pending
            .into_iter()
            .map(|slot| match slot {
                Pending::Field(field) => DynamicSlot::Field { field },
                Pending::Provider(provider) => {
                    let (mut dep_doc, mut dep_sel, mut dep_addrs) = (false, false, Vec::new());
                    for dep in &provider.dependencies {
                        let id = match &dep.0 {
                            DepKind::Doc => {
                                dep_doc = true;
                                continue;
                            }
                            DepKind::Selection => {
                                dep_sel = true;
                                continue;
                            }
                            DepKind::Facet(f) => f.id,
                            DepKind::Field(f) => f.id,
                        };
                        if let Some(&addr) = address.get(&id) {
                            if addr & 1 == 0 {
                                dep_addrs.push(addr);
                            }
                        }
                    }
                    DynamicSlot::Provider {
                        provider,
                        dep_doc,
                        dep_sel,
                        dep_addrs,
                    }
                }
                Pending::Facet(facet, providers) => {
                    let provider_addrs: Vec<usize> =
                        providers.iter().map(|p| address[&p.id]).collect();
                    let dynamic = provider_addrs
                        .iter()
                        .copied()
                        .filter(|a| a & 1 == 0)
                        .collect();
                    DynamicSlot::Facet {
                        facet,
                        providers,
                        provider_addrs,
                        dynamic,
                    }
                }
            })
            .collect();

        Ok(Configuration {
            base,
            compartments: new_compartments,
            dynamic_slots,
            address,
            static_values,
            facets,
        })
    }
}

fn flatten(
    extension: &Extension,
    compartments: &HashMap<Compartment, Extension>,
    new_compartments: &mut HashMap<Compartment, Extension>,
) -> Result<Vec<Flat>, RangeError> {
    struct Flattener<'a> {
        result: [Vec<Flat>; 5],
        seen: HashMap<*const (), usize>,
        compartments: &'a HashMap<Compartment, Extension>,
        new_compartments: &'a mut HashMap<Compartment, Extension>,
    }

    impl Flattener<'_> {
        fn inner(&mut self, ext: &Extension, prec: usize) -> Result<(), RangeError> {
            let key = ext.key();
            if let Some(&known) = self.seen.get(&key) {
                if known <= prec {
                    return Ok(());
                }
                if let Some(found) = self.result[known].iter().position(|e| e.key() == key) {
                    self.result[known].remove(found);
                }
                if let ExtKind::Compartment(c) = &ext.0 {
                    self.new_compartments.remove(&c.0);
                }
            }
            self.seen.insert(key, prec);
            match &ext.0 {
                ExtKind::List(list) => {
                    for e in list.iter() {
                        self.inner(e, prec)?;
                    }
                }
                ExtKind::Compartment(c) => {
                    let (compartment, inner) = c.as_ref();
                    if self.new_compartments.contains_key(compartment) {
                        return Err(RangeError(
                            "Duplicate use of compartment in extensions".to_string(),
                        ));
                    }
                    let content = self.compartments.get(compartment).unwrap_or(inner).clone();
                    self.new_compartments.insert(*compartment, content.clone());
                    self.inner(&content, prec)?;
                }
                ExtKind::Prec(p) => self.inner(&p.0, p.1)?,
                ExtKind::Field(field) => {
                    self.result[prec].push(Flat::Field(field.clone()));
                    if let Some(provides) = field.provides.get() {
                        self.inner(provides, prec)?;
                    }
                }
                ExtKind::Provider(p) => {
                    self.result[prec].push(Flat::Provider(p.clone()));
                    if let Some(extensions) = p.facet.extensions.get() {
                        self.inner(extensions, PREC_DEFAULT)?;
                    }
                }
            }
            Ok(())
        }
    }

    let mut flattener = Flattener {
        result: Default::default(),
        seen: HashMap::new(),
        compartments,
        new_compartments,
    };
    flattener.inner(extension, PREC_DEFAULT)?;
    Ok(flattener.result.into_iter().flatten().collect())
}
