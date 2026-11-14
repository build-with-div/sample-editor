//! A Rust port of [`@codemirror/commands`](https://codemirror.net/docs/ref/#commands)
//! 6.11.1, on [`editor_state`].
//!
//! The port so far covers:
//! - the undo history (`history.ts`): the [`history`] extension, the
//!   [`undo`], [`redo`], [`undo_selection`] and [`redo_selection`]
//!   commands, [`undo_depth`] and [`redo_depth`], [`ISOLATE_HISTORY`] and
//!   [`INVERTED_EFFECTS`];
//! - the commands of `commands.ts` that need only the state: logical
//!   character motion ([`cursor_char_forward_logical`] and the like),
//!   document and line selection ([`select_all`], [`select_line`],
//!   [`simplify_selection`]), and line and whitespace editing
//!   ([`move_line_up`], [`copy_line_down`], [`transpose_chars`],
//!   [`insert_newline_keep_indent`], [`delete_trailing_whitespace`] and
//!   the like).
//!
//! Every command has the [`StateCommand`] shape.
//!
//! Not yet ported:
//! - the other commands of `commands.ts` and `comment.ts`, and the
//!   keymaps: they need the view (motion by visual line, group or page,
//!   atomic ranges, screen-reader announcements on deletion) or
//!   `@codemirror/language` (indentation, the syntax tree, bracket
//!   matching, comment tokens);
//! - `historyKeymap` and the handler for the `historyUndo` and
//!   `historyRedo` input events, which belong to the view;
//! - the history field's JSON serialization, like `EditorState`'s.
//!
//! [`StateCommand`]: editor_state::StateCommand

mod commands;
mod history;

pub use commands::{
    copy_line_down, copy_line_up, cursor_char_backward_logical, cursor_char_forward_logical,
    cursor_doc_end, cursor_doc_start, delete_trailing_whitespace, insert_newline,
    insert_newline_keep_indent, move_line_down, move_line_up, select_all,
    select_char_backward_logical, select_char_forward_logical, select_doc_end, select_doc_start,
    select_line, simplify_selection, split_line, transpose_chars,
};

pub use history::{
    can_redo_selection, can_undo_selection, history, inverted_effects_fn, join_to_event_fn, redo,
    redo_depth, redo_selection, undo, undo_depth, undo_selection, HistoryConfig, HistoryState,
    InvertedEffects, Isolate, JoinToEvent, HISTORY_FIELD, INVERTED_EFFECTS, ISOLATE_HISTORY,
};
