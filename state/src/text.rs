//! Port of `text.ts`: the document as a balanced tree of lines.

use std::fmt;
use std::sync::Arc;

use crate::RangeError;

/// The branch factor as an exponent of 2.
const BRANCH_SHIFT: u32 = 5;
/// The approximate branch factor of the tree, in leaf and branch nodes.
const BRANCH: usize = 1 << BRANCH_SHIFT;

/// Flags passed to `decompose`.
const OPEN_FROM: u8 = 1;
const OPEN_TO: u8 = 2;

/// The data structure for documents: an immutable, cheaply cloned tree of
/// lines. Edits return a new `Text` that shares unchanged subtrees.
#[derive(Clone)]
pub struct Text(Arc<Node>);

enum Node {
    /// Leaves store line strings. There are always line breaks between
    /// them. Leaves are limited in size and are held in branches for
    /// bigger documents.
    Leaf { text: Vec<String>, length: usize },
    /// Branches provide the tree structure, balancing themselves on
    /// changes. There are implied line breaks _between_ the children (not
    /// before the first or after the last).
    Branch {
        children: Vec<Text>,
        length: usize,
        lines: usize,
    },
}

impl Text {
    fn leaf(text: Vec<String>, length: usize) -> Text {
        Text(Arc::new(Node::Leaf { text, length }))
    }

    fn leaf_of(text: Vec<String>) -> Text {
        let length = text_length(&text);
        Text::leaf(text, length)
    }

    fn branch(children: Vec<Text>, length: usize) -> Text {
        let lines = children.iter().map(Text::lines).sum();
        Text(Arc::new(Node::Branch {
            children,
            length,
            lines,
        }))
    }

    /// The empty document.
    pub fn empty() -> Text {
        Text::leaf(vec![String::new()], 0)
    }

    /// Create a document from its lines.
    pub fn of<S: AsRef<str>>(text: &[S]) -> Result<Text, RangeError> {
        if text.is_empty() {
            return Err(RangeError(
                "A document must have at least one line".to_string(),
            ));
        }
        let lines: Vec<String> = text.iter().map(|s| s.as_ref().to_string()).collect();
        Ok(Text::of_lines(lines))
    }

    fn of_lines(lines: Vec<String>) -> Text {
        if lines.len() == 1 && lines[0].is_empty() {
            Text::empty()
        } else if lines.len() <= BRANCH {
            Text::leaf_of(lines)
        } else {
            let mut parts = Vec::new();
            split_leaves(&lines, &mut parts);
            from_parts(parts)
        }
    }

    /// Create a document from a string, splitting lines at `\r\n`, `\r`
    /// and `\n` (CodeMirror's default line separator), or at `line_sep`.
    pub fn from_str_with(text: &str, line_sep: Option<&str>) -> Text {
        Text::of_lines(split_lines(text, line_sep))
    }

    /// The length of the document, in bytes, with one byte per line break.
    pub fn len(&self) -> usize {
        match &*self.0 {
            Node::Leaf { length, .. } | Node::Branch { length, .. } => *length,
        }
    }

    /// True when the document is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The number of lines in the document (always >= 1).
    pub fn lines(&self) -> usize {
        match &*self.0 {
            Node::Leaf { text, .. } => text.len(),
            Node::Branch { lines, .. } => *lines,
        }
    }

    /// For a branch node, the documents it is made of. `None` for a leaf.
    pub fn children(&self) -> Option<&[Text]> {
        match &*self.0 {
            Node::Leaf { .. } => None,
            Node::Branch { children, .. } => Some(children),
        }
    }

    fn child_count(&self) -> usize {
        match &*self.0 {
            Node::Leaf { text, .. } => text.len(),
            Node::Branch { children, .. } => children.len(),
        }
    }

