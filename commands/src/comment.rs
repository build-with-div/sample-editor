//! Port of `comment.ts`: toggling line and block comments, with the
//! comment syntax read from the `"commentTokens"` language data.
//!
//! Positions are UTF-8 bytes. Upstream measures its search margin and
//! the lines' indentation in UTF-16 units; the port measures them the same
//! way and converts, so a line indented with a wide space character is
//! commented where upstream's is.

use editor_state::{ChangeSpec, EditorState, TransactionSpec};

use crate::commands::{is_js_space, Dispatch};

/// The comment syntax of a language (`CommentTokens`), provided as
/// [language data](EditorState::language_data_at) under
/// `"commentTokens"`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommentTokens {
    /// The block comment syntax, if any: `<!--` and `-->` for HTML.
    pub block: Option<BlockTokens>,
    /// The line comment syntax, if any: `//`.
    pub line: Option<String>,
}

/// The tokens that open and close a block comment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockTokens {
    /// The opening token, such as `/*`.
    pub open: String,
    /// The closing token, such as `*/`.
    pub close: String,
}

/// Comment or uncomment the selection (`toggleComment`): with line
/// comments when the language has them, otherwise with block comments
/// around the selected lines.
pub fn toggle_comment(state: &EditorState, dispatch: Dispatch) -> bool {
    let line = state.doc().line_at(state.selection().main().from());
    let config = get_config(state, line.from);
    if config.line.is_some() {
        toggle_line_comment(state, dispatch)
    } else if config.block.is_some() {
        toggle_block_comment_by_line(state, dispatch)
    } else {
        false
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CommentOption {
    Toggle,
    Comment,
    Uncomment,
}

// `command`: run `f` unless the state is read-only, and dispatch the
// spec it returns.
fn run(
    state: &EditorState,
    dispatch: Dispatch,
    f: impl FnOnce() -> Option<TransactionSpec>,
) -> bool {
    if state.read_only() {
        return false;
    }
    let Some(spec) = f() else {
        return false;
    };
    dispatch(
        state
            .update(spec)
            .expect("comment changes inside the document"),
    );
    true
}

fn selection_ranges(state: &EditorState) -> Vec<(usize, usize)> {
    state
        .selection()
        .ranges()
        .iter()
        .map(|r| (r.from(), r.to()))
        .collect()
}

/// Comment or uncomment the selection with line comments
/// (`toggleLineComment`), from the `"commentTokens"` language data.
pub fn toggle_line_comment(state: &EditorState, dispatch: Dispatch) -> bool {
    run(state, dispatch, || {
        change_line_comment(CommentOption::Toggle, state, &selection_ranges(state))
    })
}

/// Comment the selection with line comments (`lineComment`).
pub fn line_comment(state: &EditorState, dispatch: Dispatch) -> bool {
    run(state, dispatch, || {
        change_line_comment(CommentOption::Comment, state, &selection_ranges(state))
    })
}

/// Uncomment the selection's line comments (`lineUncomment`).
pub fn line_uncomment(state: &EditorState, dispatch: Dispatch) -> bool {
    run(state, dispatch, || {
        change_line_comment(CommentOption::Uncomment, state, &selection_ranges(state))
    })
}

/// Comment or uncomment the selection with block comments
/// (`toggleBlockComment`), from the `"commentTokens"` language data.
pub fn toggle_block_comment(state: &EditorState, dispatch: Dispatch) -> bool {
    run(state, dispatch, || {
        change_block_comment(CommentOption::Toggle, state, &selection_ranges(state))
    })
}

/// Comment the selection with block comments (`blockComment`).
pub fn block_comment(state: &EditorState, dispatch: Dispatch) -> bool {
    run(state, dispatch, || {
        change_block_comment(CommentOption::Comment, state, &selection_ranges(state))
    })
}

/// Uncomment the block comments around the selection (`blockUncomment`).
pub fn block_uncomment(state: &EditorState, dispatch: Dispatch) -> bool {
    run(state, dispatch, || {
        change_block_comment(CommentOption::Uncomment, state, &selection_ranges(state))
    })
}

/// Comment or uncomment the lines around the selection with block
/// comments (`toggleBlockCommentByLine`).
pub fn toggle_block_comment_by_line(state: &EditorState, dispatch: Dispatch) -> bool {
    run(state, dispatch, || {
        change_block_comment(CommentOption::Toggle, state, &selected_line_ranges(state))
    })
}

// `getConfig`.
fn get_config(state: &EditorState, pos: usize) -> CommentTokens {
    state
        .language_data_at::<CommentTokens>("commentTokens", pos, 1)
        .into_iter()
        .next()
        .unwrap_or_default()
}

// How far `findBlockComment` looks around a range, in UTF-16 units.
const SEARCH_MARGIN: usize = 50;

// The position `units` UTF-16 units before `pos`, or the document's
// start. A line break counts as one unit, as a position does; a character
// that would pass the margin is left out.
fn units_before(state: &EditorState, pos: usize, units: usize) -> usize {
    let doc = state.doc();
    let (mut pos, mut left) = (pos, units);
    while left > 0 && pos > 0 {
        let line = doc.line_at(pos);
        if pos == line.from {
            pos -= 1;
            left -= 1;
            continue;
        }
        let ch = line.text[..pos - line.from]
            .chars()
            .next_back()
            .expect("text before pos");
        if ch.len_utf16() > left {
            break;
        }
        left -= ch.len_utf16();
        pos -= ch.len_utf8();
    }
    pos
}

// The position `units` UTF-16 units after `pos`, or the document's end.
fn units_after(state: &EditorState, pos: usize, units: usize) -> usize {
    let doc = state.doc();
    let (mut pos, mut left) = (pos, units);
    while left > 0 && pos < doc.len() {
        let line = doc.line_at(pos);
        if pos == line.to {
            pos += 1;
            left -= 1;
            continue;
        }
        let ch = line.text[pos - line.from..]
            .chars()
            .next()
            .expect("text after pos");
        if ch.len_utf16() > left {
            break;
        }
        left -= ch.len_utf16();
        pos += ch.len_utf8();
    }
    pos
}

fn leading_space(text: &str) -> usize {
    text.len() - text.trim_start_matches(is_js_space).len()
}

fn trailing_space(text: &str) -> usize {
    text.len() - text.trim_end_matches(is_js_space).len()
}

// Where a block comment's token sits: its position and whether one
// space beside it belongs to the comment.
struct TokenPos {
    pos: usize,
    margin: usize,
}

struct BlockComment {
    open: TokenPos,
    close: TokenPos,
}

// `findBlockComment`: the block comment around or just inside
// `from`..`to`, if there is one.
fn find_block_comment(
    state: &EditorState,
    tokens: &BlockTokens,
    from: usize,
    to: usize,
) -> Option<BlockComment> {
    let (open, close) = (tokens.open.as_str(), tokens.close.as_str());
    let text_before = state.slice_doc(units_before(state, from, SEARCH_MARGIN), from);
    let text_after = state.slice_doc(to, units_after(state, to, SEARCH_MARGIN));
    let space_before = trailing_space(&text_before);
    let space_after = leading_space(&text_after);
    let before_off = text_before.len() - space_before;
    if text_before[..before_off].ends_with(open) && text_after[space_after..].starts_with(close) {
        return Some(BlockComment {
            open: TokenPos {
                pos: from - space_before,
                margin: usize::from(space_before > 0),
            },
            close: TokenPos {
                pos: to + space_after,
                margin: usize::from(space_after > 0),
            },
        });
    }

    let (start_text, end_text) = if units_after(state, from, 2 * SEARCH_MARGIN) >= to {
        let text = state.slice_doc(from, to);
        (text.clone(), text)
    } else {
        (
            state.slice_doc(from, units_after(state, from, SEARCH_MARGIN)),
            state.slice_doc(units_before(state, to, SEARCH_MARGIN), to),
        )
    };
    let start_space = leading_space(&start_text);
    let end_space = trailing_space(&end_text);
    let end_off = end_text.len().checked_sub(end_space + close.len())?;
    if start_text[start_space..].starts_with(open) && end_text[end_off..].starts_with(close) {
        let after_open = start_text[start_space + open.len()..].chars().next();
        let before_close = end_text[..end_off].chars().next_back();
        return Some(BlockComment {
            open: TokenPos {
                pos: from + start_space + open.len(),
                margin: usize::from(after_open.is_some_and(is_js_space)),
            },
            close: TokenPos {
                pos: to - end_space - close.len(),
                margin: usize::from(before_close.is_some_and(is_js_space)),
            },
        });
    }
    None
}

// `selectedLineRanges`: the selected lines, from their indentation to
// their end, with ranges on touching lines joined.
fn selected_line_ranges(state: &EditorState) -> Vec<(usize, usize)> {
    let doc = state.doc();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for r in state.selection().ranges() {
        let from_line = doc.line_at(r.from());
        let mut to_line = if r.to() <= from_line.to {
            from_line
        } else {
            doc.line_at(r.to())
        };
        if to_line.from > from_line.from && to_line.from == r.to() {
            to_line = if r.to() == from_line.to + 1 {
                from_line
            } else {
                doc.line_at(r.to() - 1)
            };
        }
        match ranges.last_mut() {
            Some(last) if last.1 > from_line.from => last.1 = to_line.to,
            _ => ranges.push((from_line.from + leading_space(from_line.text), to_line.to)),
        }
    }
    ranges
}

// `changeBlockComment`: toggle, add or remove block comments around
// `ranges`.
fn change_block_comment(
    option: CommentOption,
    state: &EditorState,
    ranges: &[(usize, usize)],
) -> Option<TransactionSpec> {
    let tokens: Vec<BlockTokens> = ranges
        .iter()
        .map(|&(from, _)| get_config(state, from).block)
        .collect::<Option<_>>()?;
    let comments: Vec<Option<BlockComment>> = ranges
        .iter()
        .zip(&tokens)
        .map(|(&(from, to), token)| find_block_comment(state, token, from, to))
        .collect();
    if option != CommentOption::Uncomment && !comments.iter().all(Option::is_some) {
        let mut changes = Vec::new();
        for ((&(from, to), token), comment) in ranges.iter().zip(&tokens).zip(&comments) {
            if comment.is_none() {
                changes.push(ChangeSpec::insert(from, format!("{} ", token.open)));
                changes.push(ChangeSpec::insert(to, format!(" {}", token.close)));
            }
        }
        let changes = state
            .changes(Some(ChangeSpec::List(changes)))
            .expect("comment tokens inside the document");
        Some(TransactionSpec {
            changes: Some(ChangeSpec::Set(changes)),
            ..TransactionSpec::default()
        })
    } else if option != CommentOption::Comment && comments.iter().any(Option::is_some) {
        let mut changes = Vec::new();
        for (comment, token) in comments.iter().zip(&tokens) {
            let Some(BlockComment { open, close }) = comment else {
                continue;
            };
            changes.push(ChangeSpec::delete(
                open.pos - token.open.len(),
                open.pos + open.margin,
            ));
            changes.push(ChangeSpec::delete(
                close.pos - close.margin,
                close.pos + token.close.len(),
            ));
        }
        Some(TransactionSpec {
            changes: Some(ChangeSpec::List(changes)),
            ..TransactionSpec::default()
        })
    } else {
        None
    }
}

struct CommentLine<'a> {
    from: usize,
    text: &'a str,
    token: String,
    // The byte offset of the line's comment token, if it starts with one.
    comment: Option<usize>,
    empty: bool,
    // The indentation the comment goes after, in characters (UTF-16 units:
    // whitespace is all in the Basic Multilingual Plane).
    indent: usize,
    single: bool,
}

// The byte offset of character `chars` in `text`.
fn char_offset(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(offset, _)| offset)
}

