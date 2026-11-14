//! Port of `history.ts`: the undo history.

use std::sync::{Arc, LazyLock};

use editor_state::{
    AnnotationType, Callback, ChangeDesc, ChangeSet, ChangeSpec, EditorSelection, EditorState,
    Extension, Facet, FacetConfig, StateEffect, StateField, StateFieldSpec, Transaction,
    TransactionSpec,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum BranchName {
    Done,
    Undone,
}

struct FromHistory {
    side: BranchName,
    rest: Branch,
    selection: EditorSelection,
}

static FROM_HISTORY: LazyLock<AnnotationType<FromHistory>> = LazyLock::new(AnnotationType::define);

/// The value of an [`ISOLATE_HISTORY`] annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Isolate {
    /// Don't merge with the previous transactions (`"before"`).
    Before,
    /// Don't merge the following transactions with this one (`"after"`).
    After,
    /// Isolate the transaction on both sides (`"full"`).
    Full,
}

/// Transaction annotation that keeps the transaction from being combined
/// with other transactions in the undo history (`isolateHistory`).
pub static ISOLATE_HISTORY: LazyLock<AnnotationType<Isolate>> =
    LazyLock::new(AnnotationType::define);

/// A function that, given a transaction, returns the effects the history
/// should store when inverting it.
pub type InvertedEffects = Callback<dyn Fn(&Transaction) -> Vec<StateEffect> + Send + Sync>;

/// Wrap `f` as an [`InvertedEffects`] callback.
pub fn inverted_effects_fn(
    f: impl Fn(&Transaction) -> Vec<StateEffect> + Send + Sync + 'static,
) -> InvertedEffects {
    Callback(Arc::new(f))
}

/// Registers functions that return the effects the history stores when
/// it inverts a transaction, so that those effects can be undone and
/// redone (`invertedEffects`).
pub static INVERTED_EFFECTS: LazyLock<Facet<InvertedEffects>> = LazyLock::new(Facet::define);

/// Decides whether a transaction joins the current undo event; the flag
/// says whether its changes touch the event's changes.
pub type JoinToEvent = Callback<dyn Fn(&Transaction, bool) -> bool + Send + Sync>;

/// Wrap `f` as a [`JoinToEvent`] callback.
pub fn join_to_event_fn(
    f: impl Fn(&Transaction, bool) -> bool + Send + Sync + 'static,
) -> JoinToEvent {
    Callback(Arc::new(f))
}

/// The configuration of [`history`] (`HistoryConfig`). Fields left `None`
/// take the default, or another configuration's value.
#[derive(Clone, Default, PartialEq)]
pub struct HistoryConfig {
    /// The minimum number of events to keep. Defaults to 100; several
    /// configurations take the largest.
    pub min_depth: Option<usize>,
    /// The longest time, in milliseconds, between adjacent events that
    /// are still grouped. Defaults to 500; several configurations take
    /// the smallest.
    pub new_group_delay: Option<f64>,
    /// Decides whether changes join the current event when close enough
    /// in time. By default they join when they touch the event's changes;
    /// several configurations join when any of them says so.
    pub join_to_event: Option<JoinToEvent>,
}

// `combineConfig` with upstream's defaults and combiners.
#[derive(PartialEq)]
struct ResolvedConfig {
    min_depth: usize,
    new_group_delay: f64,
    join_to_event: Vec<JoinToEvent>,
}

impl ResolvedConfig {
    fn join_to_event(&self, tr: &Transaction, is_adjacent: bool) -> bool {
        if self.join_to_event.is_empty() {
            return is_adjacent;
        }
        self.join_to_event.iter().any(|f| f(tr, is_adjacent))
    }
}

static HISTORY_CONFIG: LazyLock<Facet<HistoryConfig, ResolvedConfig>> = LazyLock::new(|| {
    Facet::define_with(FacetConfig::new(|configs: &[HistoryConfig]| {
        ResolvedConfig {
            min_depth: configs
                .iter()
                .filter_map(|c| c.min_depth)
                .max()
                .unwrap_or(100),
            new_group_delay: configs
                .iter()
                .filter_map(|c| c.new_group_delay)
                .reduce(f64::min)
                .unwrap_or(500.0),
            join_to_event: configs
                .iter()
                .filter_map(|c| c.join_to_event.clone())
                .collect(),
        }
    }))
});

/// The history's state. Opaque; read it through [`undo_depth`] and the
/// other functions here.
pub struct HistoryState {
    done: Branch,
    undone: Branch,
    prev_time: f64,
    prev_user_event: Option<String>,
}