    /// Get the line around the given position.
    ///
    /// # Panics
    ///
    /// When `pos` is past the end of the document.
    pub fn line_at(&self, pos: usize) -> Line<'_> {
        if pos > self.len() {
            panic!(
                "Invalid position {pos} in document of length {}",
                self.len()
            );
        }
        self.line_inner(pos, false, 1, 0)
    }

    /// Get the given (1-based) line.
    ///
    /// # Panics
    ///
    /// When `n` is 0 or greater than the number of lines.
    pub fn line(&self, n: usize) -> Line<'_> {
        if n < 1 || n > self.lines() {
            panic!("Invalid line number {n} in {}-line document", self.lines());
        }
        self.line_inner(n, true, 1, 0)
    }

    fn line_inner(
        &self,
        target: usize,
        is_line: bool,
        mut line: usize,
        mut offset: usize,
    ) -> Line<'_> {
        match &*self.0 {
            Node::Leaf { text, .. } => {
                for string in text {
                    let end = offset + string.len();
                    if (if is_line { line } else { end }) >= target {
                        return Line {
                            from: offset,
                            to: end,
                            number: line,
                            text: string,
                        };
                    }
                    offset = end + 1;
                    line += 1;
                }
            }
            Node::Branch { children, .. } => {
                for child in children {
                    let end = offset + child.len();
                    let end_line = line + child.lines() - 1;
                    if (if is_line { end_line } else { end }) >= target {
                        return child.line_inner(target, is_line, line, offset);
                    }
                    offset = end + 1;
                    line = end_line + 1;
                }
            }
        }
        unreachable!("line target {target} is inside the document")
    }

    /// Replace a range of the document with the given content. The range
    /// is clipped to the document.
    pub fn replace(&self, from: usize, to: usize, text: &Text) -> Text {
        match (&*self.0, &*text.0) {
            (
                Node::Leaf {
                    text: lines,
                    length,
                },
                Node::Leaf {
                    text: insert,
                    length: ins_len,
                },
            ) => {
                let (from, to) = clip(*length, from as isize, to as isize);
                let joined = append_text(
                    lines,
                    append_text(insert, slice_text(lines, 0, from as isize), 0, isize::MAX),
                    to as isize,
                    isize::MAX,
                );
                let new_len = length + ins_len - (to - from);
                if joined.len() <= BRANCH {
                    return Text::leaf(joined, new_len);
                }
                let mut parts = Vec::new();
                split_leaves(&joined, &mut parts);
                from_parts_len(parts, new_len)
            }
            (
                Node::Branch {
                    children, lines, ..
                },
                _,
            ) => {
                let (from, to) = clip(self.len(), from as isize, to as isize);
                if text.lines() < *lines {
                    let mut pos = 0;
                    for (i, child) in children.iter().enumerate() {
                        let end = pos + child.len();
                        // Fast path: if the change only affects one child and
                        // the child's size remains in the acceptable range,
                        // only update that child.
                        if from >= pos && to <= end {
                            let updated = child.replace(from - pos, to - pos, text);
                            let total_lines = lines - child.lines() + updated.lines();
                            if updated.lines() < (total_lines >> (BRANCH_SHIFT - 1))
                                && updated.lines() > (total_lines >> (BRANCH_SHIFT + 1))
                            {
                                let mut copy = children.clone();
                                copy[i] = updated;
                                return Text::branch(copy, self.len() - (to - from) + text.len());
                            }
                            return self.replace_parts(pos, end, &updated);
                        }
                        pos = end + 1;
                    }
                }
                self.replace_parts(from, to, text)
            }
            _ => self.replace_parts(from, to, text),
        }
    }

    /// The generic replace (`Text.replace` in the base class).
    fn replace_parts(&self, from: usize, to: usize, text: &Text) -> Text {
        let (from, to) = clip(self.len(), from as isize, to as isize);
        let mut parts = Vec::new();
        self.decompose(0, from as isize, &mut parts, OPEN_TO);
        if !text.is_empty() {
            text.decompose(0, text.len() as isize, &mut parts, OPEN_FROM | OPEN_TO);
        }
        self.decompose(to as isize, self.len() as isize, &mut parts, OPEN_FROM);
        from_parts_len(parts, self.len() - (to - from) + text.len())
    }

    /// Append another document to this one.
    pub fn append(&self, other: &Text) -> Text {
        self.replace(self.len(), self.len(), other)
    }

    /// The document between the given positions, clipped to the document.
    pub fn slice(&self, from: usize, to: usize) -> Text {
        let (from, to) = clip(self.len(), from as isize, to as isize);
        let mut parts = Vec::new();
        self.decompose(from as isize, to as isize, &mut parts, 0);
        from_parts_len(parts, to - from)
    }

    /// A part of the document as a string, with `\n` between lines.
    pub fn slice_string(&self, from: usize, to: usize) -> String {
        self.slice_string_with(from, to, "\n")
    }

    /// A part of the document as a string, with `line_sep` between lines.
    pub fn slice_string_with(&self, from: usize, to: usize, line_sep: &str) -> String {
        let mut result = String::new();
        self.slice_string_into(from as isize, to as isize, line_sep, &mut result);
        result
    }

    fn slice_string_into(&self, from: isize, to: isize, line_sep: &str, result: &mut String) {
        let (from, to) = clip(self.len(), from, to);
        match &*self.0 {
            Node::Leaf { text, .. } => {
                let mut pos = 0;
                for (i, line) in text.iter().enumerate() {
                    if pos > to {
                        break;
                    }
                    let end = pos + line.len();
                    if pos > from && i > 0 {
                        result.push_str(line_sep);
                    }
                    if from < end && to > pos {
                        result.push_str(&line[from.saturating_sub(pos)..to.min(end) - pos]);
                    }
                    pos = end + 1;
                }
            }
            Node::Branch { children, .. } => {
                let mut pos = 0;
                for (i, child) in children.iter().enumerate() {
                    if pos > to {
                        break;
                    }
                    let end = pos + child.len();
                    if pos > from && i > 0 {
                        result.push_str(line_sep);
                    }
                    if from < end && to > pos {
                        child.slice_string_into(
                            from as isize - pos as isize,
                            to as isize - pos as isize,
                            line_sep,
                            result,
                        );
                    }
                    pos = end + 1;
                }
            }
        }
    }

    fn flatten(&self, target: &mut Vec<String>) {
        match &*self.0 {
            Node::Leaf { text, .. } => target.extend(text.iter().cloned()),
            Node::Branch { children, .. } => {
                for child in children {
                    child.flatten(target);
                }
            }
        }
    }

    fn scan_identical(&self, other: &Text, dir: i8) -> usize {
        let (Node::Branch { children: a, .. }, Node::Branch { children: b, .. }) =
            (&*self.0, &*other.0)
        else {
            return 0;
        };
        let mut length = 0;
        let pairs: Box<dyn Iterator<Item = (&Text, &Text)>> = if dir > 0 {
            Box::new(a.iter().zip(b.iter()))
        } else {
            Box::new(a.iter().rev().zip(b.iter().rev()))
        };
        for (ch_a, ch_b) in pairs {
            if !Arc::ptr_eq(&ch_a.0, &ch_b.0) {
                return length + ch_a.scan_identical(ch_b, dir);
            }
            length += ch_a.len() + 1;
        }
        length
    }

    fn decompose(&self, from: isize, to: isize, target: &mut Vec<Text>, open: u8) {
        match &*self.0 {
            Node::Leaf {
                text: lines,
                length,
            } => {
                let text = if from <= 0 && to >= *length as isize {
                    self.clone()
                } else {
                    Text::leaf(
                        slice_text(lines, from, to),
                        (to.min(*length as isize) - from.max(0)) as usize,
                    )
                };
                if open & OPEN_FROM != 0 {
                    let prev = target.pop().expect("an open start follows a leaf");
                    let Node::Leaf {
                        text: prev_lines,
                        length: prev_len,
                    } = &*prev.0
                    else {
                        unreachable!("an open start follows a leaf")
                    };
                    let Node::Leaf {
                        text: lines,
                        length,
                    } = &*text.0
                    else {
                        unreachable!()
                    };
                    let joined = append_text(lines, prev_lines.clone(), 0, *length as isize);
                    if joined.len() <= BRANCH {
                        target.push(Text::leaf(joined, prev_len + length));
                    } else {
                        let mid = joined.len() >> 1;
                        target.push(Text::leaf_of(joined[..mid].to_vec()));
                        target.push(Text::leaf_of(joined[mid..].to_vec()));
                    }
                } else {
                    target.push(text);
                }
            }
            Node::Branch { children, .. } => {
                let mut pos: isize = 0;
                for child in children {
                    if pos > to {
                        break;
                    }
                    let end = pos + child.len() as isize;
                    if from <= end && to >= pos {
                        let child_open = open
                            & ((if pos <= from { OPEN_FROM } else { 0 })
                                | (if end >= to { OPEN_TO } else { 0 }));
                        if pos >= from && end <= to && child_open == 0 {
                            target.push(child.clone());
                        } else {
                            child.decompose(from - pos, to - pos, target, child_open);
                        }
                    }
                    pos = end + 1;
                }
            }
        }
    }

    /// Iterate over the document, returning lines and the breaks between
    /// them as separate strings. With `dir` -1, iterate from end to start.
    pub fn iter(&self, dir: i8) -> TextCursor<'_> {
        TextCursor::new(self, dir)
    }

    /// Iterate over a range of the document. When `from` > `to`, the
    /// iterator runs in reverse.
    pub fn iter_range(&self, from: usize, to: usize) -> PartialTextCursor<'_> {
        PartialTextCursor::new(self, from, to)
    }

    /// Iterate over the given (1-based) range of lines, without the line
    /// breaks between them and yielding empty strings for empty lines.
    /// `from` and `to` default to the whole document; `to` is exclusive.
    pub fn iter_lines(&self, from: Option<usize>, to: Option<usize>) -> LineCursor<'_> {
        let inner = match from {
            None => LineInner::Raw(self.iter(1)),
            Some(from) => {
                let to = to.unwrap_or(self.lines() + 1);
                let start = self.line(from).from;
                let end = if to == self.lines() + 1 {
                    self.len()
                } else if to <= 1 {
                    0
                } else {
                    self.line(to - 1).to
                };
                LineInner::Partial(self.iter_range(start, start.max(end)))
            }
        };
        LineCursor {
            inner,
            after_break: true,
            value: "",
            done: false,
        }
    }

    /// The document as an array of lines, which [`Text::of`] reads back.
    pub fn to_json(&self) -> Vec<String> {
        let mut lines = Vec::new();
        self.flatten(&mut lines);
        lines
    }
}

