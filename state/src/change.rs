//! Port of `change.ts`: change descriptions and change sets.

use std::fmt;

use crate::text::Text;
use crate::RangeError;

/// Distinguishes the ways a position can be mapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MapMode {
    /// Map a position to a valid new position, even when its context was
    /// deleted.
    #[default]
    Simple,
    /// Return `None` if deletion happens across the position.
    TrackDel,
    /// Return `None` if the character _before_ the position is deleted.
    TrackBefore,
    /// Return `None` if the character _after_ the position is deleted.
    TrackAfter,
}

/// How a change touches a range ([`ChangeDesc::touches_range`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Touch {
    /// No change touches the range.
    No,
    /// A change touches the range.
    Yes,
    /// One change entirely covers the range (`"cover"` upstream).
    Cover,
}

/// A change description: a [`ChangeSet`] without the inserted text. It
/// cannot be applied, but is cheaper to store and manipulate.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct ChangeDesc {
    // Sections are pairs of integers. The first is the length in the
    // current document, and the second is -1 for unaffected sections and
    // the length of the replacement content otherwise. So an insertion is
    // (0, n>0), a deletion (n>0, 0), and a replacement two positive numbers.
    sections: Vec<isize>,
}

impl ChangeDesc {
    fn create(sections: Vec<isize>) -> ChangeDesc {
        ChangeDesc { sections }
    }

    /// The length of the document before the change.
    pub fn len(&self) -> usize {
        self.sections.iter().step_by(2).sum::<isize>() as usize
    }

    /// The length of the document after the change.
    pub fn new_len(&self) -> usize {
        self.sections
            .chunks(2)
            .map(|s| if s[1] < 0 { s[0] } else { s[1] })
            .sum::<isize>() as usize
    }

    /// False when there are actual changes in this set.
    pub fn is_empty(&self) -> bool {
        self.sections.is_empty() || self.sections.len() == 2 && self.sections[1] < 0
    }

    /// Iterate over the unchanged parts left by these changes. `f` gets the
    /// position of the range in the old document, its position in the new
    /// document and its length.
    pub fn iter_gaps(&self, mut f: impl FnMut(usize, usize, usize)) {
        let (mut pos_a, mut pos_b) = (0, 0);
        for s in self.sections.chunks(2) {
            let (len, ins) = (s[0] as usize, s[1]);
            if ins < 0 {
                f(pos_a, pos_b, len);
                pos_b += len;
            } else {
                pos_b += ins as usize;
            }
            pos_a += len;
        }
    }

    /// Iterate over the ranges changed by these changes: `f` gets the
    /// extent in the old document (`from_a`, `to_a`) and of the replacement
    /// in the new document (`from_b`, `to_b`). When `individual` is true,
    /// adjacent changes are reported separately.
    pub fn iter_changed_ranges(
        &self,
        mut f: impl FnMut(usize, usize, usize, usize),
        individual: bool,
    ) {
        iter_changes(
            &self.sections,
            None,
            |a, b, c, d, _| f(a, b, c, d),
            individual,
        );
    }

    /// A description of the inverted form of these changes.
    pub fn inverted_desc(&self) -> ChangeDesc {
        let mut sections = Vec::with_capacity(self.sections.len());
        for s in self.sections.chunks(2) {
            let (len, ins) = (s[0], s[1]);
            if ins < 0 {
                sections.extend([len, ins]);
            } else {
                sections.extend([ins, len]);
            }
        }
        ChangeDesc::create(sections)
    }

    /// The combined effect of applying `other` after these changes. The
    /// length of the document after this set must match the length before
    /// `other`.
    ///
    /// # Panics
    ///
    /// When the lengths do not match.
    pub fn compose_desc(&self, other: &ChangeDesc) -> ChangeDesc {
        if self.is_empty() {
            other.clone()
        } else if other.is_empty() {
            self.clone()
        } else {
            let (sections, _) = compose_sets(&self.sections, None, &other.sections, None);
            ChangeDesc::create(sections)
        }
    }

