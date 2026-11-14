//! Port of the parts of `commands.ts` that need only the state: logical
//! character motion, document and line selection, and the line and
//! whitespace editing commands. The rest need the view or
//! `@codemirror/language` and come with those ports.

use editor_state::{
    find_cluster_break, ChangeSpec, EditorSelection, EditorState, Insertion, RangeChange,
    SelectionRange, Text, Transaction, TransactionSpec,
};

type Dispatch<'a> = &'a mut dyn FnMut(Transaction);

// JavaScript's `\s`.
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

fn update_sel(
    sel: &EditorSelection,
    by: impl FnMut(&SelectionRange) -> SelectionRange,
) -> EditorSelection {
    EditorSelection::create(sel.ranges().iter().map(by).collect(), sel.main_index())
        .expect("mapped ranges form a selection")
}

fn set_sel(state: &EditorState, selection: EditorSelection) -> Transaction {
    state
        .update(TransactionSpec {
            selection: Some(selection),
            scroll_into_view: true,
            user_event: Some("select".into()),
            ..Default::default()
        })
        .expect("a selection inside the document")
}

fn update(state: &EditorState, spec: TransactionSpec) -> Transaction {
    state
        .update(spec)
        .expect("command changes fit the document")
}

fn move_sel(
    state: &EditorState,
    dispatch: Dispatch,
    how: impl FnMut(&SelectionRange) -> SelectionRange,
) -> bool {
    let selection = update_sel(state.selection(), how);
    if selection.eq_with(state.selection(), true) {
        return false;
    }
    dispatch(set_sel(state, selection));
    true
}

fn range_end(range: &SelectionRange, forward: bool) -> SelectionRange {
    EditorSelection::cursor(if forward { range.to() } else { range.from() })
}

fn by_char_logical(state: &EditorState, range: &SelectionRange, forward: bool) -> SelectionRange {
    let mut pos = range.head();
    let line = state.doc().line_at(pos);
    if pos == if forward { line.to } else { line.from } {
        pos = if forward {
            state.doc().len().min(line.to + 1)
        } else {
            line.from.saturating_sub(1)
        };
    } else {
        pos = line.from + find_cluster_break(line.text, pos - line.from, forward, true);
    }
    EditorSelection::cursor_with(pos, if forward { -1 } else { 1 }, None, None)
}

fn move_by_char_logical(state: &EditorState, dispatch: Dispatch, forward: bool) -> bool {
    move_sel(state, dispatch, |range| {
        if range.is_empty() {
            by_char_logical(state, range, forward)
        } else {
            range_end(range, forward)
        }
    })
}

/// Move the selection one character forward, in logical
/// (non-text-direction-aware) string order (`cursorCharForwardLogical`).
pub fn cursor_char_forward_logical(state: &EditorState, dispatch: Dispatch) -> bool {
    move_by_char_logical(state, dispatch, true)
}

/// Move the selection one character backward, in logical string order
/// (`cursorCharBackwardLogical`).
pub fn cursor_char_backward_logical(state: &EditorState, dispatch: Dispatch) -> bool {
    move_by_char_logical(state, dispatch, false)
}

fn extend_sel(
    state: &EditorState,
    dispatch: Dispatch,
    forward: bool,
    mut how: impl FnMut(&SelectionRange) -> SelectionRange,
) -> bool {
    let selection = update_sel(state.selection(), |range| {
        let mut range = *range;
        if range.undirectional() && (range.head() >= range.anchor()) != forward {
            range = EditorSelection::range(range.head(), range.anchor());
        }
        let head = how(&range);
        EditorSelection::range_with(
            range.anchor(),
            head.head(),
            head.goal_column(),
            head.bidi_level().filter(|&level| level != 0),
            head.assoc(),
        )
    });
    if selection.eq_with(state.selection(), false) {
        return false;
    }
    dispatch(set_sel(state, selection));
    true
}

/// Move the selection head one character forward in logical string
/// order (`selectCharForwardLogical`).
pub fn select_char_forward_logical(state: &EditorState, dispatch: Dispatch) -> bool {
    extend_sel(state, dispatch, true, |range| {
        by_char_logical(state, range, true)
    })
}

