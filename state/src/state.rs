//! Port of `state.ts` and `extension.ts`: the editor state and its
//! built-in facets.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::change::{ChangeSet, ChangeSpec, Insertion};
use crate::charcategory::{make_categorizer, CharCategory};
use crate::cluster::find_cluster_break;
use crate::facet::{
    AnyValue, Callback, Compartment, Configuration, Extension, FacetInner, FacetReader, FieldInner,
    StateField, COMPARTMENT_RECONFIGURE, STATUS_COMPUTED, STATUS_COMPUTING, STATUS_UNRESOLVED,
};
use crate::selection::{check_selection, EditorSelection, SelectionRange};
use crate::text::Text;
use crate::transaction::{
    effects, resolve_transaction, SpecItem, StateEffect, Transaction, TransactionSpec,
};
use crate::RangeError;

/// Language data: named values a language provides for a position, such
/// as `"wordChars"` or `"commentTokens"`.
pub type LanguageData = Arc<HashMap<String, AnyValue>>;

/// A language data provider ([`facets::LANGUAGE_DATA`]): given a state, a
/// position and a side (-1, 0 or 1), the language data that applies
/// there.
pub type LanguageDataProvider =
    Callback<dyn Fn(&EditorState, usize, i8) -> Vec<LanguageData> + Send + Sync>;

impl LanguageDataProvider {
    /// Wrap a language data provider function.
    pub fn new(
        f: impl Fn(&EditorState, usize, i8) -> Vec<LanguageData> + Send + Sync + 'static,
    ) -> Self {
        Callback(Arc::new(f))
    }
}

/// A command that needs only the state and a way to dispatch
/// transactions, not a view (`StateCommand`).
pub type StateCommand = fn(&EditorState, &mut dyn FnMut(Transaction)) -> bool;

/// The facets `EditorState` defines (its static properties upstream).
pub mod facets {
    use std::collections::HashMap;
    use std::sync::{Arc, LazyLock};

    use super::LanguageDataProvider;
    use crate::facet::{Facet, FacetConfig};
    use crate::transaction::{ChangeFilter, TransactionExtender, TransactionFilter};

    /// Allows more than one selection range when any input is true
    /// (`EditorState.allowMultipleSelections`). Static.
    pub static ALLOW_MULTIPLE_SELECTIONS: LazyLock<Facet<bool, bool>> = LazyLock::new(|| {
        Facet::define_with(
            FacetConfig::new(|values: &[bool]| values.iter().any(|&v| v)).static_facet(),
        )
    });

    /// The line separator; by default `"\n"`, `"\r\n"` and `"\r"` all split
    /// lines and lines are joined with `"\n"`
    /// (`EditorState.lineSeparator`). Static.
    pub static LINE_SEPARATOR: LazyLock<Facet<String, Option<String>>> = LazyLock::new(|| {
        Facet::define_with(
            FacetConfig::new(|values: &[String]| values.first().cloned()).static_facet(),
        )
    });

    /// The tab size: the first input, or 4 (`EditorState.tabSize`).
    pub static TAB_SIZE: LazyLock<Facet<usize, usize>> = LazyLock::new(|| {
        Facet::define_with(FacetConfig::new(|values: &[usize]| {
            values.first().copied().unwrap_or(4)
        }))
    });

    /// Whether editing commands should disable themselves: the first
    /// input, or false (`EditorState.readOnly`).
    pub static READ_ONLY: LazyLock<Facet<bool, bool>> = LazyLock::new(|| {
        Facet::define_with(FacetConfig::new(|values: &[bool]| {
            values.first().copied().unwrap_or(false)
        }))
    });

    /// Translations for [`EditorState::phrase`](crate::EditorState::phrase)
    /// (`EditorState.phrases`).
    pub static PHRASES: LazyLock<Facet<Arc<HashMap<String, String>>>> =
        LazyLock::new(Facet::define);

    /// Language data providers (`EditorState.languageData`).
    pub static LANGUAGE_DATA: LazyLock<Facet<LanguageDataProvider>> = LazyLock::new(Facet::define);

    /// Change filters, which can suppress part or all of a transaction's
    /// changes (`EditorState.changeFilter`).
    pub static CHANGE_FILTER: LazyLock<Facet<ChangeFilter>> = LazyLock::new(Facet::define);

