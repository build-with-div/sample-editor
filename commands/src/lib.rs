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
//!   the like);
//! - the motion of `commands.ts` that needs the view, by character,
//!   group and subword in visual order and to line boundaries
//!   ([`cursor_char_left`], [`cursor_group_forward`],
//!   [`cursor_subword_forward`], [`cursor_line_boundary_backward`],
//!   [`select_char_right`] and the like);
//! - deletion by character, group and line boundary
//!   ([`delete_char_backward`], [`delete_group_forward`],
//!   [`delete_to_line_end`], [`delete_line_boundary_backward`] and the
//!   like), skipping the view's atomic ranges and announcing a deleted
//!   selection to screen readers;
//! - indentation by unit ([`indent_more`], [`indent_less`],
//!   [`insert_tab`]) with `@codemirror/language`'s indentation unit, and
//!   the tab-focus commands ([`toggle_tab_focus_mode`]);
//! - `comment.ts`: line and block comments from the `"commentTokens"`
//!   language data ([`toggle_comment`], [`toggle_line_comment`],
//!   [`toggle_block_comment`], [`CommentTokens`] and the like).
//!
//! Upstream's `Command`s take the view here too; the others have the
//! [`StateCommand`] shape. `deleteGroupBackward` and `deleteGroupForward`
//! are state commands that skip the view's atomic ranges when run on a
//! view, so they also come in a view form ([`delete_group_backward_view`]).
//!
//! Not yet ported:
//! - the commands that need the view's coordinates (motion by visual
//!   line or page, `addCursorAbove`, `deleteLine`; Div gaps 5 and 6) or
//!   `@codemirror/language`'s syntax tree (syntax motion, bracket
//!   matching, `indentSelection`, `insertNewlineAndIndent`,
//!   `insertBlankLine`), and the keymaps, which bind them;
//! - `historyKeymap` and the handler for the `historyUndo` and
//!   `historyRedo` input events, which belong to the view;
//! - the history field's JSON serialization, like `EditorState`'s.
//!
//! [`StateCommand`]: editor_state::StateCommand

mod commands;
mod comment;
mod editing;
mod history;
mod motion;

pub use commands::{
    copy_line_down, copy_line_up, cursor_char_backward_logical, cursor_char_forward_logical,
    cursor_doc_end, cursor_doc_start, delete_trailing_whitespace, insert_newline,
    insert_newline_keep_indent, move_line_down, move_line_up, select_all,
    select_char_backward_logical, select_char_forward_logical, select_doc_end, select_doc_start,
    select_line, simplify_selection, split_line, transpose_chars,
};

pub use motion::{
    cursor_char_backward, cursor_char_forward, cursor_char_left, cursor_char_right,
    cursor_group_backward, cursor_group_forward, cursor_group_forward_win, cursor_group_left,
    cursor_group_right, cursor_line_boundary_backward, cursor_line_boundary_forward,
    cursor_line_boundary_left, cursor_line_boundary_right, cursor_line_end, cursor_line_start,
    cursor_subword_backward, cursor_subword_forward, select_char_backward, select_char_forward,
    select_char_left, select_char_right, select_group_backward, select_group_forward,
    select_group_forward_win, select_group_left, select_group_right, select_line_boundary_backward,
    select_line_boundary_forward, select_line_boundary_left, select_line_boundary_right,
    select_line_end, select_line_start, select_subword_backward, select_subword_forward,
};

pub use comment::{
    block_comment, block_uncomment, line_comment, line_uncomment, toggle_block_comment,
    toggle_block_comment_by_line, toggle_comment, toggle_line_comment, BlockTokens, CommentTokens,
};

pub use editing::{
    delete_char_backward, delete_char_backward_strict, delete_char_forward, delete_group_backward,
    delete_group_backward_view, delete_group_forward, delete_group_forward_view,
    delete_group_forward_win, delete_line_boundary_backward, delete_line_boundary_forward,
    delete_to_line_end, delete_to_line_start, indent_less, indent_more, insert_tab,
    temporarily_set_tab_focus_mode, toggle_tab_focus_mode,
};

pub use history::{
    can_redo_selection, can_undo_selection, history, inverted_effects_fn, join_to_event_fn, redo,
    redo_depth, redo_selection, undo, undo_depth, undo_selection, HistoryConfig, HistoryState,
    InvertedEffects, Isolate, JoinToEvent, HISTORY_FIELD, INVERTED_EFFECTS, ISOLATE_HISTORY,
};