/// The state field holding the history (`historyField`).
pub static HISTORY_FIELD: LazyLock<StateField<Arc<HistoryState>>> = LazyLock::new(|| {
    StateField::define(StateFieldSpec::with_compare(
        |_| Arc::new(HistoryState::new(Vec::new(), Vec::new())),
        update_history,
        Arc::ptr_eq,
    ))
});

fn update_history(state: &Arc<HistoryState>, tr: &Transaction) -> Arc<HistoryState> {
    let config = tr.state().facet(&HISTORY_CONFIG);

    if let Some(from_hist) = tr.annotation(*FROM_HISTORY) {
        let item = HistEvent::from_transaction(tr, Some(from_hist.selection.clone()));
        let from = from_hist.side;
        let other = if from == BranchName::Done {
            &state.undone
        } else {
            &state.done
        };
        let other = match item {
            Some(item) => update_branch(other, other.len(), config.min_depth, item),
            None => add_selection(other, tr.start_state().selection().clone()),
        };
        return Arc::new(if from == BranchName::Done {
            HistoryState::new(from_hist.rest.clone(), other)
        } else {
            HistoryState::new(other, from_hist.rest.clone())
        });
    }

    let isolate = tr.annotation(*ISOLATE_HISTORY).copied();
    let mut state = state.clone();
    if matches!(isolate, Some(Isolate::Full | Isolate::Before)) {
        state = state.isolate();
    }

    if tr.annotation(Transaction::ADD_TO_HISTORY) == Some(&false) {
        return if !tr.changes().is_empty() {
            Arc::new(state.add_mapping(tr.changes().desc()))
        } else {
            state
        };
    }

    let event = HistEvent::from_transaction(tr, None);
    let time = tr.annotation(Transaction::TIME).copied().unwrap_or(0.0);
    let user_event = tr.annotation(Transaction::USER_EVENT).map(String::as_str);
    if let Some(event) = event {
        state = Arc::new(state.add_changes(event, time, user_event, config, tr));
    } else if tr.selection().is_some() {
        state = state.add_selection(
            tr.start_state().selection().clone(),
            time,
            user_event,
            config.new_group_delay,
        );
    }

    if matches!(isolate, Some(Isolate::Full | Isolate::After)) {
        state = state.isolate();
    }
    state
}

/// The history extension (`history`). Upstream also handles the
/// `historyUndo` and `historyRedo` input events in the view, which comes
/// with the view port.
pub fn history(config: HistoryConfig) -> Extension {
    Extension::from(vec![
        Extension::from(&*HISTORY_FIELD),
        HISTORY_CONFIG.of(config),
    ])
}

fn cmd(
    side: BranchName,
    selection: bool,
    state: &EditorState,
    dispatch: &mut dyn FnMut(Transaction),
) -> bool {
    if !selection && state.read_only() {
        return false;
    }
    let Some(history_state) = state.field_opt(&HISTORY_FIELD) else {
        return false;
    };
    let Some(tr) = history_state.pop(side, state, selection) else {
        return false;
    };
    dispatch(tr);
    true
}

/// Undo a single group of history events. Returns false when there is
/// none (`undo`).
pub fn undo(state: &EditorState, dispatch: &mut dyn FnMut(Transaction)) -> bool {
    cmd(BranchName::Done, false, state, dispatch)
}

/// Redo a group of history events. Returns false when there is none
/// (`redo`).
pub fn redo(state: &EditorState, dispatch: &mut dyn FnMut(Transaction)) -> bool {
    cmd(BranchName::Undone, false, state, dispatch)
}

/// Undo a change or selection change (`undoSelection`).
pub fn undo_selection(state: &EditorState, dispatch: &mut dyn FnMut(Transaction)) -> bool {
    cmd(BranchName::Done, true, state, dispatch)
}

/// Redo a change or selection change (`redoSelection`).
pub fn redo_selection(state: &EditorState, dispatch: &mut dyn FnMut(Transaction)) -> bool {
    cmd(BranchName::Undone, true, state, dispatch)
}

fn depth(side: BranchName, state: &EditorState) -> usize {
    let Some(hist_state) = state.field_opt(&HISTORY_FIELD) else {
        return 0;
    };
    let branch = hist_state.branch(side);
    branch.len() - usize::from(branch.first().is_some_and(|e| e.changes.is_none()))
}