    /// Transaction filters, which can replace a transaction with other
    /// specs (`EditorState.transactionFilter`).
    pub static TRANSACTION_FILTER: LazyLock<Facet<TransactionFilter>> =
        LazyLock::new(Facet::define);

    /// Transaction extenders, which add effects and annotations even to
    /// unfiltered transactions (`EditorState.transactionExtender`).
    pub static TRANSACTION_EXTENDER: LazyLock<Facet<TransactionExtender>> =
        LazyLock::new(Facet::define);
}

/// Options for [`EditorState::create`] (`EditorStateConfig`).
#[derive(Clone, Debug, Default)]
pub struct EditorStateConfig {
    /// The document: a string, split at the line separator, or a
    /// [`Text`]. Empty by default.
    pub doc: Option<Insertion>,
    /// The selection; a cursor at the start by default.
    pub selection: Option<EditorSelection>,
    /// The extensions.
    pub extensions: Extension,
}

/// What a range's change in [`EditorState::change_by_range`] produces.
#[derive(Clone, Debug)]
pub struct RangeChange {
    /// The range after the change, in the coordinates of the document the
    /// range's own changes produce.
    pub range: SelectionRange,
    /// The changes, in the coordinates of the start document.
    pub changes: Option<ChangeSpec>,
    /// Effects, in the coordinates of the document the range's own changes
    /// produce.
    pub effects: Vec<StateEffect>,
}

// How the slots of a state being built get their values.
#[derive(Clone)]
enum Compute {
    Create,
    Update(Transaction),
    Reconfigure(EditorState),
}

struct StateInner {
    config: Arc<Configuration>,
    doc: Text,
    selection: EditorSelection,
    // The values the slots start from: the previous state's values for an
    // update, none when creating or reconfiguring.
    start: Vec<Option<AnyValue>>,
    values: Vec<OnceLock<AnyValue>>,
    status: Vec<AtomicU8>,
    compute: Mutex<Option<Compute>>,
}

/// The editor state: a persistent, immutable value (`EditorState`). Apply
/// a [`Transaction`] to get a new state. Cloning is cheap and shares the
/// state.
#[derive(Clone)]
pub struct EditorState(Arc<StateInner>);

impl EditorState {
    fn build(
        config: Arc<Configuration>,
        doc: Text,
        selection: EditorSelection,
        start: Vec<Option<AnyValue>>,
        compute: Compute,
        tr: Option<&Transaction>,
    ) -> EditorState {
        let n = config.dynamic_slots.len();
        let state = EditorState(Arc::new(StateInner {
            config,
            doc,
            selection,
            start,
            values: (0..n).map(|_| OnceLock::new()).collect(),
            status: (0..n).map(|_| AtomicU8::new(STATUS_UNRESOLVED)).collect(),
            compute: Mutex::new(Some(compute)),
        }));
        // Set the transaction's state first, so that queries for it made
        // while the slots are computed return this state.
        if let Some(tr) = tr {
            tr.set_state(state.clone());
        }
        for i in 0..n {
            state.ensure_addr(i << 1);
        }
        *state.0.compute.lock().expect("compute lock") = None;
        state
    }

    pub(crate) fn config(&self) -> &Configuration {
        &self.0.config
    }

    pub(crate) fn same_config(&self, other: &EditorState) -> bool {
        Arc::ptr_eq(&self.0.config, &other.0.config)
    }

    /// Compute the slot at `addr` if needed (`ensureAddr`), returning its
    /// status.
    pub(crate) fn ensure_addr(&self, addr: usize) -> u8 {
        if addr & 1 == 1 {
            return STATUS_COMPUTED;
        }
        let idx = addr >> 1;
        let status = self.0.status[idx].load(Ordering::Acquire);
        if status == STATUS_COMPUTING {
            panic!("Cyclic dependency between fields and/or facets");
        }
        if status & STATUS_COMPUTED != 0 {
            return status;
        }
        self.0.status[idx].store(STATUS_COMPUTING, Ordering::Release);
        let compute = self
            .0
            .compute
            .lock()
            .expect("compute lock")
            .clone()
            .expect("an unresolved slot in a state being built");
        let slot = &self.0.config.dynamic_slots[idx];
        let (value, changed) = match &compute {
            Compute::Create => slot.create(self),
            Compute::Update(tr) => {
                let old = self.0.start[idx].as_ref().expect("a start value");
                slot.update(self, old, tr)
            }
            Compute::Reconfigure(old_state) => slot.reconfigure(self, old_state),
        };
        let _ = self.0.values[idx].set(value);
        let status = STATUS_COMPUTED | changed as u8;
        self.0.status[idx].store(status, Ordering::Release);
        status
    }