    /// Map this description, which starts in the same document as `other`,
    /// over `other`, so that it applies after it. When `before` is true,
    /// map as if these changes happened before the ones in `other`.
    ///
    /// # Panics
    ///
    /// When the lengths do not match.
    pub fn map_desc(&self, other: &ChangeDesc, before: bool) -> ChangeDesc {
        if other.is_empty() {
            return self.clone();
        }
        let (sections, _) = map_set(&self.sections, None, &other.sections, before);
        ChangeDesc::create(sections)
    }

    /// Map a position through these changes into the new document.
    ///
    /// When `assoc` is negative, the position stays with the character
    /// before it: it moves before insertions at that point and replacements
    /// across it. When it is zero or positive, it stays with the character
    /// after it and moves forward. CodeMirror's default is -1.
    ///
    /// # Panics
    ///
    /// When `pos` is past the end of the old document.
    pub fn map_pos(&self, pos: usize, assoc: i32) -> usize {
        self.map_pos_mode(pos, assoc, MapMode::Simple)
            .expect("simple mapping always produces a position")
    }

    /// [`map_pos`](Self::map_pos), returning `None` when `mode` reports a
    /// deletion around the position.
    ///
    /// # Panics
    ///
    /// When `pos` is past the end of the old document.
    pub fn map_pos_mode(&self, pos: usize, assoc: i32, mode: MapMode) -> Option<usize> {
        let pos = pos as isize;
        let (mut pos_a, mut pos_b) = (0isize, 0isize);
        for s in self.sections.chunks(2) {
            let (len, ins) = (s[0], s[1]);
            let end_a = pos_a + len;
            if ins < 0 {
                if end_a > pos {
                    return Some((pos_b + (pos - pos_a)) as usize);
                }
                pos_b += len;
            } else {
                if mode != MapMode::Simple
                    && end_a >= pos
                    && (mode == MapMode::TrackDel && pos_a < pos && end_a > pos
                        || mode == MapMode::TrackBefore && pos_a < pos
                        || mode == MapMode::TrackAfter && end_a > pos)
                {
                    return None;
                }
                if end_a > pos || end_a == pos && assoc < 0 && len == 0 {
                    return Some(if pos == pos_a || assoc < 0 {
                        pos_b
                    } else {
                        pos_b + ins
                    } as usize);
                }
                pos_b += ins;
            }
            pos_a = end_a;
        }
        if pos > pos_a {
            panic!("Position {pos} is out of range for changeset of length {pos_a}");
        }
        Some(pos_b as usize)
    }

    /// Whether these changes touch the range `from`..`to`.
    pub fn touches_range(&self, from: usize, to: usize) -> Touch {
        let mut pos = 0;
        for s in self.sections.chunks(2) {
            if pos > to {
                break;
            }
            let (len, ins) = (s[0] as usize, s[1]);
            let end = pos + len;
            if ins >= 0 && pos <= to && end >= from {
                return if pos < from && end > to {
                    Touch::Cover
                } else {
                    Touch::Yes
                };
            }
            pos = end;
        }
        Touch::No
    }

    /// The JSON representation: the flat section list.
    pub fn to_json(&self) -> Vec<isize> {
        self.sections.clone()
    }

    /// Read a description from its JSON representation.
    pub fn from_json(json: &[isize]) -> Result<ChangeDesc, RangeError> {
        if !json.len().is_multiple_of(2) || json.chunks(2).any(|s| s[0] < 0 || s[1] < -1) {
            return Err(RangeError(
                "Invalid JSON representation of ChangeDesc".to_string(),
            ));
        }
        Ok(ChangeDesc::create(json.to_vec()))
    }
}

impl fmt::Display for ChangeDesc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, s) in self.sections.chunks(2).enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            write!(f, "{}", s[0])?;
            if s[1] >= 0 {
                write!(f, ":{}", s[1])?;
            }
        }
        Ok(())
    }
}

impl fmt::Debug for ChangeDesc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ChangeDesc({self})")
    }
}

/// Text to insert in a [`Change`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Insertion {
    /// A string, split into lines at the line separator.
    Str(String),
    /// A document.
    Text(Text),
}

/// One plain change: a deletion, insertion or replacement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The start of the replaced range.
    pub from: usize,
    /// The end of the replaced range; equal to `from` for an insertion.
    pub to: usize,
    /// The inserted content, if any.
    pub insert: Option<Insertion>,
}