impl Default for Text {
    fn default() -> Text {
        Text::empty()
    }
}

impl From<&str> for Text {
    fn from(text: &str) -> Text {
        Text::from_str_with(text, None)
    }
}

impl PartialEq for Text {
    fn eq(&self, other: &Text) -> bool {
        if Arc::ptr_eq(&self.0, &other.0) {
            return true;
        }
        if other.len() != self.len() || other.lines() != self.lines() {
            return false;
        }
        let start = self.scan_identical(other, 1);
        let end = self.len() - self.scan_identical(other, -1);
        let mut a = TextCursor::new(self, 1);
        let mut b = TextCursor::new(other, 1);
        let mut skip = start as isize;
        let mut pos = start;
        loop {
            a.advance(skip);
            b.advance(skip);
            skip = 0;
            if a.line_break != b.line_break || a.done != b.done || a.value != b.value {
                return false;
            }
            pos += a.value.len();
            if a.done || pos >= end {
                return true;
            }
        }
    }
}

impl Eq for Text {}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.slice_string(0, self.len()))
    }
}

impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Text({:?})", self.to_string())
    }
}

impl<'a> IntoIterator for &'a Text {
    type Item = &'a str;
    type IntoIter = TextCursor<'a>;

    fn into_iter(self) -> TextCursor<'a> {
        self.iter(1)
    }
}

