//! Port of the deletion, indentation and tab commands of `commands.ts`:
//! `deleteBy` with its atomic-range skipping, deletion by character,
//! group and line boundary, `indentMore`, `indentLess`, `insertTab` and the
//! tab-focus commands.
//!
//! `deleteLine` keeps the cursor's visual column with `moveVertically`,
//! which needs Div gaps 5 and 6; it comes with vertical motion. The
//! indentation commands that read the syntax tree (`indentSelection`,
//! `insertNewlineAndIndent`, `insertBlankLine`) come with
//! `@codemirror/language`'s tree (M8).

use editor_language::{get_indent_unit, indent_string, INDENT_UNIT};
use editor_state::{
    count_column, find_cluster_break, ChangeSpec, CharCategory, EditorSelection, EditorState, Line,
    RangeChange, SelectionRange, Transaction, TransactionSpec,
};
use editor_view::{EditorView, TabFocusMode, ATOMIC_RANGES};

use crate::commands::{change_by_range, is_js_space, range_only, update, Dispatch};
use crate::motion::{dispatch, to_group_start};

// `skipAtomic`: move `pos` out of the view's atomic ranges that cover it.
// Upstream does this only when the command's target is a view.
fn skip_atomic(view: Option<&EditorView>, pos: usize, forward: bool) -> usize {
    let mut pos = pos;
    let Some(view) = view else {
        return pos;
    };
    for ranges in view.state().facet(&ATOMIC_RANGES).iter() {
        let at = pos;
        ranges(view).between(at, at, |from, to, _| {
            if from < pos && to > pos {
                pos = if forward { to } else { from };
            }
            true
        });
    }
    pos
}

// `deleteBy`: delete each selection range, or, from a cursor, up to the
// position `by` gives. `view` is the target when it is a view.
fn delete_by(
    state: &EditorState,
    view: Option<&EditorView>,
    mut by: impl FnMut(&SelectionRange) -> usize,
) -> Option<Transaction> {
    if state.read_only() {
        return None;
    }
    let mut event = "delete.selection";
    let mut spec = change_by_range(state, |range| {
        let (mut from, mut to) = (range.from(), range.to());
        if from == to {
            let mut towards = by(range);
            if towards < from {
                event = "delete.backward";
                towards = skip_atomic(view, towards, false);
            } else if towards > from {
                event = "delete.forward";
                towards = skip_atomic(view, towards, true);
            }
            from = from.min(towards);
            to = to.max(towards);
        } else {
            from = skip_atomic(view, from, false);
            to = skip_atomic(view, to, true);
        }
        if from == to {
            range_only(range)
        } else {
            RangeChange {
                range: EditorSelection::cursor_with(
                    from,
                    if from < range.head() { -1 } else { 1 },
                    None,
                    None,
                ),
                changes: Some(ChangeSpec::delete(from, to)),
                effects: Vec::new(),
            }
        }
    });
    if matches!(&spec.changes, Some(ChangeSpec::Set(changes)) if changes.is_empty()) {
        return None;
    }
    spec.scroll_into_view = true;
    spec.user_event = Some(event.into());
    if event == "delete.selection" {
        spec.effects
            .push(EditorView::announce(state.phrase("Selection deleted", &[])));
    }
    Some(update(state, spec))
}

// Run a deletion on the view and dispatch its transaction.
fn delete_in_view(
    view: &mut EditorView,
    by: impl Fn(&EditorView, &SelectionRange) -> usize,
) -> bool {
    let target = &*view;
    let Some(tr) = delete_by(target.state(), Some(target), |range| by(target, range)) else {
        return false;
    };
    dispatch(view, tr);
    true
}

// Whether `s` holds a variation selector (U+FE00 to U+FE0F).
fn has_variation_selector(s: &str) -> bool {
    s.chars().any(|c| ('\u{fe00}'..='\u{fe0f}').contains(&c))
}

// `deleteByChar`: the position a deletion by character from `range`
// reaches, or the indentation unit before the cursor with
// `by_indent_unit`.
fn delete_char_target(
    state: &EditorState,
    range: &SelectionRange,
    forward: bool,
    by_indent_unit: bool,
) -> usize {
    let mut pos = range.from();
    let doc = state.doc();
    let line = doc.line_at(pos);
    let before = &line.text[..pos - line.from];
    // Upstream checks `pos < line.from + 200` in UTF-16 units; the text
    // before is spaces and tabs whenever the check matters, so bytes
    // count the same.
    if by_indent_unit
        && !forward
        && pos > line.from
        && pos < line.from + 200
        && before.bytes().all(|b| b == b' ' || b == b'\t')
    {
        if before.ends_with('\t') {
            return pos - 1;
        }
        let col = count_column(before, state.tab_size(), None);
        let unit = get_indent_unit(state);
        let drop = if col.is_multiple_of(unit) {
            unit
        } else {
            col % unit
        };
        let bytes = before.as_bytes();
        let mut i = 0;
        while i < drop && i < bytes.len() && bytes[bytes.len() - 1 - i] == b' ' {
            pos -= 1;
            i += 1;
        }
        pos
    } else {
        let mut target =
            find_cluster_break(line.text, pos - line.from, forward, forward) + line.from;
        if target == pos && line.number != (if forward { doc.lines() } else { 1 }) {
            target = if forward { target + 1 } else { target - 1 };
        } else if !forward
            && has_variation_selector(&line.text[target - line.from..pos - line.from])
        {
            target = find_cluster_break(line.text, target - line.from, false, false) + line.from;
        }
        target
    }
}

