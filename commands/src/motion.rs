//! Port of the cursor and selection motion of `commands.ts` that needs
//! the view: by character, group and subword in visual order, and to line
//! boundaries. Each command takes the view, as upstream's `Command` does,
//! so `editor_view::command` makes a key binding's command of it.
//!
//! Vertical and page motion (`cursorLineDown`, `cursorPageUp` and the
//! like) need the view's coordinates, which wait for Div's Range geometry
//! and `caretPositionFromPoint` (gaps 5 and 6); syntax motion needs
//! `@codemirror/language`.

use editor_state::{CharCategory, EditorSelection, SelectionRange, Transaction};
use editor_view::{ByChar, Direction, EditorView};

use crate::commands::{extended, is_js_space, range_end, set_sel, update_sel};

// Dispatch `tr` to the view. An error from the view's DOM is logged, as
// `editor_view::state_command` logs one.
fn dispatch(view: &mut EditorView, tr: Transaction) {
    if let Err(error) = view.dispatch(tr) {
        eprintln!("CodeMirror command: {error}");
    }
}

// `moveSel`.
fn move_sel(
    view: &mut EditorView,
    how: impl Fn(&EditorView, &SelectionRange) -> SelectionRange,
) -> bool {
    let selection = update_sel(view.state().selection(), |range| how(view, range));
    if selection.eq_with(view.state().selection(), true) {
        return false;
    }
    let tr = set_sel(view.state(), selection);
    dispatch(view, tr);
    true
}

// `extendSel`.
fn extend_sel(
    view: &mut EditorView,
    forward: bool,
    how: impl Fn(&EditorView, &SelectionRange) -> SelectionRange,
) -> bool {
    let selection = extended(view.state().selection(), forward, |range| how(view, range));
    if selection.eq_with(view.state().selection(), false) {
        return false;
    }
    let tr = set_sel(view.state(), selection);
    dispatch(view, tr);
    true
}

// `ltrAtCursor`.
fn ltr_at_cursor(view: &EditorView) -> bool {
    view.text_direction_at(view.state().selection().main().head()) == Direction::Ltr
}

fn cursor_by_char(view: &mut EditorView, forward: bool) -> bool {
    move_sel(view, |view, range| {
        if range.is_empty() {
            view.move_by_char(range, forward)
        } else {
            range_end(range, forward)
        }
    })
}

/// Move the selection one character to the left, which is backward in
/// left-to-right text and forward in right-to-left text
/// (`cursorCharLeft`).
pub fn cursor_char_left(view: &mut EditorView) -> bool {
    let forward = !ltr_at_cursor(view);
    cursor_by_char(view, forward)
}

/// Move the selection one character to the right (`cursorCharRight`).
pub fn cursor_char_right(view: &mut EditorView) -> bool {
    let forward = ltr_at_cursor(view);
    cursor_by_char(view, forward)
}

/// Move the selection one character forward (`cursorCharForward`).
pub fn cursor_char_forward(view: &mut EditorView) -> bool {
    cursor_by_char(view, true)
}

/// Move the selection one character backward (`cursorCharBackward`).
pub fn cursor_char_backward(view: &mut EditorView) -> bool {
    cursor_by_char(view, false)
}

fn cursor_by_group(view: &mut EditorView, forward: bool) -> bool {
    move_sel(view, |view, range| {
        if range.is_empty() {
            view.move_by_group(range, forward)
        } else {
            range_end(range, forward)
        }
    })
}

/// Move the selection to the left across one group of word or non-word
/// (but also non-space) characters (`cursorGroupLeft`).
pub fn cursor_group_left(view: &mut EditorView) -> bool {
    let forward = !ltr_at_cursor(view);
    cursor_by_group(view, forward)
}

/// Move the selection one group to the right (`cursorGroupRight`).
pub fn cursor_group_right(view: &mut EditorView) -> bool {
    let forward = ltr_at_cursor(view);
    cursor_by_group(view, forward)
}