/// `TextNode.from`, with the length computed from the parts.
fn from_parts(children: Vec<Text>) -> Text {
    let length = children.iter().map(|ch| ch.len() + 1).sum::<usize>() - 1;
    from_parts_len(children, length)
}

/// `TextNode.from`: build a balanced tree from a list of parts.
fn from_parts_len(children: Vec<Text>, length: usize) -> Text {
    let lines: usize = children.iter().map(Text::lines).sum();
    if lines < BRANCH {
        let mut flat = Vec::new();
        for ch in &children {
            ch.flatten(&mut flat);
        }
        return Text::leaf(flat, length);
    }
    let chunk = BRANCH.max(lines >> BRANCH_SHIFT);
    let mut builder = Chunker {
        chunk,
        max_chunk: chunk << 1,
        min_chunk: chunk >> 1,
        chunked: Vec::new(),
        current_lines: 0,
        current_len: -1,
        current_chunk: Vec::new(),
    };
    for child in children {
        builder.add(child);
    }
    builder.flush();
    let mut chunked = builder.chunked;
    if chunked.len() == 1 {
        chunked.pop().expect("one chunk")
    } else {
        Text::branch(chunked, length)
    }
}

struct Chunker {
    chunk: usize,
    max_chunk: usize,
    min_chunk: usize,
    chunked: Vec<Text>,
    current_lines: usize,
    current_len: isize,
    current_chunk: Vec<Text>,
}