// `changeLineComment`: toggle, add or remove line comments on the lines
// `ranges` touch.
fn change_line_comment(
    option: CommentOption,
    state: &EditorState,
    ranges: &[(usize, usize)],
) -> Option<TransactionSpec> {
    let doc = state.doc();
    let mut lines: Vec<CommentLine> = Vec::new();
    let mut prev_line: Option<usize> = None;
    'ranges: for &(from, to) in ranges {
        let start_i = lines.len();
        let mut min_indent = usize::MAX;
        let mut token: Option<String> = None;
        let mut pos = from;
        while pos <= to {
            let line = doc.line_at(pos);
            let token = match &token {
                Some(token) => token.clone(),
                None => match get_config(state, line.from).line {
                    Some(found) => {
                        token = Some(found.clone());
                        found
                    }
                    None => continue 'ranges,
                },
            };
            if prev_line.is_none_or(|prev| line.from > prev) && (from == to || to > line.from) {
                prev_line = Some(line.from);
                let space = leading_space(line.text);
                let indent = line.text[..space].chars().count();
                let empty = space == line.text.len();
                let comment = line.text[space..].starts_with(&token).then_some(space);
                if space < line.text.len() && indent < min_indent {
                    min_indent = indent;
                }
                lines.push(CommentLine {
                    from: line.from,
                    text: line.text,
                    token,
                    comment,
                    empty,
                    indent,
                    single: false,
                });
            }
            pos = line.to + 1;
        }
        if min_indent < usize::MAX {
            for line in &mut lines[start_i..] {
                if char_offset(line.text, line.indent) < line.text.len() {
                    line.indent = min_indent;
                }
            }
        }
        if lines.len() == start_i + 1 {
            lines[start_i].single = true;
        }
    }

    if option != CommentOption::Uncomment
        && lines
            .iter()
            .any(|l| l.comment.is_none() && (!l.empty || l.single))
    {
        let changes = lines
            .iter()
            .filter(|l| l.single || !l.empty)
            .map(|l| {
                ChangeSpec::insert(
                    l.from + char_offset(l.text, l.indent),
                    format!("{} ", l.token),
                )
            })
            .collect();
        let changes = state
            .changes(Some(ChangeSpec::List(changes)))
            .expect("comment tokens inside the document");
        let selection = state.selection().map(changes.desc(), 1);
        Some(TransactionSpec {
            changes: Some(ChangeSpec::Set(changes)),
            selection: Some(selection),
            ..TransactionSpec::default()
        })
    } else if option != CommentOption::Comment && lines.iter().any(|l| l.comment.is_some()) {
        let mut changes = Vec::new();
        for line in &lines {
            let Some(comment) = line.comment else {
                continue;
            };
            let from = line.from + comment;
            let mut to = from + line.token.len();
            if line.text.as_bytes().get(to - line.from) == Some(&b' ') {
                to += 1;
            }
            changes.push(ChangeSpec::delete(from, to));
        }
        Some(TransactionSpec {
            changes: Some(ChangeSpec::List(changes)),
            ..TransactionSpec::default()
        })
    } else {
        None
    }
}