/// A description of document changes, for [`ChangeSet::of`]: a plain
/// change, a change set, or a list of specs (`ChangeSpec` upstream).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeSpec {
    /// A deletion, insertion or replacement.
    Change(Change),
    /// A change set for the same document.
    Set(ChangeSet),
    /// Several specs, all in the coordinates of the starting document.
    List(Vec<ChangeSpec>),
}

impl ChangeSpec {
    /// Insert `text` at `from`.
    pub fn insert(from: usize, text: impl Into<String>) -> ChangeSpec {
        ChangeSpec::replace(from, from, text)
    }

    /// Delete `from`..`to`.
    pub fn delete(from: usize, to: usize) -> ChangeSpec {
        ChangeSpec::Change(Change {
            from,
            to,
            insert: None,
        })
    }

    /// Replace `from`..`to` with `text`.
    pub fn replace(from: usize, to: usize, text: impl Into<String>) -> ChangeSpec {
        ChangeSpec::Change(Change {
            from,
            to,
            insert: Some(Insertion::Str(text.into())),
        })
    }
}

impl From<Change> for ChangeSpec {
    fn from(change: Change) -> ChangeSpec {
        ChangeSpec::Change(change)
    }
}

impl From<ChangeSet> for ChangeSpec {
    fn from(set: ChangeSet) -> ChangeSpec {
        ChangeSpec::Set(set)
    }
}

impl From<Vec<ChangeSpec>> for ChangeSpec {
    fn from(list: Vec<ChangeSpec>) -> ChangeSpec {
        ChangeSpec::List(list)
    }
}

/// One part of a change set's JSON representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeJson {
    /// An unchanged range of this length (a number upstream).
    Retain(usize),
    /// A range of this length replaced by these lines; no lines for a
    /// deletion (an array upstream).
    Replace(usize, Vec<String>),
}

/// A group of changes to a document. It stores the document length and
/// applies only to documents of exactly that length.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct ChangeSet {
    desc: ChangeDesc,
    inserted: Vec<Text>,
}

impl ChangeSet {
    fn create_set(sections: Vec<isize>, inserted: Vec<Text>) -> ChangeSet {
        ChangeSet {
            desc: ChangeDesc::create(sections),
            inserted,
        }
    }

    /// The [change description](ChangeDesc) of this set.
    pub fn desc(&self) -> &ChangeDesc {
        &self.desc
    }

    /// The length of the document before the change.
    pub fn len(&self) -> usize {
        self.desc.len()
    }

    /// The length of the document after the change.
    pub fn new_len(&self) -> usize {
        self.desc.new_len()
    }

    /// False when there are actual changes in this set.
    pub fn is_empty(&self) -> bool {
        self.desc.is_empty()
    }

    /// See [`ChangeDesc::map_pos`].
    pub fn map_pos(&self, pos: usize, assoc: i32) -> usize {
        self.desc.map_pos(pos, assoc)
    }

    /// See [`ChangeDesc::map_pos_mode`].
    pub fn map_pos_mode(&self, pos: usize, assoc: i32, mode: MapMode) -> Option<usize> {
        self.desc.map_pos_mode(pos, assoc, mode)
    }

    /// See [`ChangeDesc::touches_range`].
    pub fn touches_range(&self, from: usize, to: usize) -> Touch {
        self.desc.touches_range(from, to)
    }

    /// See [`ChangeDesc::iter_gaps`].
    pub fn iter_gaps(&self, f: impl FnMut(usize, usize, usize)) {
        self.desc.iter_gaps(f)
    }

    /// See [`ChangeDesc::iter_changed_ranges`].
    pub fn iter_changed_ranges(&self, f: impl FnMut(usize, usize, usize, usize), individual: bool) {
        self.desc.iter_changed_ranges(f, individual)
    }

    /// Apply the changes to a document, returning the modified document.
    ///
    /// # Panics
    ///
    /// When the document's length differs from this set's.
    pub fn apply(&self, doc: &Text) -> Text {
        if self.len() != doc.len() {
            panic!("Applying change set to a document with the wrong length");
        }
        let mut doc = doc.clone();
        iter_changes(
            &self.desc.sections,
            Some(&self.inserted),
            |from_a, to_a, from_b, _to_b, text| {
                doc = doc.replace(from_b, from_b + (to_a - from_a), &text);
            },
            false,
        );
        doc
    }