    /// The value at `addr`, which must be computed (`getAddr`).
    pub(crate) fn get_addr(&self, addr: usize) -> &AnyValue {
        if addr & 1 == 1 {
            &self.0.config.static_values[addr >> 1]
        } else {
            self.0.values[addr >> 1].get().expect("a computed slot")
        }
    }

    pub(crate) fn facet_value<'a>(&'a self, facet: &'a FacetInner) -> &'a AnyValue {
        match self.0.config.address.get(&facet.id) {
            None => &facet.default,
            Some(&addr) => {
                self.ensure_addr(addr);
                self.get_addr(addr)
            }
        }
    }

    pub(crate) fn field_value(&self, field: &FieldInner) -> Option<&AnyValue> {
        let &addr = self.0.config.address.get(&field.id)?;
        self.ensure_addr(addr);
        Some(self.get_addr(addr))
    }

    /// The document.
    pub fn doc(&self) -> &Text {
        &self.0.doc
    }

    /// The selection.
    pub fn selection(&self) -> &EditorSelection {
        &self.0.selection
    }

    /// The value of a state field (`field`).
    ///
    /// # Panics
    ///
    /// When the field is not present in this state.
    pub fn field<V: 'static>(&self, field: &StateField<V>) -> &V {
        self.field_opt(field)
            .unwrap_or_else(|| panic!("Field is not present in this state"))
    }

    /// The value of a state field, or `None` when it is not present
    /// (`field(f, false)`).
    pub fn field_opt<V: 'static>(&self, field: &StateField<V>) -> Option<&V> {
        self.field_value(&field.inner)
            .map(|value| value.downcast_ref::<V>().expect("the field's value type"))
    }

    /// The value of a facet (`facet`).
    pub fn facet<'a, O: 'static>(&'a self, facet: &'a FacetReader<O>) -> &'a O {
        self.facet_value(&facet.inner)
            .downcast_ref::<O>()
            .expect("the facet's output type")
    }

    /// A transaction that updates this state (`update` with one spec).
    pub fn update(&self, spec: impl Into<SpecItem>) -> Result<Transaction, RangeError> {
        resolve_transaction(self, &[spec.into()], true)
    }

    /// A transaction combining several specs (`update` with several).
    /// Unless a spec is [`sequential`](TransactionSpec::sequential), its
    /// changes are in the coordinates of this state's document, and its
    /// selection and effects in those of the document its own changes
    /// produce. Later selections take precedence.
    pub fn update_all(
        &self,
        specs: impl IntoIterator<Item = SpecItem>,
    ) -> Result<Transaction, RangeError> {
        let specs: Vec<SpecItem> = specs.into_iter().collect();
        resolve_transaction(self, &specs, true)
    }

    pub(crate) fn apply_transaction(&self, tr: &Transaction) {
        let old_config = &self.0.config;
        let mut reconfigured = false;
        let mut base = old_config.base.clone();
        let mut compartments: Option<HashMap<Compartment, Extension>> = None;
        for effect in tr.effects() {
            if let Some((compartment, extension)) = effect.value_of(&COMPARTMENT_RECONFIGURE) {
                reconfigured = true;
                compartments
                    .get_or_insert_with(|| old_config.compartments.clone())
                    .insert(*compartment, extension.clone());
            } else if let Some(extension) = effect.value_of(&effects::RECONFIGURE) {
                reconfigured = true;
                base = extension.clone();
            } else if let Some(extension) = effect.value_of(&effects::APPEND_CONFIG) {
                reconfigured = true;
                let mut list = base.as_list();
                // `Array.concat` spreads an array argument.
                list.extend(extension.as_list());
                base = Extension::from(list);
            }
        }
        let (config, start) = if reconfigured {
            let compartments = compartments.as_ref().unwrap_or(&old_config.compartments);
            let config = Arc::new(
                Configuration::resolve(base, compartments, Some(self))
                    .unwrap_or_else(|e| panic!("{e}")),
            );
            let n = config.dynamic_slots.len();
            let intermediate = EditorState::build(
                config.clone(),
                self.0.doc.clone(),
                self.0.selection.clone(),
                vec![None; n],
                Compute::Reconfigure(self.clone()),
                None,
            );
            (config, intermediate.values())
        } else {
            (old_config.clone(), self.values())
        };
        let selection = if *self.facet(&facets::ALLOW_MULTIPLE_SELECTIONS) {
            tr.new_selection()
        } else {
            tr.new_selection().as_single()
        };
        EditorState::build(
            config,
            tr.new_doc().clone(),
            selection,
            start,
            Compute::Update(tr.clone()),
            Some(tr),
        );
    }

    fn values(&self) -> Vec<Option<AnyValue>> {
        self.0.values.iter().map(|v| v.get().cloned()).collect()
    }

    /// A spec that replaces every selection range with `text`
    /// (`replaceSelection`).
    pub fn replace_selection(&self, text: impl Into<Insertion>) -> TransactionSpec {
        let text = match text.into() {
            Insertion::Str(s) => self.to_text(&s),
            Insertion::Text(text) => text,
        };
        self.change_by_range(|range| RangeChange {
            range: EditorSelection::cursor_with(range.from() + text.len(), -1, None, None),
            changes: Some(ChangeSpec::Change(crate::change::Change {
                from: range.from(),
                to: range.to(),
                insert: Some(Insertion::Text(text.clone())),
            })),
            effects: Vec::new(),
        })
        .expect("changes inside the selection are valid")
    }

    /// Run `f` for every selection range and merge the changes, ranges and
    /// effects it returns into one spec (`changeByRange`).
    pub fn change_by_range(
        &self,
        mut f: impl FnMut(&SelectionRange) -> RangeChange,
    ) -> Result<TransactionSpec, RangeError> {
        let sel = self.selection();
        let first = f(&sel.ranges()[0]);
        let mut changes = self.changes(first.changes)?;
        let mut ranges = vec![first.range];
        let mut effects = first.effects;
        for range in &sel.ranges()[1..] {
            let result = f(range);
            let new_changes = self.changes(result.changes)?;
            let new_mapped = new_changes.map(changes.desc(), false);
            for r in ranges.iter_mut() {
                *r = r.map(new_mapped.desc(), -1);
            }
            let map_by = changes.desc().map_desc(new_changes.desc(), true);
            ranges.push(result.range.map(&map_by, -1));
            changes = changes.compose(&new_mapped);
            let mut mapped = StateEffect::map_effects(&effects, new_mapped.desc());
            mapped.extend(StateEffect::map_effects(&result.effects, &map_by));
            effects = mapped;
        }
        Ok(TransactionSpec {
            changes: Some(ChangeSpec::Set(changes)),
            selection: Some(EditorSelection::create(ranges, sel.main_index())?),
            effects,
            ..TransactionSpec::default()
        })
    }

    /// A change set for `spec` in this state's document, using its line
    /// separator (`changes`).
    pub fn changes(&self, spec: Option<ChangeSpec>) -> Result<ChangeSet, RangeError> {
        match spec {
            Some(ChangeSpec::Set(set)) => Ok(set),
            spec => ChangeSet::of(
                spec.unwrap_or(ChangeSpec::List(Vec::new())),
                self.doc().len(),
                self.line_separator(),
            ),
        }
    }

    pub(crate) fn line_separator(&self) -> Option<&str> {
        self.facet(&facets::LINE_SEPARATOR).as_deref()
    }

    /// A [`Text`] for `string`, split at this state's line separator
    /// (`toText`).
    pub fn to_text(&self, string: &str) -> Text {
        Text::from_str_with(string, self.line_separator())
    }

    /// The document from `from` to `to`, joined with this state's line
    /// break (`sliceDoc`).
    pub fn slice_doc(&self, from: usize, to: usize) -> String {
        self.doc().slice_string_with(from, to, self.line_break())
    }

    /// Create a state (`EditorState.create`).
    pub fn create(config: EditorStateConfig) -> Result<EditorState, RangeError> {
        let configuration = Configuration::resolve(config.extensions, &HashMap::new(), None)?;
        let line_sep = configuration
            .static_facet(&facets::LINE_SEPARATOR)
            .as_deref();
        let doc = match config.doc {
            Some(Insertion::Text(text)) => text,
            Some(Insertion::Str(s)) => Text::from_str_with(&s, line_sep),
            None => Text::from_str_with("", line_sep),
        };
        let mut selection = config
            .selection
            .unwrap_or_else(|| EditorSelection::single(0, 0));
        check_selection(&selection, doc.len())?;
        if !*configuration.static_facet(&facets::ALLOW_MULTIPLE_SELECTIONS) {
            selection = selection.as_single();
        }
        let n = configuration.dynamic_slots.len();
        Ok(EditorState::build(
            Arc::new(configuration),
            doc,
            selection,
            vec![None; n],
            Compute::Create,
            None,
        ))
    }

    /// The tab size ([`facets::TAB_SIZE`]).
    pub fn tab_size(&self) -> usize {
        *self.facet(&facets::TAB_SIZE)
    }

    /// The line break for this state: its line separator or `"\n"`
    /// (`lineBreak`).
    pub fn line_break(&self) -> &str {
        self.line_separator().unwrap_or("\n")
    }

    /// Whether the state is read-only ([`facets::READ_ONLY`]).
    pub fn read_only(&self) -> bool {
        *self.facet(&facets::READ_ONLY)
    }

    /// The translation of `phrase` from [`facets::PHRASES`], or `phrase`
    /// itself, with `$1`, `$2`… (or a bare `$` for `$1`) replaced by the
    /// matching element of `insert` and `$$` by `$` (`phrase`).
    pub fn phrase(&self, phrase: &str, insert: &[&str]) -> String {
        let mut phrase = phrase.to_string();
        for map in self.facet(&facets::PHRASES).iter() {
            if let Some(translated) = map.get(&phrase) {
                phrase = translated.clone();
                break;
            }
        }
        if insert.is_empty() {
            return phrase;
        }
        // `phrase.replace(/\$(\$|\d*)/g, …)`
        let mut result = String::with_capacity(phrase.len());
        let mut rest = phrase.as_str();
        while let Some(at) = rest.find('$') {
            result.push_str(&rest[..at]);
            let after = &rest[at + 1..];
            if let Some(tail) = after.strip_prefix('$') {
                result.push('$');
                rest = tail;
                continue;
            }
            let digits = after.len() - after.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            let matched = &rest[at..at + 1 + digits];
            let n = if digits == 0 {
                1
            } else {
                after[..digits].parse::<usize>().unwrap_or(usize::MAX)
            };
            if n == 0 || n > insert.len() {
                result.push_str(matched);
            } else {
                result.push_str(insert[n - 1]);
            }
            rest = &after[digits..];
        }
        result.push_str(rest);
        result
    }

    /// The values of the language data field `name` at `pos`, from the
    /// providers in [`facets::LANGUAGE_DATA`] (`languageDataAt`). Values of
    /// another type than `T` are skipped.
    pub fn language_data_at<T: Clone + 'static>(&self, name: &str, pos: usize, side: i8) -> Vec<T> {
        let mut values = Vec::new();
        for provider in self.facet(&facets::LANGUAGE_DATA).iter() {
            for result in provider(self, pos, side) {
                if let Some(value) = result.get(name) {
                    if let Some(value) = value.downcast_ref::<T>() {
                        values.push(value.clone());
                    }
                }
            }
        }
        values
    }

    /// A function that sorts a string (meant to be one grapheme cluster)
    /// into word, space or other characters, counting the `"wordChars"`
    /// language data at `at` as word characters (`charCategorizer`).
    pub fn char_categorizer(&self, at: usize) -> impl Fn(&str) -> CharCategory {
        let chars = self.language_data_at::<String>("wordChars", at, -1);
        make_categorizer(chars.into_iter().next().unwrap_or_default())
    }

    /// The range of word characters around `pos`, or `None` when no word
    /// character is next to it (`wordAt`).
    pub fn word_at(&self, pos: usize) -> Option<SelectionRange> {
        let line = self.doc().line_at(pos);
        let (text, from) = (line.text, line.from);
        let cat = self.char_categorizer(pos);
        let mut start = pos - from;
        let mut end = pos - from;
        while start > 0 {
            let prev = find_cluster_break(text, start, false, true);
            if cat(&text[prev..start]) != CharCategory::Word {
                break;
            }
            start = prev;
        }
        while end < text.len() {
            let next = find_cluster_break(text, end, true, true);
            if cat(&text[end..next]) != CharCategory::Word {
                break;
            }
            end = next;
        }
        (start != end).then(|| EditorSelection::range(start + from, end + from))
    }
}

impl fmt::Debug for EditorState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EditorState")
            .field("doc", &self.0.doc)
            .field("selection", &self.0.selection)
            .finish_non_exhaustive()
    }
}