/// Move the selection head one character backward in logical string
/// order (`selectCharBackwardLogical`).
pub fn select_char_backward_logical(state: &EditorState, dispatch: Dispatch) -> bool {
    extend_sel(state, dispatch, false, |range| {
        by_char_logical(state, range, false)
    })
}

/// Move the selection to the start of the document (`cursorDocStart`).
pub fn cursor_doc_start(state: &EditorState, dispatch: Dispatch) -> bool {
    dispatch(set_sel(state, EditorSelection::single(0, 0)));
    true
}

/// Move the selection to the end of the document (`cursorDocEnd`).
pub fn cursor_doc_end(state: &EditorState, dispatch: Dispatch) -> bool {
    let end = state.doc().len();
    dispatch(set_sel(state, EditorSelection::single(end, end)));
    true
}

/// Move the selection head to the start of the document
/// (`selectDocStart`).
pub fn select_doc_start(state: &EditorState, dispatch: Dispatch) -> bool {
    let anchor = state.selection().main().anchor();
    dispatch(set_sel(state, EditorSelection::single(anchor, 0)));
    true
}

/// Move the selection head to the end of the document (`selectDocEnd`).
pub fn select_doc_end(state: &EditorState, dispatch: Dispatch) -> bool {
    let anchor = state.selection().main().anchor();
    dispatch(set_sel(
        state,
        EditorSelection::single(anchor, state.doc().len()),
    ));
    true
}

/// Select the whole document (`selectAll`).
pub fn select_all(state: &EditorState, dispatch: Dispatch) -> bool {
    dispatch(update(
        state,
        TransactionSpec {
            selection: Some(EditorSelection::single(0, state.doc().len())),
            user_event: Some("select".into()),
            ..Default::default()
        },
    ));
    true
}

/// Expand the selection to cover whole lines (`selectLine`).
pub fn select_line(state: &EditorState, dispatch: Dispatch) -> bool {
    let len = state.doc().len();
    let ranges = selected_line_blocks(state)
        .iter()
        .map(|block| EditorSelection::undirectional_range(block.from, (block.to + 1).min(len)))
        .collect();
    dispatch(update(
        state,
        TransactionSpec {
            selection: Some(
                EditorSelection::create(ranges, 0).expect("line blocks form a selection"),
            ),
            user_event: Some("select".into()),
            ..Default::default()
        },
    ));
    true
}

/// With several selection ranges, keep only the main one; otherwise turn
/// a non-empty selection into a cursor (`simplifySelection`).
pub fn simplify_selection(state: &EditorState, dispatch: Dispatch) -> bool {
    let cur = state.selection();
    let selection = if cur.ranges().len() > 1 {
        cur.main()
    } else if !cur.main().is_empty() {
        EditorSelection::cursor(cur.main().head())
    } else {
        return false;
    };
    let selection = EditorSelection::create(vec![selection], 0).expect("one range");
    dispatch(set_sel(state, selection));
    true
}

/// Delete all whitespace directly before a line end
/// (`deleteTrailingWhitespace`).
pub fn delete_trailing_whitespace(state: &EditorState, dispatch: Dispatch) -> bool {
    if state.read_only() {
        return false;
    }
    let mut changes = Vec::new();
    let mut pos = 0;
    let mut prev = "";
    let mut iter = state.doc().iter(1);
    loop {
        use editor_state::TextIterator;
        iter.advance(0);
        if iter.line_break() || iter.done() {
            let trailing = prev.trim_end_matches(is_js_space).len();
            if trailing < prev.len() {
                changes.push(ChangeSpec::delete(pos - (prev.len() - trailing), pos));
            }
            if iter.done() {
                break;
            }
            prev = "";
        } else {
            prev = iter.value();
        }
        pos += iter.value().len();
    }
    if changes.is_empty() {
        return false;
    }
    dispatch(update(
        state,
        TransactionSpec {
            changes: Some(ChangeSpec::List(changes)),
            user_event: Some("delete".into()),
            ..Default::default()
        },
    ));
    true
}