fn delete_by_char(view: &mut EditorView, forward: bool, by_indent_unit: bool) -> bool {
    delete_in_view(view, |view, range| {
        delete_char_target(view.state(), range, forward, by_indent_unit)
    })
}

/// Delete the selection, or, from a cursor, the character or indentation
/// unit before it (`deleteCharBackward`).
pub fn delete_char_backward(view: &mut EditorView) -> bool {
    delete_by_char(view, false, true)
}

/// Delete the selection, or the character before the cursor, never a
/// whole indentation unit (`deleteCharBackwardStrict`).
pub fn delete_char_backward_strict(view: &mut EditorView) -> bool {
    delete_by_char(view, false, false)
}

/// Delete the selection, or the character after the cursor
/// (`deleteCharForward`).
pub fn delete_char_forward(view: &mut EditorView) -> bool {
    delete_by_char(view, true, false)
}

// `deleteByGroup`: the end of the group next to the cursor, skipping a
// single space first.
fn delete_group_target(state: &EditorState, range: &SelectionRange, forward: bool) -> usize {
    let mut pos = range.head();
    let doc = state.doc();
    let line = doc.line_at(pos);
    let categorize = state.char_categorizer(pos);
    let mut cat: Option<CharCategory> = None;
    loop {
        if pos == (if forward { line.to } else { line.from }) {
            if pos == range.head() && line.number != (if forward { doc.lines() } else { 1 }) {
                pos = if forward { pos + 1 } else { pos - 1 };
            }
            break;
        }
        let next = find_cluster_break(line.text, pos - line.from, forward, true) + line.from;
        let next_char = &line.text[pos.min(next) - line.from..pos.max(next) - line.from];
        let next_cat = categorize(next_char);
        if cat.is_some_and(|cat| cat != next_cat) {
            break;
        }
        if next_char != " " || pos != range.head() {
            cat = Some(next_cat);
        }
        pos = next;
    }
    pos
}

fn delete_by_group(state: &EditorState, dispatch: Dispatch, forward: bool) -> bool {
    match delete_by(state, None, |range| {
        delete_group_target(state, range, forward)
    }) {
        Some(tr) => {
            dispatch(tr);
            true
        }
        None => false,
    }
}

/// Delete the selection, or backward to the end of the next group,
/// skipping whitespace only when it is a single space
/// (`deleteGroupBackward`).
///
/// Upstream's command also skips the view's atomic ranges when its target
/// is a view; this [`StateCommand`](editor_state::StateCommand) has no
/// view, so a key binding gets that from [`delete_group_backward_view`].
pub fn delete_group_backward(state: &EditorState, dispatch: Dispatch) -> bool {
    delete_by_group(state, dispatch, false)
}

/// Delete the selection, or forward to the end of the next group
/// (`deleteGroupForward`). See [`delete_group_backward`] for its view
/// form, [`delete_group_forward_view`].
pub fn delete_group_forward(state: &EditorState, dispatch: Dispatch) -> bool {
    delete_by_group(state, dispatch, true)
}

/// [`delete_group_backward`] with the view as its target, as upstream's
/// command runs in a key binding: deletions skip the view's atomic
/// ranges.
pub fn delete_group_backward_view(view: &mut EditorView) -> bool {
    delete_in_view(view, |view, range| {
        delete_group_target(view.state(), range, false)
    })
}

/// [`delete_group_forward`] with the view as its target.
pub fn delete_group_forward_view(view: &mut EditorView) -> bool {
    delete_in_view(view, |view, range| {
        delete_group_target(view.state(), range, true)
    })
}

/// Delete the selection, or forward over the next group and the
/// whitespace after it, in the Windows style (`deleteGroupForwardWin`).
pub fn delete_group_forward_win(view: &mut EditorView) -> bool {
    delete_in_view(view, |view, range| {
        let head = range.head();
        view.move_by_char_by(range, true, move |start| to_group_start(view, head, start))
            .head()
    })
}

/// Delete the selection, or to the end of the line, or the line break
/// after a cursor at the end of the line (`deleteToLineEnd`).
pub fn delete_to_line_end(view: &mut EditorView) -> bool {
    delete_in_view(view, |view, range| {
        let line_end = view.line_block_at(range.head()).to();
        if range.head() < line_end {
            line_end
        } else {
            view.state().doc().len().min(range.head() + 1)
        }
    })
}

/// Delete the selection, or to the start of the line, or the line break
/// before a cursor at the start of the line (`deleteToLineStart`).
pub fn delete_to_line_start(view: &mut EditorView) -> bool {
    delete_in_view(view, |view, range| {
        let line_start = view.line_block_at(range.head()).from;
        if range.head() > line_start {
            line_start
        } else {
            range.head().saturating_sub(1)
        }
    })
}