    /// Given the document as it was _before_ these changes, the inverse
    /// set, which goes from the changed document back to that one.
    pub fn invert(&self, doc: &Text) -> ChangeSet {
        let mut sections = self.desc.sections.clone();
        let mut inserted = Vec::new();
        let mut pos = 0;
        for i in (0..sections.len()).step_by(2) {
            let (len, ins) = (sections[i], sections[i + 1]);
            if ins >= 0 {
                sections[i] = ins;
                sections[i + 1] = len;
                let index = i >> 1;
                while inserted.len() < index {
                    inserted.push(Text::empty());
                }
                inserted.push(if len > 0 {
                    doc.slice(pos, pos + len as usize)
                } else {
                    Text::empty()
                });
            }
            pos += len as usize;
        }
        ChangeSet::create_set(sections, inserted)
    }

    /// Combine two subsequent change sets: if this set goes from `docA` to
    /// `docB` and `other` from `docB` to `docC`, the result goes from
    /// `docA` to `docC`.
    ///
    /// # Panics
    ///
    /// When `other` does not start in the document this set produces.
    pub fn compose(&self, other: &ChangeSet) -> ChangeSet {
        if self.is_empty() {
            other.clone()
        } else if other.is_empty() {
            self.clone()
        } else {
            let (sections, inserted) = compose_sets(
                &self.desc.sections,
                Some(&self.inserted),
                &other.desc.sections,
                Some(&other.inserted),
            );
            ChangeSet::create_set(sections, inserted.expect("a set"))
        }
    }

    /// Map this set over `other`, which starts in the same document, so it
    /// applies to the document `other` produces. When `before` is true,
    /// order these changes before `other`'s; otherwise `other` comes first.
    ///
    /// For sets `a` and `b`, `a.compose(&b.map(a.desc(), false))` and
    /// `b.compose(&a.map(b.desc(), true))` produce the same document.
    ///
    /// # Panics
    ///
    /// When the lengths do not match.
    pub fn map(&self, other: &ChangeDesc, before: bool) -> ChangeSet {
        if other.is_empty() {
            return self.clone();
        }
        let (sections, inserted) = map_set(
            &self.desc.sections,
            Some(&self.inserted),
            &other.sections,
            before,
        );
        ChangeSet::create_set(sections, inserted.expect("a set"))
    }

    /// Iterate over the changed ranges, calling `f` with the range in the
    /// old document, the range that replaces it in the new document, and
    /// the inserted text. When `individual` is true, adjacent changes are
    /// reported separately.
    pub fn iter_changes(&self, f: impl FnMut(usize, usize, usize, usize, Text), individual: bool) {
        iter_changes(&self.desc.sections, Some(&self.inserted), f, individual);
    }

    /// The JSON representation of this set.
    pub fn to_json(&self) -> Vec<ChangeJson> {
        self.desc
            .sections
            .chunks(2)
            .enumerate()
            .map(|(i, s)| {
                let (len, ins) = (s[0] as usize, s[1]);
                if ins < 0 {
                    ChangeJson::Retain(len)
                } else if ins == 0 {
                    ChangeJson::Replace(len, Vec::new())
                } else {
                    ChangeJson::Replace(len, self.inserted[i].to_json())
                }
            })
            .collect()
    }

    /// Read a change set from its JSON representation.
    pub fn from_json(json: &[ChangeJson]) -> ChangeSet {
        let mut sections = Vec::new();
        let mut inserted = Vec::new();
        for (i, part) in json.iter().enumerate() {
            match part {
                ChangeJson::Retain(len) => sections.extend([*len as isize, -1]),
                ChangeJson::Replace(len, lines) if lines.is_empty() => {
                    sections.extend([*len as isize, 0])
                }
                ChangeJson::Replace(len, lines) => {
                    while inserted.len() < i {
                        inserted.push(Text::empty());
                    }
                    let text = Text::of(lines).expect("at least one line");
                    sections.extend([*len as isize, text.len() as isize]);
                    inserted.push(text);
                }
            }
        }
        ChangeSet::create_set(sections, inserted)
    }