/// The number of undoable change events in `state` (`undoDepth`).
pub fn undo_depth(state: &EditorState) -> usize {
    depth(BranchName::Done, state)
}

/// The number of redoable change events in `state` (`redoDepth`).
pub fn redo_depth(state: &EditorState) -> usize {
    depth(BranchName::Undone, state)
}

fn has_selection(side: BranchName, state: &EditorState) -> bool {
    let Some(hist_state) = state.field_opt(&HISTORY_FIELD) else {
        return false;
    };
    hist_state
        .branch(side)
        .first()
        .is_some_and(|e| !e.selections_after.is_empty())
}

/// Whether an undoable selection is available (`canUndoSelection`).
pub fn can_undo_selection(state: &EditorState) -> bool {
    has_selection(BranchName::Done, state)
}

/// Whether a redoable selection is available (`canRedoSelection`).
/// Upstream 6.11.1 defines this with the done branch, like
/// [`can_undo_selection`]; the port keeps that behaviour.
pub fn can_redo_selection(state: &EditorState) -> bool {
    has_selection(BranchName::Done, state)
}

// History events store groups of changes or effects that are undone and
// redone together.
struct HistEvent {
    // The changes. Normal events hold at least one change or effect; a
    // selection event, stored before the first change, holds no changes
    // and no start selection.
    changes: Option<ChangeSet>,
    effects: Vec<StateEffect>,
    // Accumulated mapping (from transactions outside the history) to
    // apply to the events below this one.
    mapped: Option<ChangeDesc>,
    // The selection before this event.
    start_selection: Option<EditorSelection>,
    // Selection changes after this event, for selection undo and redo.
    selections_after: Vec<EditorSelection>,
}

impl HistEvent {
    fn set_sel_after(&self, after: Vec<EditorSelection>) -> HistEvent {
        HistEvent {
            changes: self.changes.clone(),
            effects: self.effects.clone(),
            mapped: self.mapped.clone(),
            start_selection: self.start_selection.clone(),
            selections_after: after,
        }
    }

    // Does not check `addToHistory` and such; assumes the transaction is
    // to become an event. `None` when it has no changes or effects.
    fn from_transaction(tr: &Transaction, selection: Option<EditorSelection>) -> Option<HistEvent> {
        let mut effects = Vec::new();
        for invert in tr.start_state().facet(&INVERTED_EFFECTS) {
            effects.extend(invert(tr));
        }
        if effects.is_empty() && tr.changes().is_empty() {
            return None;
        }
        Some(HistEvent {
            changes: Some(tr.changes().invert(tr.start_state().doc())),
            effects,
            mapped: None,
            start_selection: Some(
                selection.unwrap_or_else(|| tr.start_state().selection().clone()),
            ),
            selections_after: Vec::new(),
        })
    }

    fn selection(selections: Vec<EditorSelection>) -> HistEvent {
        HistEvent {
            changes: None,
            effects: Vec::new(),
            mapped: None,
            start_selection: None,
            selections_after: selections,
        }
    }
}

// Events are shared between branches, as upstream shares them between
// arrays.
type Branch = Vec<Arc<HistEvent>>;

fn update_branch(
    branch: &[Arc<HistEvent>],
    to: usize,
    max_len: usize,
    new_event: HistEvent,
) -> Branch {
    let start = if to + 1 > max_len + 20 {
        to - max_len - 1
    } else {
        0
    };
    let mut new_branch = branch[start..to].to_vec();
    new_branch.push(Arc::new(new_event));
    new_branch
}

fn is_adjacent(a: &ChangeDesc, b: &ChangeDesc) -> bool {
    let mut ranges = Vec::new();
    let mut is_adjacent = false;
    a.iter_changed_ranges(|f, t, _, _| ranges.push((f, t)), false);
    b.iter_changed_ranges(
        |_, _, f, t| {
            for &(from, to) in &ranges {
                if t >= from && f <= to {
                    is_adjacent = true;
                }
            }
        },
        false,
    );
    is_adjacent
}

fn eq_selection_shape(a: &EditorSelection, b: &EditorSelection) -> bool {
    a.ranges().len() == b.ranges().len()
        && a.ranges()
            .iter()
            .zip(b.ranges())
            .all(|(a, b)| a.is_empty() == b.is_empty())
}

const MAX_SELECTIONS_PER_EVENT: usize = 200;