/// Delete the selection, or to the start of the line or the wrap point
/// before the cursor (`deleteLineBoundaryBackward`). Wrap points wait for
/// Div gaps 5 and 6 (see [`EditorView::move_to_line_boundary`]).
pub fn delete_line_boundary_backward(view: &mut EditorView) -> bool {
    delete_in_view(view, |view, range| {
        let line_start = view.move_to_line_boundary(range, false, true).head();
        if range.head() > line_start {
            line_start
        } else {
            range.head().saturating_sub(1)
        }
    })
}

/// Delete the selection, or to the end of the line or the wrap point after
/// the cursor (`deleteLineBoundaryForward`).
pub fn delete_line_boundary_forward(view: &mut EditorView) -> bool {
    delete_in_view(view, |view, range| {
        let line_end = view.move_to_line_boundary(range, true, true).head();
        if range.head() < line_end {
            line_end
        } else {
            view.state().doc().len().min(range.head() + 1)
        }
    })
}

// `changeBySelectedLine`: call `f` once for each line a selection range
// touches, keeping each range on its text.
fn change_by_selected_line(
    state: &EditorState,
    mut f: impl FnMut(&Line, &mut Vec<ChangeSpec>, &SelectionRange),
) -> TransactionSpec {
    let mut at_line = 0;
    change_by_range(state, |range| {
        let mut changes = Vec::new();
        let mut pos = range.from();
        while pos <= range.to() {
            let line = state.doc().line_at(pos);
            if line.number > at_line && (range.is_empty() || range.to() > line.from) {
                f(&line, &mut changes, range);
                at_line = line.number;
            }
            pos = line.to + 1;
        }
        let set = state
            .changes(Some(ChangeSpec::List(changes)))
            .expect("changes inside the document");
        RangeChange {
            range: EditorSelection::range(
                set.map_pos(range.anchor(), 1),
                set.map_pos(range.head(), 1),
            ),
            changes: Some(ChangeSpec::Set(set)),
            effects: Vec::new(),
        }
    })
}

// The leading JavaScript `\s` of `text`, in bytes.
fn leading_space(text: &str) -> usize {
    text.len() - text.trim_start_matches(is_js_space).len()
}

/// Add a unit of indentation ([`INDENT_UNIT`]) to every selected line
/// (`indentMore`).
pub fn indent_more(state: &EditorState, dispatch: Dispatch) -> bool {
    if state.read_only() {
        return false;
    }
    let unit = state.facet(&INDENT_UNIT);
    let mut spec = change_by_selected_line(state, |line, changes, _| {
        changes.push(ChangeSpec::insert(line.from, unit.as_str()));
    });
    spec.user_event = Some("input.indent".into());
    dispatch(update(state, spec));
    true
}

/// Remove a unit of indentation from every selected line (`indentLess`).
pub fn indent_less(state: &EditorState, dispatch: Dispatch) -> bool {
    if state.read_only() {
        return false;
    }
    let mut spec = change_by_selected_line(state, |line, changes, _| {
        let space = &line.text[..leading_space(line.text)];
        if space.is_empty() {
            return;
        }
        let col = count_column(space, state.tab_size(), None);
        let insert = indent_string(state, col.saturating_sub(get_indent_unit(state)));
        // Upstream compares UTF-16 units; comparing characters keeps
        // `keep` on a character boundary.
        let keep: usize = space
            .chars()
            .zip(insert.chars())
            .take_while(|(a, b)| a == b)
            .map(|(a, _)| a.len_utf8())
            .sum();
        changes.push(ChangeSpec::replace(
            line.from + keep,
            line.from + space.len(),
            &insert[keep..],
        ));
    });
    spec.user_event = Some("delete.dedent".into());
    dispatch(update(state, spec));
    true
}

/// Turn tab-focus mode on or off (`toggleTabFocusMode`): while on, Tab
/// and Shift-Tab run no key bindings, so the keyboard can move focus out
/// of the editor.
pub fn toggle_tab_focus_mode(view: &mut EditorView) -> bool {
    view.set_tab_focus_mode(TabFocusMode::Toggle);
    true
}

/// Turn tab-focus mode on for two seconds or until another key is pressed
/// (`temporarilySetTabFocusMode`).
pub fn temporarily_set_tab_focus_mode(view: &mut EditorView) -> bool {
    view.set_tab_focus_mode(TabFocusMode::For(2000.0));
    true
}

/// Insert a tab at the cursor, or indent the selected lines with
/// [`indent_more`] when something is selected (`insertTab`).
pub fn insert_tab(state: &EditorState, dispatch: Dispatch) -> bool {
    if state.selection().ranges().iter().any(|r| !r.is_empty()) {
        return indent_more(state, dispatch);
    }
    let mut spec = state.replace_selection("\t");
    spec.scroll_into_view = true;
    spec.user_event = Some("input".into());
    dispatch(update(state, spec));
    true
}