impl Chunker {
    fn add(&mut self, child: Text) {
        if child.lines() > self.max_chunk && child.children().is_some() {
            for node in child.children().expect("a branch") {
                self.add(node.clone());
            }
        } else if child.lines() > self.min_chunk
            && (self.current_lines > self.min_chunk || self.current_lines == 0)
        {
            self.flush();
            self.chunked.push(child);
        } else if let Some(merged) = self
            .current_chunk
            .last()
            .filter(|_| self.current_lines > 0)
            .and_then(|last| merge_leaves(last, &child))
        {
            self.current_lines += child.lines();
            self.current_len += child.len() as isize + 1;
            *self.current_chunk.last_mut().expect("a last part") = merged;
        } else {
            if self.current_lines + child.lines() > self.chunk {
                self.flush();
            }
            self.current_lines += child.lines();
            self.current_len += child.len() as isize + 1;
            self.current_chunk.push(child);
        }
    }

    fn flush(&mut self) {
        if self.current_lines == 0 {
            return;
        }
        let parts = std::mem::take(&mut self.current_chunk);
        let node = if parts.len() == 1 {
            parts.into_iter().next().expect("one part")
        } else {
            from_parts_len(parts, self.current_len as usize)
        };
        self.chunked.push(node);
        self.current_len = -1;
        self.current_lines = 0;
    }
}

/// `TextNode.from`'s leaf merge: two leaves that fit in one become one.
fn merge_leaves(last: &Text, child: &Text) -> Option<Text> {
    match (&*last.0, &*child.0) {
        (
            Node::Leaf {
                text: last_text,
                length: last_len,
            },
            Node::Leaf { text, length },
        ) if text.len() + last_text.len() <= BRANCH => Some(Text::leaf(
            last_text.iter().chain(text.iter()).cloned().collect(),
            last_len + 1 + length,
        )),
        _ => None,
    }
}

fn text_length(text: &[String]) -> usize {
    text.iter().map(|line| line.len() + 1).sum::<usize>() - 1
}

/// `TextLeaf.split`: cut lines into leaves of at most `BRANCH` lines.
fn split_leaves(text: &[String], target: &mut Vec<Text>) {
    for part in text.chunks(BRANCH) {
        target.push(Text::leaf_of(part.to_vec()));
    }
}

/// Append the lines of `text` between `from` and `to` to `target`, joining
/// the first appended piece to `target`'s last line.
fn append_text(text: &[String], mut target: Vec<String>, from: isize, to: isize) -> Vec<String> {
    let mut pos: isize = 0;
    let mut first = true;
    for line in text {
        if pos > to {
            break;
        }
        let end = pos + line.len() as isize;
        if end >= from {
            let mut piece: &str = line;
            if end > to {
                piece = &piece[..(to - pos) as usize];
            }
            if pos < from {
                piece = &piece[(from - pos) as usize..];
            }
            if first {
                target.last_mut().expect("a target line").push_str(piece);
                first = false;
            } else {
                target.push(piece.to_string());
            }
        }
        pos = end + 1;
    }
    target
}

fn slice_text(text: &[String], from: isize, to: isize) -> Vec<String> {
    append_text(text, vec![String::new()], from, to)
}

fn clip(length: usize, from: isize, to: isize) -> (usize, usize) {
    let from = from.clamp(0, length as isize);
    (from as usize, to.min(length as isize).max(from) as usize)
}

/// Split a string into lines at `line_sep`, or at `\r\n`, `\r` and `\n`.
fn split_lines(text: &str, line_sep: Option<&str>) -> Vec<String> {
    if let Some(sep) = line_sep.filter(|sep| !sep.is_empty()) {
        return text.split(sep).map(str::to_string).collect();
    }
    let mut lines = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(['\r', '\n']) {
        lines.push(rest[..at].to_string());
        let skip = if rest[at..].starts_with("\r\n") { 2 } else { 1 };
        rest = &rest[at + skip..];
    }
    lines.push(rest.to_string());
    lines
}