fn add_selection(branch: &[Arc<HistEvent>], selection: EditorSelection) -> Branch {
    let Some(last_event) = branch.last() else {
        return vec![Arc::new(HistEvent::selection(vec![selection]))];
    };
    let after = &last_event.selections_after;
    let mut sels = after[after.len().saturating_sub(MAX_SELECTIONS_PER_EVENT)..].to_vec();
    if sels
        .last()
        .is_some_and(|last| last.eq_with(&selection, false))
    {
        return branch.to_vec();
    }
    sels.push(selection);
    update_branch(
        branch,
        branch.len() - 1,
        1_000_000_000,
        last_event.set_sel_after(sels),
    )
}

// Assumes the top event has one or more `selections_after`.
fn pop_selection(branch: &[Arc<HistEvent>]) -> Branch {
    let last = branch.last().expect("a non-empty branch");
    let mut new_branch = branch.to_vec();
    let after = &last.selections_after;
    *new_branch.last_mut().expect("a non-empty branch") =
        Arc::new(last.set_sel_after(after[..after.len() - 1].to_vec()));
    new_branch
}

// Add a mapping to the top event in the branch. If that maps away all the
// event's changes and effects, drop it and carry the mapping to the next
// event.
fn add_mapping_to_branch(branch: &[Arc<HistEvent>], mapping: &ChangeDesc) -> Branch {
    if branch.is_empty() {
        return Vec::new();
    }
    let mut length = branch.len();
    let mut selections: Vec<EditorSelection> = Vec::new();
    let mut mapping = mapping.clone();
    while length > 0 {
        let event = map_event(&branch[length - 1], &mapping, &selections);
        if event.changes.as_ref().is_some_and(|c| !c.is_empty()) || !event.effects.is_empty() {
            // The event survived the mapping.
            let mut result = branch[..length].to_vec();
            result[length - 1] = Arc::new(event);
            return result;
        }
        // Drop the event: it has no changes or effects left. Only a
        // selection event, always the bottom one, lacks a mapping.
        if let Some(mapped) = event.mapped {
            mapping = mapped;
        }
        length -= 1;
        selections = event.selections_after;
    }
    if selections.is_empty() {
        Vec::new()
    } else {
        vec![Arc::new(HistEvent::selection(selections))]
    }
}

fn map_event(
    event: &HistEvent,
    mapping: &ChangeDesc,
    extra_selections: &[EditorSelection],
) -> HistEvent {
    let mut selections: Vec<EditorSelection> = event
        .selections_after
        .iter()
        .map(|s| s.map(mapping, -1))
        .collect();
    selections.extend_from_slice(extra_selections);
    // Change-less events don't store mappings; they are always the last
    // event in a branch.
    let Some(changes) = &event.changes else {
        return HistEvent::selection(selections);
    };

    let mapped_changes = changes.map(mapping, false);
    let before = mapping.map_desc(changes.desc(), true);
    let full_mapping = match &event.mapped {
        Some(mapped) => mapped.compose_desc(&before),
        None => before.clone(),
    };
    HistEvent {
        changes: Some(mapped_changes),
        effects: StateEffect::map_effects(&event.effects, mapping),
        mapped: Some(full_mapping),
        start_selection: Some(
            event
                .start_selection
                .as_ref()
                .expect("an event with changes has a start selection")
                .map(&before, -1),
        ),
        selections_after: selections,
    }
}

// `/^(input\.type|delete)($|\.)/`
fn joinable_user_event(event: &str) -> bool {
    has_prefix_segment(event, "input.type") || has_prefix_segment(event, "delete")
}