fn change_by_range(
    state: &EditorState,
    f: impl FnMut(&SelectionRange) -> RangeChange,
) -> TransactionSpec {
    state
        .change_by_range(f)
        .expect("changes inside the document")
}

fn range_only(range: &SelectionRange) -> RangeChange {
    RangeChange {
        range: *range,
        changes: None,
        effects: Vec::new(),
    }
}

fn replace(range: SelectionRange, from: usize, to: usize, insert: Insertion) -> RangeChange {
    RangeChange {
        range,
        changes: Some(ChangeSpec::Change(editor_state::Change {
            from,
            to,
            insert: Some(insert),
        })),
        effects: Vec::new(),
    }
}

/// Replace each selection range with a line break, leaving the cursor on
/// the line before the break (`splitLine`).
pub fn split_line(state: &EditorState, dispatch: Dispatch) -> bool {
    if state.read_only() {
        return false;
    }
    let break_text = Text::of(&["", ""]).expect("two lines");
    let mut spec = change_by_range(state, |range| {
        replace(
            EditorSelection::cursor(range.from()),
            range.from(),
            range.to(),
            Insertion::Text(break_text.clone()),
        )
    });
    spec.scroll_into_view = true;
    spec.user_event = Some("input".into());
    dispatch(update(state, spec));
    true
}

fn spec_changes_empty(spec: &TransactionSpec) -> bool {
    matches!(&spec.changes, Some(ChangeSpec::Set(set)) if set.is_empty())
}

/// Swap the characters before and after each cursor (`transposeChars`).
pub fn transpose_chars(state: &EditorState, dispatch: Dispatch) -> bool {
    if state.read_only() {
        return false;
    }
    let doc = state.doc();
    let mut spec = change_by_range(state, |range| {
        if !range.is_empty() || range.from() == 0 || range.from() == doc.len() {
            return range_only(range);
        }
        let pos = range.from();
        let line = doc.line_at(pos);
        let from = if pos == line.from {
            pos - 1
        } else {
            find_cluster_break(line.text, pos - line.from, false, true) + line.from
        };
        let to = if pos == line.to {
            pos + 1
        } else {
            find_cluster_break(line.text, pos - line.from, true, true) + line.from
        };
        replace(
            EditorSelection::cursor(to),
            from,
            to,
            Insertion::Text(doc.slice(pos, to).append(&doc.slice(from, pos))),
        )
    });
    if spec_changes_empty(&spec) {
        return false;
    }
    spec.scroll_into_view = true;
    spec.user_event = Some("move.character".into());
    dispatch(update(state, spec));
    true
}

struct LineBlock {
    from: usize,
    to: usize,
    ranges: Vec<SelectionRange>,
}

fn selected_line_blocks(state: &EditorState) -> Vec<LineBlock> {
    let doc = state.doc();
    let mut blocks: Vec<LineBlock> = Vec::new();
    let mut upto = 0;
    for range in state.selection().ranges() {
        let start_line = doc.line_at(range.from());
        let mut end_line = doc.line_at(range.to());
        if !range.is_empty() && range.to() == end_line.from {
            end_line = doc.line_at(range.to() - 1);
        }
        match blocks.last_mut() {
            Some(prev) if upto >= start_line.number => {
                prev.to = end_line.to;
                prev.ranges.push(*range);
            }
            _ => blocks.push(LineBlock {
                from: start_line.from,
                to: end_line.to,
                ranges: vec![*range],
            }),
        }
        upto = end_line.number + 1;
    }
    blocks
}