/// A line in the document, created on demand by [`Text::line_at`] and
/// [`Text::line`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line<'a> {
    /// The position of the start of the line.
    pub from: usize,
    /// The position at the end of the line, _before_ the line break, or at
    /// the end of the document for the last line.
    pub to: usize,
    /// The line's 1-based number.
    pub number: usize,
    /// The line's content.
    pub text: &'a str,
}

impl Line<'_> {
    /// The length of the line, without any line break after it.
    pub fn len(&self) -> usize {
        self.to - self.from
    }

    /// True for an empty line.
    pub fn is_empty(&self) -> bool {
        self.from == self.to
    }
}

/// CodeMirror's `TextIterator`: a cursor over strings that can skip
/// positions. Over a [`Text`], the values are lines or line breaks.
pub trait TextIterator<'a> {
    /// Move to the next string, first skipping `skip` positions (backwards
    /// when negative). `TextIterator.next(skip)` upstream.
    fn advance(&mut self, skip: isize) -> &mut Self;
    /// The current string; empty at the end or before the first `advance`.
    fn value(&self) -> &'a str;
    /// Whether the end has been reached.
    fn done(&self) -> bool;
    /// Whether the current string is a line break.
    fn line_break(&self) -> bool;
}

/// Iterates over a whole document (`RawTextCursor` upstream).
pub struct TextCursor<'a> {
    dir: i8,
    nodes: Vec<&'a Text>,
    /// The offset into the node at each level, shifted one to the left,
    /// with the low bit telling whether the position is before (0) or
    /// after (1) the line break between adjacent nodes.
    offsets: Vec<isize>,
    done: bool,
    line_break: bool,
    value: &'a str,
}

impl<'a> TextCursor<'a> {
    fn new(text: &'a Text, dir: i8) -> TextCursor<'a> {
        let dir = if dir < 0 { -1 } else { 1 };
        TextCursor {
            dir,
            nodes: vec![text],
            offsets: vec![if dir > 0 {
                1
            } else {
                (text.child_count() << 1) as isize
            }],
            done: false,
            line_break: false,
            value: "",
        }
    }

    fn next_inner(&mut self, mut skip: usize, dir: i8) {
        self.done = false;
        self.line_break = false;
        loop {
            let last = self.nodes.len() - 1;
            let top = self.nodes[last];
            let offset_value = self.offsets[last];
            let offset = (offset_value >> 1) as usize;
            let size = top.child_count();
            if offset == (if dir > 0 { size } else { 0 }) {
                if last == 0 {
                    self.done = true;
                    self.value = "";
                    return;
                }
                if dir > 0 {
                    self.offsets[last - 1] += 1;
                }
                self.nodes.pop();
                self.offsets.pop();
            } else if (offset_value & 1) == (if dir > 0 { 0 } else { 1 }) {
                self.offsets[last] += dir as isize;
                if skip == 0 {
                    self.line_break = true;
                    self.value = "\n";
                    return;
                }
                skip -= 1;
            } else {
                let index = if dir < 0 { offset - 1 } else { offset };
                match &*top.0 {
                    Node::Leaf { text, .. } => {
                        // Move to the next string.
                        let next: &'a str = &text[index];
                        self.offsets[last] += dir as isize;
                        if next.len() > skip {
                            self.value = if skip == 0 {
                                next
                            } else if dir > 0 {
                                &next[skip..]
                            } else {
                                &next[..next.len() - skip]
                            };
                            return;
                        }
                        skip -= next.len();
                    }
                    Node::Branch { children, .. } => {
                        let next: &'a Text = &children[index];
                        if skip > next.len() {
                            skip -= next.len();
                            self.offsets[last] += dir as isize;
                        } else {
                            if dir < 0 {
                                self.offsets[last] -= 1;
                            }
                            self.nodes.push(next);
                            self.offsets.push(if dir > 0 {
                                1
                            } else {
                                (next.child_count() << 1) as isize
                            });
                        }
                    }
                }
            }
        }
    }
}

impl<'a> TextIterator<'a> for TextCursor<'a> {
    fn advance(&mut self, skip: isize) -> &mut Self {
        let mut skip = skip;
        if skip < 0 {
            self.next_inner(skip.unsigned_abs(), -self.dir);
            skip = self.value.len() as isize;
        }
        self.next_inner(skip as usize, self.dir);
        self
    }