fn has_prefix_segment(event: &str, prefix: &str) -> bool {
    event
        .strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

impl HistoryState {
    fn new(done: Branch, undone: Branch) -> HistoryState {
        HistoryState {
            done,
            undone,
            prev_time: 0.0,
            prev_user_event: None,
        }
    }

    fn branch(&self, side: BranchName) -> &Branch {
        match side {
            BranchName::Done => &self.done,
            BranchName::Undone => &self.undone,
        }
    }

    fn isolate(self: &Arc<Self>) -> Arc<HistoryState> {
        if self.prev_time != 0.0 {
            Arc::new(HistoryState::new(self.done.clone(), self.undone.clone()))
        } else {
            self.clone()
        }
    }

    fn add_changes(
        &self,
        event: HistEvent,
        time: f64,
        user_event: Option<&str>,
        config: &ResolvedConfig,
        tr: &Transaction,
    ) -> HistoryState {
        let mut done = self.done.clone();
        let last_event = done.last();
        let join = match (last_event, &event.changes) {
            (Some(last), Some(changes)) => match &last.changes {
                Some(last_changes) if !last_changes.is_empty() => {
                    user_event.is_none_or(joinable_user_event)
                        && (last.selections_after.is_empty()
                            && time - self.prev_time < config.new_group_delay
                            && config.join_to_event(tr, is_adjacent(last_changes.desc(), changes.desc()))
                            // For compose (but not compose.start) events,
                            // always join with the previous event.
                            || user_event == Some("input.type.compose"))
                }
                _ => false,
            },
            _ => false,
        };
        if join {
            let last = done.last().expect("a last event").clone();
            let last_changes = last.changes.as_ref().expect("a last event with changes");
            let changes = event.changes.as_ref().expect("changes");
            let mut effects = StateEffect::map_effects(&event.effects, last_changes.desc());
            effects.extend(last.effects.iter().cloned());
            let joined = HistEvent {
                changes: Some(changes.compose(last_changes)),
                effects,
                mapped: last.mapped.clone(),
                start_selection: last.start_selection.clone(),
                selections_after: Vec::new(),
            };
            done = update_branch(&done, done.len() - 1, config.min_depth, joined);
        } else {
            done = update_branch(&done, done.len(), config.min_depth, event);
        }
        HistoryState {
            done,
            undone: Vec::new(),
            prev_time: time,
            prev_user_event: user_event.map(str::to_string),
        }
    }

    fn add_selection(
        self: &Arc<Self>,
        selection: EditorSelection,
        time: f64,
        user_event: Option<&str>,
        new_group_delay: f64,
    ) -> Arc<HistoryState> {
        let last = self
            .done
            .last()
            .map_or(&[][..], |e| &e.selections_after[..]);
        if let Some(last_sel) = last.last() {
            if time - self.prev_time < new_group_delay
                && user_event == self.prev_user_event.as_deref()
                && user_event.is_some_and(|e| has_prefix_segment(e, "select"))
                && eq_selection_shape(last_sel, &selection)
            {
                return self.clone();
            }
        }
        Arc::new(HistoryState {
            done: add_selection(&self.done, selection),
            undone: self.undone.clone(),
            prev_time: time,
            prev_user_event: user_event.map(str::to_string),
        })
    }

    fn add_mapping(&self, mapping: &ChangeDesc) -> HistoryState {
        HistoryState {
            done: add_mapping_to_branch(&self.done, mapping),
            undone: add_mapping_to_branch(&self.undone, mapping),
            prev_time: self.prev_time,
            prev_user_event: self.prev_user_event.clone(),
        }
    }

    fn pop(
        &self,
        side: BranchName,
        state: &EditorState,
        only_selection: bool,
    ) -> Option<Transaction> {
        let branch = self.branch(side);
        let event = branch.last()?;
        let selection = match (event.selections_after.first(), &event.start_selection) {
            (Some(first), _) => first.clone(),
            (None, Some(start)) => start.map(
                &event
                    .changes
                    .as_ref()
                    .expect("an event with a start selection has changes")
                    .desc()
                    .inverted_desc(),
                1,
            ),
            (None, None) => state.selection().clone(),
        };
        let user_event = |done: &str, undone: &str| {
            Some(
                if side == BranchName::Done {
                    done
                } else {
                    undone
                }
                .to_string(),
            )
        };
        let spec = if only_selection && !event.selections_after.is_empty() {
            TransactionSpec {
                selection: event.selections_after.last().cloned(),
                annotations: vec![FROM_HISTORY.of(FromHistory {
                    side,
                    rest: pop_selection(branch),
                    selection,
                })],
                user_event: user_event("select.undo", "select.redo"),
                scroll_into_view: true,
                ..Default::default()
            }
        } else {
            let changes = event.changes.as_ref()?;
            let mut rest = branch[..branch.len() - 1].to_vec();
            if let Some(mapped) = &event.mapped {
                rest = add_mapping_to_branch(&rest, mapped);
            }
            TransactionSpec {
                changes: Some(ChangeSpec::Set(changes.clone())),
                selection: event.start_selection.clone(),
                effects: event.effects.clone(),
                annotations: vec![FROM_HISTORY.of(FromHistory {
                    side,
                    rest,
                    selection,
                })],
                filter: Some(false),
                user_event: user_event("undo", "redo"),
                scroll_into_view: true,
                ..Default::default()
            }
        };
        Some(
            state
                .update(spec)
                .expect("history events fit the document they apply to"),
        )
    }
}