    /// Create a change set for `changes` in a document of `length`,
    /// splitting inserted strings at `line_sep` (by default at `\r\n`, `\r`
    /// and `\n`).
    pub fn of(
        changes: impl Into<ChangeSpec>,
        length: usize,
        line_sep: Option<&str>,
    ) -> Result<ChangeSet, RangeError> {
        let mut builder = SetBuilder {
            length,
            line_sep,
            sections: Vec::new(),
            inserted: Vec::new(),
            pos: 0,
            total: None,
        };
        builder.process(&changes.into())?;
        let force = builder.total.is_none();
        builder.flush(force);
        Ok(builder.total.expect("flushed"))
    }

    /// An empty change set for a document of `length`.
    pub fn empty(length: usize) -> ChangeSet {
        ChangeSet::create_set(
            if length > 0 {
                vec![length as isize, -1]
            } else {
                Vec::new()
            },
            Vec::new(),
        )
    }
}

impl fmt::Debug for ChangeSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ChangeSet({:?})", self.to_json())
    }
}

impl fmt::Display for ChangeSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.desc, f)
    }
}

struct SetBuilder<'a> {
    length: usize,
    line_sep: Option<&'a str>,
    sections: Vec<isize>,
    inserted: Vec<Text>,
    pos: usize,
    total: Option<ChangeSet>,
}

impl SetBuilder<'_> {
    fn flush(&mut self, force: bool) {
        if !force && self.sections.is_empty() {
            return;
        }
        if self.pos < self.length {
            add_section(
                &mut self.sections,
                (self.length - self.pos) as isize,
                -1,
                false,
            );
        }
        let set = ChangeSet::create_set(
            std::mem::take(&mut self.sections),
            std::mem::take(&mut self.inserted),
        );
        self.add_total(set);
        self.pos = 0;
    }

    fn add_total(&mut self, set: ChangeSet) {
        self.total = Some(match self.total.take() {
            Some(total) => total.compose(&set.map(total.desc(), false)),
            None => set,
        });
    }

    fn process(&mut self, spec: &ChangeSpec) -> Result<(), RangeError> {
        match spec {
            ChangeSpec::List(list) => {
                for sub in list {
                    self.process(sub)?;
                }
            }
            ChangeSpec::Set(set) => {
                if set.len() != self.length {
                    return Err(RangeError(format!(
                        "Mismatched change set length (got {}, expected {})",
                        set.len(),
                        self.length
                    )));
                }
                self.flush(false);
                self.add_total(set.clone());
            }
            ChangeSpec::Change(Change { from, to, insert }) => {
                let (from, to) = (*from, *to);
                if from > to || to > self.length {
                    return Err(RangeError(format!(
                        "Invalid change range {from} to {to} (in doc of length {})",
                        self.length
                    )));
                }
                let ins_text = match insert {
                    None => Text::empty(),
                    Some(Insertion::Str(s)) if s.is_empty() => Text::empty(),
                    Some(Insertion::Str(s)) => Text::from_str_with(s, self.line_sep),
                    Some(Insertion::Text(text)) => text.clone(),
                };
                let ins_len = ins_text.len();
                if from == to && ins_len == 0 {
                    return Ok(());
                }
                if from < self.pos {
                    self.flush(false);
                }
                if from > self.pos {
                    add_section(&mut self.sections, (from - self.pos) as isize, -1, false);
                }
                add_section(
                    &mut self.sections,
                    (to - from) as isize,
                    ins_len as isize,
                    false,
                );
                add_insert(&mut self.inserted, &self.sections, &ins_text);
                self.pos = to;
            }
        }
        Ok(())
    }
}

fn add_section(sections: &mut Vec<isize>, len: isize, ins: isize, force_join: bool) {
    if len == 0 && ins <= 0 {
        return;
    }
    let n = sections.len();
    if n >= 2 && ins <= 0 && ins == sections[n - 1] {
        sections[n - 2] += len;
    } else if n >= 2 && len == 0 && sections[n - 2] == 0 {
        sections[n - 1] += ins;
    } else if force_join {
        sections[n - 2] += len;
        sections[n - 1] += ins;
    } else {
        sections.extend([len, ins]);
    }
}