    fn value(&self) -> &'a str {
        self.value
    }

    fn done(&self) -> bool {
        self.done
    }

    fn line_break(&self) -> bool {
        self.line_break
    }
}

impl<'a> Iterator for TextCursor<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        self.advance(0);
        (!self.done).then_some(self.value)
    }
}

/// Iterates over a range of a document (`PartialTextCursor` upstream).
pub struct PartialTextCursor<'a> {
    cursor: TextCursor<'a>,
    value: &'a str,
    pos: isize,
    from: isize,
    to: isize,
    done: bool,
}

impl<'a> PartialTextCursor<'a> {
    fn new(text: &'a Text, start: usize, end: usize) -> PartialTextCursor<'a> {
        PartialTextCursor {
            cursor: TextCursor::new(text, if start > end { -1 } else { 1 }),
            value: "",
            pos: if start > end { text.len() as isize } else { 0 },
            from: start.min(end) as isize,
            to: start.max(end) as isize,
            done: false,
        }
    }

    fn next_inner(&mut self, mut skip: isize, dir: i8) {
        if if dir < 0 {
            self.pos <= self.from
        } else {
            self.pos >= self.to
        } {
            self.value = "";
            self.done = true;
            return;
        }
        skip += 0.max(if dir < 0 {
            self.pos - self.to
        } else {
            self.from - self.pos
        });
        let mut limit = if dir < 0 {
            self.pos - self.from
        } else {
            self.to - self.pos
        };
        if skip > limit {
            skip = limit;
        }
        limit -= skip;
        let value = self.cursor.advance(skip).value;
        self.pos += (value.len() as isize + skip) * dir as isize;
        let limit = limit.max(0) as usize;
        self.value = if value.len() <= limit {
            value
        } else if dir < 0 {
            &value[value.len() - limit..]
        } else {
            &value[..limit]
        };
        self.done = self.value.is_empty();
    }
}

impl<'a> TextIterator<'a> for PartialTextCursor<'a> {
    fn advance(&mut self, skip: isize) -> &mut Self {
        let mut skip = skip;
        if skip < 0 {
            skip = skip.max(self.from - self.pos);
        } else if skip > 0 {
            skip = skip.min(self.to - self.pos);
        }
        self.next_inner(skip, self.cursor.dir);
        self
    }

    fn value(&self) -> &'a str {
        self.value
    }

    fn done(&self) -> bool {
        self.done
    }

    fn line_break(&self) -> bool {
        self.cursor.line_break && !self.value.is_empty()
    }
}

impl<'a> Iterator for PartialTextCursor<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        self.advance(0);
        (!self.done).then_some(self.value)
    }
}

enum LineInner<'a> {
    Raw(TextCursor<'a>),
    Partial(PartialTextCursor<'a>),
}

impl<'a> LineInner<'a> {
    fn advance(&mut self, skip: isize) -> (bool, bool, &'a str) {
        match self {
            LineInner::Raw(c) => {
                c.advance(skip);
                (c.done(), c.line_break(), c.value())
            }
            LineInner::Partial(c) => {
                c.advance(skip);
                (c.done(), c.line_break(), c.value())
            }
        }
    }
}

/// Iterates over lines without the breaks between them, yielding empty
/// strings for empty lines (`LineCursor` upstream).
pub struct LineCursor<'a> {
    inner: LineInner<'a>,
    after_break: bool,
    value: &'a str,
    done: bool,
}

impl<'a> TextIterator<'a> for LineCursor<'a> {
    fn advance(&mut self, skip: isize) -> &mut Self {
        let (done, line_break, value) = self.inner.advance(skip);
        if done && self.after_break {
            self.value = "";
            self.after_break = false;
        } else if done {
            self.done = true;
            self.value = "";
        } else if line_break {
            if self.after_break {
                self.value = "";
            } else {
                self.after_break = true;
                self.advance(0);
            }
        } else {
            self.value = value;
            self.after_break = false;
        }
        self
    }

    fn value(&self) -> &'a str {
        self.value
    }

    fn done(&self) -> bool {
        self.done
    }

    fn line_break(&self) -> bool {
        false
    }
}

impl<'a> Iterator for LineCursor<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        self.advance(0);
        (!self.done).then_some(self.value)
    }
}
