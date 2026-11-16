//! A port of the cursor motion of `cursor.ts` that follows the text:
//! motion by character and group in visual order (`moveByChar`,
//! `byGroup`), to a line's boundary (`moveToLineBoundary`, `blockAt`), and
//! over atomic ranges (`skipAtomicRanges`, `skipAtoms`).
//!
//! The motion that needs the text's geometry comes later: vertical motion
//! (`moveVertically`), the wrap points `moveToLineBoundary` finds in a
//! wrapped line, and `posAtCoords` need Range geometry and
//! `caretPositionFromPoint` (gaps 5 and 6).

use editor_state::{CharCategory, EditorSelection, SelectionRange};

use crate::bidi::move_visually_over;
use crate::decoration::{BlockType, DecorationSet};
use crate::extension::ATOMIC_RANGES;
use crate::heightmap::{BlockInfo, BlockKind};
use crate::EditorView;

/// The predicate [`EditorView::move_by_char_by`] moves over clusters
/// with, after the first.
pub type ByChar<'a> = Box<dyn FnMut(&str) -> bool + 'a>;

// What makes the predicate from the first cluster (`by`).
pub(crate) type ByCharStart<'a> = Box<dyn FnOnce(&str) -> ByChar<'a> + 'a>;

fn block_type(block: &BlockInfo) -> Option<BlockType> {
    match block.kind() {
        BlockKind::Type(kind) => Some(kind),
        BlockKind::Blocks(_) => None,
    }
}

// `blockAt`: the block of the line at `pos` that holds it, preferring a
// text block on the `side` of `pos`.
pub(crate) fn block_at(view: &EditorView, pos: usize, side: i32) -> BlockInfo {
    let line = view.line_block_at(pos);
    let BlockKind::Blocks(blocks) = line.kind() else {
        return line;
    };
    let mut best: Option<&BlockInfo> = None;
    for block in blocks {
        if block.from > pos {
            break;
        }
        if block.to() < pos {
            continue;
        }
        if block.from < pos && block.to() > pos {
            return block.clone();
        }
        let better = match best {
            None => true,
            Some(best) => {
                block_type(block) == Some(BlockType::Text)
                    && (block_type(best) != block_type(block)
                        || (if side < 0 {
                            block.from < pos
                        } else {
                            block.to() > pos
                        }))
            }
        };
        if better {
            best = Some(block);
        }
    }
    best.cloned().unwrap_or_else(|| line.clone())
}

// `moveToLineBoundary`.
pub(crate) fn move_to_line_boundary(
    view: &EditorView,
    start: &SelectionRange,
    forward: bool,
    include_wrap: bool,
) -> SelectionRange {
    let side = if start.assoc() != 0 {
        i32::from(start.assoc())
    } else {
        -1
    };
    let block = block_at(view, start.head(), side);
    let wraps = include_wrap
        && block_type(&block) == Some(BlockType::Text)
        && (view.line_wrapping() || block.widget_line_breaks() > 0);
    if wraps {
        // div-gap: 5 Range geometry, 6 `caretPositionFromPoint`. Upstream
        // takes the cursor's coordinates (`coordsAtPos`) and finds the
        // position at the editor's edge on that visual line
        // (`posAndSideAtCoords`), the line's next wrap point. Without
        // them, the motion goes to the line's boundary, as upstream's does
        // when `coordsAtPos` finds nothing.
    }
    if forward {
        EditorSelection::cursor_with(block.to(), 1, None, None)
    } else {
        EditorSelection::cursor_with(block.from, -1, None, None)
    }
}

// `moveByChar`.
pub(crate) fn move_by_char<'a>(
    view: &'a EditorView,
    start: &SelectionRange,
    forward: bool,
    by: Option<ByCharStart<'a>>,
) -> SelectionRange {
    let doc = view.state().doc();
    let mut line = doc.line_at(start.head());
    let mut spans = view.bidi_spans(&line);
    let direction = view.text_direction_at(line.from);
    let mut by = by;
    let mut check: Option<ByChar<'a>> = None;
    let mut cur = *start;
    loop {
        let (next, over) = match move_visually_over(&line, &spans, direction, &cur, forward) {
            Some((next, over)) => (next, over),
            None => {
                if line.number == (if forward { doc.lines() } else { 1 }) {
                    return cur;
                }
                line = doc.line(if forward {
                    line.number + 1
                } else {
                    line.number - 1
                });
                spans = view.bidi_spans(&line);
                let next = if forward {
                    EditorSelection::cursor_with(line.from, -1, None, None)
                } else {
                    EditorSelection::cursor_with(line.to, 1, None, None)
                };
                (next, "\n")
            }
        };
        match &mut check {
            None => {
                let Some(by) = by.take() else {
                    return next;
                };
                check = Some(by(over));
            }
            Some(check) => {
                if !check(over) {
                    return cur;
                }
            }
        }
        cur = next;
    }
}

// `byGroup`: after `start`, move over characters of its category, or,
// from whitespace, of the first category after it.
pub(crate) fn by_group<'a>(view: &'a EditorView, pos: usize, start: &str) -> ByChar<'a> {
    let categorize = view.state().char_categorizer(pos);
    let mut cat = categorize(start);
    Box::new(move |next: &str| {
        let next_cat = categorize(next);
        if cat == CharCategory::Space {
            cat = next_cat;
        }
        cat == next_cat
    })
}

/// Move `pos` out of the atomic ranges in `atoms` that cover it, towards
/// `bias` (`skipAtomicRanges`): -1 to their start, 1 to their end, 0 to
/// the nearer end.
pub fn skip_atomic_ranges(atoms: &[DecorationSet], pos: usize, bias: i32) -> usize {
    let mut pos = pos;
    loop {
        let mut moved = 0;
        for set in atoms {
            set.between(pos.saturating_sub(1), pos + 1, |from, to, _| {
                if pos > from && pos < to {
                    let side = if moved != 0 {
                        moved
                    } else if bias != 0 {
                        bias
                    } else if pos - from < to - pos {
                        -1
                    } else {
                        1
                    };
                    pos = if side < 0 { from } else { to };
                    moved = side;
                }
                true
            });
        }
        if moved == 0 {
            return pos;
        }
    }
}

// `skipAtoms`: move a motion's result out of the view's atomic ranges, in
// the direction it moved.
pub(crate) fn skip_atoms(
    view: &EditorView,
    old_pos: &SelectionRange,
    pos: SelectionRange,
) -> SelectionRange {
    let atoms: Vec<DecorationSet> = view
        .state()
        .facet(&ATOMIC_RANGES)
        .iter()
        .map(|atoms| atoms(view))
        .collect();
    let bias = if old_pos.head() > pos.from() { -1 } else { 1 };
    let new_pos = skip_atomic_ranges(&atoms, pos.from(), bias);
    if new_pos == pos.from() {
        pos
    } else {
        EditorSelection::cursor_with(
            new_pos,
            if new_pos < pos.from() { 1 } else { -1 },
            None,
            None,
        )
    }
}