fn add_insert(values: &mut Vec<Text>, sections: &[isize], value: &Text) {
    if value.is_empty() {
        return;
    }
    let index = (sections.len() - 2) >> 1;
    if index < values.len() {
        let last = values.last_mut().expect("a value");
        *last = last.append(value);
    } else {
        while values.len() < index {
            values.push(Text::empty());
        }
        values.push(value.clone());
    }
}

fn iter_changes(
    sections: &[isize],
    inserted: Option<&[Text]>,
    mut f: impl FnMut(usize, usize, usize, usize, Text),
    individual: bool,
) {
    let (mut pos_a, mut pos_b, mut i) = (0isize, 0isize, 0);
    while i < sections.len() {
        let (mut len, mut ins) = (sections[i], sections[i + 1]);
        i += 2;
        if ins < 0 {
            pos_a += len;
            pos_b += len;
        } else {
            let (mut end_a, mut end_b) = (pos_a, pos_b);
            let mut text = Text::empty();
            loop {
                end_a += len;
                end_b += ins;
                if ins != 0 {
                    if let Some(part) = inserted.and_then(|all| all.get((i - 2) >> 1)) {
                        text = text.append(part);
                    }
                }
                if individual || i == sections.len() || sections[i + 1] < 0 {
                    break;
                }
                len = sections[i];
                ins = sections[i + 1];
                i += 2;
            }
            f(
                pos_a as usize,
                end_a as usize,
                pos_b as usize,
                end_b as usize,
                text,
            );
            pos_a = end_a;
            pos_b = end_b;
        }
    }
}

/// Produce a copy of set A that applies to the document after set B
/// (both starting in the same document). With `inserted_a`, also the text.
fn map_set(
    sections_a: &[isize],
    inserted_a: Option<&[Text]>,
    sections_b: &[isize],
    before: bool,
) -> (Vec<isize>, Option<Vec<Text>>) {
    let mut sections = Vec::new();
    let mut insert: Option<Vec<Text>> = inserted_a.map(|_| Vec::new());
    let mut a = SectionIter::new(sections_a, inserted_a);
    let mut b = SectionIter::new(sections_b, None);
    // For changes in A that are processed piece by piece, `inserted` is
    // the section index of the one whose content was already inserted.
    let mut inserted: isize = -1;
    loop {
        if a.done() && b.len != 0 || b.done() && a.len != 0 {
            panic!("Mismatched change set lengths");
        } else if a.ins == -1 && b.ins == -1 {
            // Move across ranges skipped by both sets.
            let len = a.len.min(b.len);
            add_section(&mut sections, len, -1, false);
            a.forward(len);
            b.forward(len);
        } else if b.ins >= 0
            && (a.ins < 0
                || inserted == a.i as isize
                || a.off == 0 && (b.len < a.len || b.len == a.len && !before))
        {
            // A change in B comes before the next change in A (ordered by
            // start, then length, then the before flag): skip it, and
            // process any changes in A it covers.
            let mut len = b.len;
            add_section(&mut sections, b.ins, -1, false);
            while len != 0 {
                let piece = a.len.min(len);
                if a.ins >= 0 && inserted < a.i as isize && a.len <= piece {
                    add_section(&mut sections, 0, a.ins, false);
                    if let Some(insert) = insert.as_mut() {
                        add_insert(insert, &sections, &a.text());
                    }
                    inserted = a.i as isize;
                }
                a.forward(piece);
                len -= piece;
            }
            b.next();
        } else if a.ins >= 0 {
            // Process the part of a change in A up to the start of the next
            // non-deletion change in B (if overlapping).
            let (mut len, mut left) = (0, a.len);
            while left != 0 {
                if b.ins == -1 {
                    let piece = left.min(b.len);
                    len += piece;
                    left -= piece;
                    b.forward(piece);
                } else if b.ins == 0 && b.len < left {
                    left -= b.len;
                    b.next();
                } else {
                    break;
                }
            }
            let fresh = inserted < a.i as isize;
            add_section(&mut sections, len, if fresh { a.ins } else { 0 }, false);
            if fresh {
                if let Some(insert) = insert.as_mut() {
                    add_insert(insert, &sections, &a.text());
                }
            }
            inserted = a.i as isize;
            a.forward(a.len - left);
        } else if a.done() && b.done() {
            return (sections, insert);
        } else {
            panic!("Mismatched change set lengths");
        }
    }
}