/// Move the selection one group forward (`cursorGroupForward`).
pub fn cursor_group_forward(view: &mut EditorView) -> bool {
    cursor_by_group(view, true)
}

/// Move the selection one group backward (`cursorGroupBackward`).
pub fn cursor_group_backward(view: &mut EditorView) -> bool {
    cursor_by_group(view, false)
}

// `toGroupStart`: move over the group `start` begins and the whitespace
// after it.
fn to_group_start<'a>(view: &'a EditorView, pos: usize, start: &str) -> ByChar<'a> {
    let categorize = view.state().char_categorizer(pos);
    let cat = categorize(start);
    let mut initial = cat != CharCategory::Space;
    Box::new(move |next: &str| {
        let next_cat = categorize(next);
        if next_cat != CharCategory::Space {
            return initial && next_cat == cat;
        }
        initial = false;
        true
    })
}

/// Move the cursor one group forward in the default Windows style, to
/// the start of the next group (`cursorGroupForwardWin`).
pub fn cursor_group_forward_win(view: &mut EditorView) -> bool {
    move_sel(view, |view, range| {
        if range.is_empty() {
            let head = range.head();
            view.move_by_char_by(range, true, move |start| to_group_start(view, head, start))
        } else {
            range_end(range, true)
        }
    })
}

// A character category, with word punctuation apart (upstream's -1).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Cat {
    Of(CharCategory),
    WordPunctuation,
}

// What `moveBySubword`'s steps keep.
struct Subword {
    cat: Cat,
    pos: usize,
    done: bool,
    saw_upper: bool,
    saw_lower: bool,
}

// The text of the one position after `pos` (`sliceDoc(pos, pos + 1)`).
fn char_after(view: &EditorView, pos: usize) -> String {
    let doc = view.state().doc();
    let line = doc.line_at(pos);
    match line.text[pos - line.from..].chars().next() {
        Some(ch) => ch.to_string(),
        None if pos < doc.len() => "\n".to_string(),
        None => String::new(),
    }
}

// `moveBySubword`.
fn move_by_subword(view: &EditorView, range: &SelectionRange, forward: bool) -> SelectionRange {
    let categorize = view.state().char_categorizer(range.from());
    let mut s = Subword {
        cat: Cat::Of(CharCategory::Space),
        pos: range.from(),
        done: false,
        saw_upper: false,
        saw_lower: false,
    };
    let step = {
        move |next: &str| -> bool {
            if s.done {
                return false;
            }
            if forward {
                s.pos += next.len();
            } else {
                s.pos -= next.len();
            }
            let mut next_cat = Cat::Of(categorize(next));
            let ascii = next.chars().next().is_some_and(|c| c.is_ascii());
            if next_cat == Cat::Of(CharCategory::Word)
                && ascii
                && next.chars().any(|c| !c.is_ascii_alphanumeric())
            {
                // Treat word punctuation specially.
                next_cat = Cat::WordPunctuation;
            }
            if s.cat == Cat::Of(CharCategory::Space) {
                s.cat = next_cat;
            }
            if s.cat != next_cat {
                return false;
            }
            if s.cat == Cat::Of(CharCategory::Word) {
                if next.to_lowercase() == next {
                    if !forward && s.saw_upper {
                        return false;
                    }
                    s.saw_lower = true;
                } else if s.saw_lower {
                    if forward {
                        return false;
                    }
                    s.done = true;
                } else {
                    if s.saw_upper && forward {
                        let ahead = char_after(view, s.pos);
                        if categorize(&ahead) == CharCategory::Word && ahead.to_lowercase() == ahead
                        {
                            return false;
                        }
                    }
                    s.saw_upper = true;
                }
            }
            true
        }
    };
    // div-gap: 30 `Intl.Segmenter` word segmentation. Upstream, where the
    // segmenter exists, counts the steps and splits a run of CJK word
    // characters at the word boundaries the segmenter finds; without it,
    // as upstream without one, the run moves as one subword.
    view.move_by_char_by(range, forward, move |start| {
        let mut step = step;
        step(start);
        Box::new(step)
    })
}