fn move_line(state: &EditorState, dispatch: Dispatch, forward: bool) -> bool {
    if state.read_only() {
        return false;
    }
    let doc = state.doc();
    let mut changes = Vec::new();
    let mut ranges = Vec::new();
    for block in selected_line_blocks(state) {
        if if forward {
            block.to == doc.len()
        } else {
            block.from == 0
        } {
            continue;
        }
        let next_line = doc.line_at(if forward {
            block.to + 1
        } else {
            block.from - 1
        });
        let size = next_line.len() + 1;
        if forward {
            changes.push(ChangeSpec::delete(block.to, next_line.to));
            changes.push(ChangeSpec::insert(
                block.from,
                format!("{}{}", next_line.text, state.line_break()),
            ));
            for r in &block.ranges {
                ranges.push(EditorSelection::range(
                    doc.len().min(r.anchor() + size),
                    doc.len().min(r.head() + size),
                ));
            }
        } else {
            changes.push(ChangeSpec::delete(next_line.from, block.from));
            changes.push(ChangeSpec::insert(
                block.to,
                format!("{}{}", state.line_break(), next_line.text),
            ));
            for r in &block.ranges {
                ranges.push(EditorSelection::range(r.anchor() - size, r.head() - size));
            }
        }
    }
    if changes.is_empty() {
        return false;
    }
    dispatch(update(
        state,
        TransactionSpec {
            changes: Some(ChangeSpec::List(changes)),
            scroll_into_view: true,
            selection: Some(
                EditorSelection::create(ranges, state.selection().main_index())
                    .expect("moved ranges form a selection"),
            ),
            user_event: Some("move.line".into()),
            ..Default::default()
        },
    ));
    true
}

/// Move the selected lines up one line (`moveLineUp`).
pub fn move_line_up(state: &EditorState, dispatch: Dispatch) -> bool {
    move_line(state, dispatch, false)
}

/// Move the selected lines down one line (`moveLineDown`).
pub fn move_line_down(state: &EditorState, dispatch: Dispatch) -> bool {
    move_line(state, dispatch, true)
}

fn copy_line(state: &EditorState, dispatch: Dispatch, forward: bool) -> bool {
    if state.read_only() {
        return false;
    }
    let doc = state.doc();
    let changes = selected_line_blocks(state)
        .iter()
        .map(|block| {
            let text = doc.slice_string(block.from, block.to);
            if forward {
                ChangeSpec::insert(block.from, format!("{text}{}", state.line_break()))
            } else {
                ChangeSpec::insert(block.to, format!("{}{text}", state.line_break()))
            }
        })
        .collect();
    let change_set = state
        .changes(Some(ChangeSpec::List(changes)))
        .expect("insertions inside the document");
    let selection = state
        .selection()
        .map(change_set.desc(), if forward { 1 } else { -1 });
    dispatch(update(
        state,
        TransactionSpec {
            changes: Some(ChangeSpec::Set(change_set)),
            selection: Some(selection),
            scroll_into_view: true,
            user_event: Some("input.copyline".into()),
            ..Default::default()
        },
    ));
    true
}

/// Copy the selected lines, keeping the selection in the top copy
/// (`copyLineUp`).
pub fn copy_line_up(state: &EditorState, dispatch: Dispatch) -> bool {
    copy_line(state, dispatch, false)
}

/// Copy the selected lines, keeping the selection in the bottom copy
/// (`copyLineDown`).
pub fn copy_line_down(state: &EditorState, dispatch: Dispatch) -> bool {
    copy_line(state, dispatch, true)
}

/// Replace the selection with a line break (`insertNewline`).
pub fn insert_newline(state: &EditorState, dispatch: Dispatch) -> bool {
    let mut spec = state.replace_selection(state.line_break());
    spec.scroll_into_view = true;
    spec.user_event = Some("input".into());
    dispatch(update(state, spec));
    true
}

/// Replace the selection with a line break and the indentation of the
/// line it starts on (`insertNewlineKeepIndent`).
pub fn insert_newline_keep_indent(state: &EditorState, dispatch: Dispatch) -> bool {
    let doc = state.doc();
    let mut spec = change_by_range(state, |range| {
        let text = doc.line_at(range.from()).text;
        let indent = &text[..text.len() - text.trim_start_matches(is_js_space).len()];
        replace(
            EditorSelection::cursor(range.from() + indent.len() + 1),
            range.from(),
            range.to(),
            Insertion::Str(format!("{}{indent}", state.line_break())),
        )
    });
    spec.scroll_into_view = true;
    spec.user_event = Some("input".into());
    dispatch(update(state, spec));
    true
}