fn compose_sets(
    sections_a: &[isize],
    inserted_a: Option<&[Text]>,
    sections_b: &[isize],
    inserted_b: Option<&[Text]>,
) -> (Vec<isize>, Option<Vec<Text>>) {
    let mut sections = Vec::new();
    let mut insert: Option<Vec<Text>> = inserted_a.map(|_| Vec::new());
    let mut a = SectionIter::new(sections_a, inserted_a);
    let mut b = SectionIter::new(sections_b, inserted_b);
    let mut open = false;
    loop {
        if a.done() && b.done() {
            return (sections, insert);
        } else if a.ins == 0 {
            // Deletion in A.
            add_section(&mut sections, a.len, 0, open);
            a.next();
        } else if b.len == 0 && !b.done() {
            // Insertion in B.
            add_section(&mut sections, 0, b.ins, open);
            if let Some(insert) = insert.as_mut() {
                add_insert(insert, &sections, &b.text());
            }
            b.next();
        } else if a.done() || b.done() {
            panic!("Mismatched change set lengths");
        } else {
            let len = a.len2().min(b.len);
            let section_len = sections.len();
            if a.ins == -1 {
                let ins_b = if b.ins == -1 {
                    -1
                } else if b.off != 0 {
                    0
                } else {
                    b.ins
                };
                add_section(&mut sections, len, ins_b, open);
                if ins_b != 0 {
                    if let Some(insert) = insert.as_mut() {
                        add_insert(insert, &sections, &b.text());
                    }
                }
            } else if b.ins == -1 {
                add_section(&mut sections, if a.off != 0 { 0 } else { a.len }, len, open);
                if let Some(insert) = insert.as_mut() {
                    add_insert(insert, &sections, &a.text_bit(Some(len)));
                }
            } else {
                add_section(
                    &mut sections,
                    if a.off != 0 { 0 } else { a.len },
                    if b.off != 0 { 0 } else { b.ins },
                    open,
                );
                if b.off == 0 {
                    if let Some(insert) = insert.as_mut() {
                        add_insert(insert, &sections, &b.text());
                    }
                }
            }
            open = (a.ins > len || b.ins >= 0 && b.len > len)
                && (open || sections.len() > section_len);
            a.forward2(len);
            b.forward(len);
        }
    }
}

struct SectionIter<'a> {
    sections: &'a [isize],
    inserted: Option<&'a [Text]>,
    i: usize,
    len: isize,
    off: isize,
    ins: isize,
}

impl<'a> SectionIter<'a> {
    fn new(sections: &'a [isize], inserted: Option<&'a [Text]>) -> SectionIter<'a> {
        let mut iter = SectionIter {
            sections,
            inserted,
            i: 0,
            len: 0,
            off: 0,
            ins: 0,
        };
        iter.next();
        iter
    }

    fn next(&mut self) {
        if self.i < self.sections.len() {
            self.len = self.sections[self.i];
            self.ins = self.sections[self.i + 1];
            self.i += 2;
        } else {
            self.len = 0;
            self.ins = -2;
        }
        self.off = 0;
    }

    fn done(&self) -> bool {
        self.ins == -2
    }

    fn len2(&self) -> isize {
        if self.ins < 0 {
            self.len
        } else {
            self.ins
        }
    }

    fn text(&self) -> Text {
        let index = (self.i - 2) >> 1;
        self.inserted
            .and_then(|all| all.get(index))
            .cloned()
            .unwrap_or_else(Text::empty)
    }

    fn text_bit(&self, len: Option<isize>) -> Text {
        let index = (self.i - 2) >> 1;
        match self.inserted.and_then(|all| all.get(index)) {
            None => Text::empty(),
            Some(text) => {
                let from = self.off as usize;
                let to = len.map_or(text.len(), |len| from + len as usize);
                text.slice(from, to)
            }
        }
    }

    fn forward(&mut self, len: isize) {
        if len == self.len {
            self.next();
        } else {
            self.len -= len;
            self.off += len;
        }
    }

    fn forward2(&mut self, len: isize) {
        if self.ins == -1 {
            self.forward(len);
        } else if len == self.ins {
            self.next();
        } else {
            self.ins -= len;
            self.off += len;
        }
    }
}