fn cursor_by_subword(view: &mut EditorView, forward: bool) -> bool {
    move_sel(view, |view, range| {
        if range.is_empty() {
            move_by_subword(view, range, forward)
        } else {
            range_end(range, forward)
        }
    })
}

/// Move the selection one group or camel-case subword forward
/// (`cursorSubwordForward`).
pub fn cursor_subword_forward(view: &mut EditorView) -> bool {
    cursor_by_subword(view, true)
}

/// Move the selection one group or camel-case subword backward
/// (`cursorSubwordBackward`).
pub fn cursor_subword_backward(view: &mut EditorView) -> bool {
    cursor_by_subword(view, false)
}

// `moveByLineBoundary`: to the next wrap point or the line's end, or
// backward to the end of the line's indentation before its start.
fn move_by_line_boundary(
    view: &EditorView,
    start: &SelectionRange,
    forward: bool,
) -> SelectionRange {
    let line = view.line_block_at(start.head());
    let mut moved = view.move_to_line_boundary(start, forward, true);
    if moved.head() == start.head() && moved.head() != (if forward { line.to() } else { line.from })
    {
        moved = view.move_to_line_boundary(start, forward, false);
    }
    if !forward && moved.head() == line.from && line.length > 0 {
        // `/^\s*/` over the line's first 100 UTF-16 units.
        let text = view.state().slice_doc(line.from, line.to());
        let mut units = 0;
        let mut space = 0;
        for ch in text.chars() {
            units += ch.len_utf16();
            if units > 100 || !is_js_space(ch) {
                break;
            }
            space += ch.len_utf8();
        }
        if space > 0 && start.head() != line.from + space {
            moved = EditorSelection::cursor(line.from + space);
        }
    }
    moved
}

/// Move the selection to the next line wrap point, or to the end of the
/// line if there is none left on it (`cursorLineBoundaryForward`).
pub fn cursor_line_boundary_forward(view: &mut EditorView) -> bool {
    move_sel(view, |view, range| move_by_line_boundary(view, range, true))
}

/// Move the selection to the previous line wrap point, or to the start
/// of the line, or, in an indented line, first to the end of the
/// indentation (`cursorLineBoundaryBackward`).
pub fn cursor_line_boundary_backward(view: &mut EditorView) -> bool {
    move_sel(view, |view, range| {
        move_by_line_boundary(view, range, false)
    })
}

/// Move the selection one line wrap point to the left
/// (`cursorLineBoundaryLeft`).
pub fn cursor_line_boundary_left(view: &mut EditorView) -> bool {
    move_sel(view, |view, range| {
        move_by_line_boundary(view, range, !ltr_at_cursor(view))
    })
}

/// Move the selection one line wrap point to the right
/// (`cursorLineBoundaryRight`).
pub fn cursor_line_boundary_right(view: &mut EditorView) -> bool {
    move_sel(view, |view, range| {
        move_by_line_boundary(view, range, ltr_at_cursor(view))
    })
}

/// Move the selection to the start of the line (`cursorLineStart`).
pub fn cursor_line_start(view: &mut EditorView) -> bool {
    move_sel(view, |view, range| {
        view.move_to_line_boundary(range, false, false)
    })
}

/// Move the selection to the end of the line (`cursorLineEnd`).
pub fn cursor_line_end(view: &mut EditorView) -> bool {
    move_sel(view, |view, range| {
        view.move_to_line_boundary(range, true, false)
    })
}

fn select_by_char(view: &mut EditorView, forward: bool) -> bool {
    extend_sel(view, forward, |view, range| {
        view.move_by_char(range, forward)
    })
}

/// Move the selection head one character to the left, leaving the anchor
/// in place (`selectCharLeft`).
pub fn select_char_left(view: &mut EditorView) -> bool {
    let forward = !ltr_at_cursor(view);
    select_by_char(view, forward)
}

