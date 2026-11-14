//! A Rust port of [`@codemirror/commands`](https://codemirror.net/docs/ref/#commands)
//! 6.11.1, on [`editor_state`].
//!
//! The port so far covers the undo history (`history.ts`): the
//! [`history`] extension, the [`undo`], [`redo`], [`undo_selection`] and
//! [`redo_selection`] commands, which have the [`StateCommand`] shape,
//! [`undo_depth`] and [`redo_depth`], [`ISOLATE_HISTORY`] and
//! [`INVERTED_EFFECTS`].
//!
//! Not yet ported:
//! - the editing and motion commands and the default keymap
//!   (`commands.ts`, `comment.ts`), many of which need the view and
//!   `@codemirror/language`;
//! - `historyKeymap` and the handler for the `historyUndo` and
//!   `historyRedo` input events, which belong to the view;
//! - the history field's JSON serialization, like `EditorState`'s.
//!
//! [`StateCommand`]: editor_state::StateCommand

mod history;

pub use history::{
    can_redo_selection, can_undo_selection, history, inverted_effects_fn, join_to_event_fn, redo,
    redo_depth, redo_selection, undo, undo_depth, undo_selection, HistoryConfig, HistoryState,
    InvertedEffects, Isolate, JoinToEvent, HISTORY_FIELD, INVERTED_EFFECTS, ISOLATE_HISTORY,
};