/// Move the selection head one character to the right
/// (`selectCharRight`).
pub fn select_char_right(view: &mut EditorView) -> bool {
    let forward = ltr_at_cursor(view);
    select_by_char(view, forward)
}

/// Move the selection head one character forward (`selectCharForward`).
pub fn select_char_forward(view: &mut EditorView) -> bool {
    select_by_char(view, true)
}

/// Move the selection head one character backward
/// (`selectCharBackward`).
pub fn select_char_backward(view: &mut EditorView) -> bool {
    select_by_char(view, false)
}

fn select_by_group(view: &mut EditorView, forward: bool) -> bool {
    extend_sel(view, forward, |view, range| {
        view.move_by_group(range, forward)
    })
}

/// Move the selection head one group to the left (`selectGroupLeft`).
pub fn select_group_left(view: &mut EditorView) -> bool {
    let forward = !ltr_at_cursor(view);
    select_by_group(view, forward)
}

/// Move the selection head one group to the right (`selectGroupRight`).
pub fn select_group_right(view: &mut EditorView) -> bool {
    let forward = ltr_at_cursor(view);
    select_by_group(view, forward)
}

/// Move the selection head one group forward (`selectGroupForward`).
pub fn select_group_forward(view: &mut EditorView) -> bool {
    select_by_group(view, true)
}

/// Move the selection head one group backward (`selectGroupBackward`).
pub fn select_group_backward(view: &mut EditorView) -> bool {
    select_by_group(view, false)
}

/// Move the selection head one group forward in the default Windows
/// style, to the start of the next group (`selectGroupForwardWin`).
pub fn select_group_forward_win(view: &mut EditorView) -> bool {
    extend_sel(view, true, |view, range| {
        let head = range.head();
        view.move_by_char_by(range, true, move |start| to_group_start(view, head, start))
    })
}

fn select_by_subword(view: &mut EditorView, forward: bool) -> bool {
    extend_sel(view, forward, |view, range| {
        move_by_subword(view, range, forward)
    })
}

/// Move the selection head one group or camel-case subword forward
/// (`selectSubwordForward`).
pub fn select_subword_forward(view: &mut EditorView) -> bool {
    select_by_subword(view, true)
}

/// Move the selection head one group or subword backward
/// (`selectSubwordBackward`).
pub fn select_subword_backward(view: &mut EditorView) -> bool {
    select_by_subword(view, false)
}

/// Move the selection head to the next line boundary
/// (`selectLineBoundaryForward`).
pub fn select_line_boundary_forward(view: &mut EditorView) -> bool {
    extend_sel(view, true, |view, range| {
        move_by_line_boundary(view, range, true)
    })
}

/// Move the selection head to the previous line boundary
/// (`selectLineBoundaryBackward`).
pub fn select_line_boundary_backward(view: &mut EditorView) -> bool {
    extend_sel(view, false, |view, range| {
        move_by_line_boundary(view, range, false)
    })
}

/// Move the selection head one line boundary to the left
/// (`selectLineBoundaryLeft`).
pub fn select_line_boundary_left(view: &mut EditorView) -> bool {
    let forward = !ltr_at_cursor(view);
    extend_sel(view, forward, |view, range| {
        move_by_line_boundary(view, range, forward)
    })
}

/// Move the selection head one line boundary to the right
/// (`selectLineBoundaryRight`).
pub fn select_line_boundary_right(view: &mut EditorView) -> bool {
    let forward = ltr_at_cursor(view);
    extend_sel(view, forward, |view, range| {
        move_by_line_boundary(view, range, forward)
    })
}

/// Move the selection head to the start of the line (`selectLineStart`).
pub fn select_line_start(view: &mut EditorView) -> bool {
    extend_sel(view, false, |view, range| {
        EditorSelection::cursor(view.line_block_at(range.head()).from)
    })
}

/// Move the selection head to the end of the line (`selectLineEnd`).
pub fn select_line_end(view: &mut EditorView) -> bool {
    extend_sel(view, true, |view, range| {
        EditorSelection::cursor(view.line_block_at(range.head()).to())
    })
}
