//! A port of `bidi.ts`: the text direction, the bidi algorithm CM6 uses
//! to order a line's text visually ([`BidiSpan`], `computeOrder`), and
//! visual cursor motion over that order (`moveVisually`).
//!
//! Positions are UTF-8 byte offsets into the line, where upstream's are
//! UTF-16 code units. The algorithm runs over the line's characters, so
//! its index arithmetic stays upstream's; a character outside the Basic
//! Multilingual Plane, two left-to-right units upstream, is one here,
//! which orders the same.

use editor_state::{find_cluster_break, EditorSelection, Line, SelectionRange};

/// A text direction (`Direction`). The values are the bidi base levels of
/// spans in that direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Left-to-right.
    Ltr = 0,
    /// Right-to-left.
    Rtl = 1,
}

// Character types (`T`).
const L: u16 = 1; // Left-to-right
const R: u16 = 2; // Right-to-left
const AL: u16 = 4; // Right-to-left Arabic
const EN: u16 = 8; // European number
const AN: u16 = 16; // Arabic number
const ET: u16 = 64; // European number terminator
const CS: u16 = 128; // Common number separator
const NI: u16 = 256; // Neutral or isolate (BN, N, WS)
const NSM: u16 = 512; // Non-spacing mark
const STRONG: u16 = L | R | AL;
const NUM: u16 = EN | AN;

// The types of code points 0 to 0xf7, each encoded as log2(type).
const LOW_TYPES: &[u8] = concat!(
    "8888888888888888888888888888888888866688888878783333333333788888",
    "8000000000000000000000000008888880000000000000000000000000088888",
    "8888888888888888888888888888888878666688880888886633808883088888",
    "00000000000000000000000800000000000000000000000000000008",
)
.as_bytes();

// The types of code points 0x600 to 0x6f9.
const ARABIC_TYPES: &[u8] = concat!(
    "4444448826627288999999999992222222222222222222222222222222222222",
    "2222222222299999999999999999999944444444446442228222222222222222",
    "2222222222222222222222222222222222222222222222222222222222222222",
    "2222222222222222222222999999949999999229989999223333333333",
)
.as_bytes();

fn dec(code: u8) -> u16 {
    1 << (code - b'0')
}

// `charType`.
fn char_type(ch: char) -> u16 {
    let ch = ch as u32;
    match ch {
        0..=0xf7 => dec(LOW_TYPES[ch as usize]),
        0x590..=0x5f4 => R,
        0x600..=0x6f9 => dec(ARABIC_TYPES[(ch - 0x600) as usize]),
        // 0x6ee to 0x6f9 come from the table above, as upstream checks it first.
        0x6fa..=0x8ac => AL,
        0x2000..=0x200c => NI,
        0xfb50..=0xfdff => AL,
        _ => L,
    }
}

// `BidiRE`: whether a character can make a line need the algorithm.
fn is_bidi_char(ch: char) -> bool {
    matches!(ch as u32, 0x590..=0x5f4 | 0x600..=0x6ff | 0x700..=0x8ac | 0xfb50..=0xfdff)
}

// `Brackets`: the closing bracket of an opening one, or the negated
// opening bracket of a closing one. Upstream leaves out the rest of
// BidiBrackets.txt to keep its size down.
fn bracket(ch: char) -> i64 {
    match ch {
        '(' => ')' as i64,
        ')' => -('(' as i64),
        '[' => ']' as i64,
        ']' => -('[' as i64),
        '{' => '}' as i64,
        '}' => -('{' as i64),
        _ => 0,
    }
}

// Tracks direction in and before bracketed ranges (`Bracketed`).
const OPPOSITE_BEFORE: usize = 1;
const EMBED_INSIDE: usize = 2;
const OPPOSITE_INSIDE: usize = 4;
const MAX_DEPTH: usize = 3 * 63;

/// A stretch of a line with a single direction (`BidiSpan`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BidiSpan {
    /// The start of the span, relative to the start of the line.
    pub from: usize,
    /// The end of the span.
    pub to: usize,
    /// The span's [bidi level](https://unicode.org/reports/tr9/#Basic_Display_Algorithm):
    /// 0 is left-to-right, 1 right-to-left, 2 a left-to-right number
    /// inside right-to-left text.
    pub level: u8,
}

impl BidiSpan {
    /// The span's direction.
    pub fn dir(&self) -> Direction {
        if self.level % 2 == 1 {
            Direction::Rtl
        } else {
            Direction::Ltr
        }
    }

    pub(crate) fn side(&self, end: bool, dir: Direction) -> usize {
        if (self.dir() == dir) == end {
            self.to
        } else {
            self.from
        }
    }

    pub(crate) fn forward(&self, forward: bool, dir: Direction) -> bool {
        forward == (self.dir() == dir)
    }

    /// The index in `order` of the span holding `index` (`BidiSpan.find`):
    /// the one at `level` if any, else, when several hold it, the one on
    /// the `assoc` side, or with the lowest level when `assoc` is 0.
    ///
    /// # Panics
    ///
    /// When no span holds `index`, where upstream throws a `RangeError`.
    pub(crate) fn find(order: &[BidiSpan], index: usize, level: i32, assoc: i32) -> usize {
        let mut maybe: Option<usize> = None;
        for (i, span) in order.iter().enumerate() {
            if span.from <= index && span.to >= index {
                if i32::from(span.level) == level {
                    return i;
                }
                // When several spans match, take the one on the `assoc`
                // side, or, without one, the one with the lowest level.
                let better = match maybe {
                    None => true,
                    Some(_) if assoc < 0 => span.from < index,
                    Some(_) if assoc > 0 => span.to > index,
                    Some(m) => order[m].level > span.level,
                };
                if better {
                    maybe = Some(i);
                }
            }
        }
        maybe.expect("Index out of range")
    }
}

/// A range of a line isolated from the bidi ordering around it
/// (`Isolate`). Isolates are sorted by position and never empty; nested
/// ones stay inside their parent. Equality is `isolatesEq`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Isolate {
    /// The start, relative to the start of the line.
    pub from: usize,
    /// The end.
    pub to: usize,
    /// The isolate's direction.
    pub direction: Direction,
    /// The isolates inside this one.
    pub inner: Vec<Isolate>,
}

// The line's characters and the working state of one `computeOrder`:
// the character types, which upstream keeps in a reused global array,
// and the spans.
struct Order<'a> {
    chars: &'a [char],
    types: Vec<u16>,
    spans: Vec<BidiSpan>,
}

// The stretch before isolate `i` (after isolate `i - 1`) of `r_from..r_to`.
fn segment(isolates: &[Isolate], i: usize, r_from: usize, r_to: usize) -> (usize, usize) {
    let from = if i > 0 { isolates[i - 1].to } else { r_from };
    let to = if i < isolates.len() {
        isolates[i].from
    } else {
        r_to
    };
    (from, to)
}

impl Order<'_> {
    // `computeCharTypes`: fill in the character types from `r_from` to
    // `r_to` and apply the W rules.
    fn compute_char_types(&mut self, r_from: usize, r_to: usize, isolates: &[Isolate], outer: u16) {
        let types = &mut self.types;
        for i_i in 0..=isolates.len() {
            let (from, to) = segment(isolates, i_i, r_from, r_to);
            let prev_type = if i_i > 0 { NI } else { outer };

            // W1. Examine each non-spacing mark (NSM) in the level run, and
            // change the type of the NSM to the type of the previous
            // character. If the NSM is at the start of the level run, it
            // will get the type of sor.
            // W2. Search backwards from each instance of a European number
            // until the first strong type (R, L, AL, or sor) is found. If
            // an AL is found, change the type of the European number to
            // Arabic number.
            // W3. Change all ALs to R.
            // (Left after this: L, R, EN, AN, ET, CS, NI)
            let (mut prev, mut prev_strong) = (prev_type, prev_type);
            for (slot, &ch) in types[from..to].iter_mut().zip(&self.chars[from..to]) {
                let mut ty = char_type(ch);
                if ty == NSM {
                    ty = prev;
                } else if ty == EN && prev_strong == AL {
                    ty = AN;
                }
                *slot = if ty == AL { R } else { ty };
                if ty & STRONG != 0 {
                    prev_strong = ty;
                }
                prev = ty;
            }

            // W5. A sequence of European terminators adjacent to European
            // numbers changes to all European numbers.
            // W6. Otherwise, separators and terminators change to Other
            // Neutral.
            // W7. Search backwards from each instance of a European number
            // until the first strong type (R, L, or sor) is found. If an L
            // is found, then change the type of the European number to L.
            // (Left after this: L, R, EN+AN, NI)
            let (mut prev, mut prev_strong) = (prev_type, prev_type);
            let mut i = from;
            while i < to {
                let mut ty = types[i];
                if ty == CS {
                    if i + 1 < to && prev == types[i + 1] && prev & NUM != 0 {
                        ty = prev;
                        types[i] = prev;
                    } else {
                        types[i] = NI;
                    }
                } else if ty == ET {
                    let mut end = i + 1;
                    while end < to && types[end] == ET {
                        end += 1;
                    }
                    let replace = if (i != 0 && prev == EN) || (end < r_to && types[end] == EN) {
                        if prev_strong == L {
                            L
                        } else {
                            EN
                        }
                    } else {
                        NI
                    };
                    types[i..end].fill(replace);
                    i = end - 1;
                } else if ty == EN && prev_strong == L {
                    types[i] = L;
                }
                prev = ty;
                if ty & STRONG != 0 {
                    prev_strong = ty;
                }
                i += 1;
            }
        }
    }

    // `processBracketPairs`: process brackets throughout a run sequence.
    fn process_bracket_pairs(
        &mut self,
        r_from: usize,
        r_to: usize,
        isolates: &[Isolate],
        outer: u16,
    ) {
        let opposite = if outer == L { R } else { L };
        let types = &mut self.types;
        // [start index, bracket, flags] triples for each open bracket
        // (`BracketStack`).
        let mut stack: Vec<usize> = Vec::new();
        let (mut s_i, mut context) = (0usize, 0usize);
        for i_i in 0..=isolates.len() {
            let (from, to) = segment(isolates, i_i, r_from, r_to);
            // N0. Process bracket pairs in an isolating run sequence
            // sequentially in the logical order of the text positions of
            // the opening paired brackets using the logic given below.
            // Within this scope, bidirectional types EN and AN are treated
            // as R.
            for i in from..to {
                let ch = self.chars[i];
                let br = bracket(ch);
                if br != 0 {
                    if br < 0 {
                        // A closing bracket.
                        let mut s_j = s_i;
                        while s_j >= 3 {
                            s_j -= 3;
                            if stack[s_j + 1] == (-br) as usize {
                                let flags = stack[s_j + 2];
                                let ty = if flags & EMBED_INSIDE != 0 {
                                    outer
                                } else if flags & OPPOSITE_INSIDE == 0 {
                                    0
                                } else if flags & OPPOSITE_BEFORE != 0 {
                                    opposite
                                } else {
                                    outer
                                };
                                if ty != 0 {
                                    types[i] = ty;
                                    types[stack[s_j]] = ty;
                                }
                                s_i = s_j;
                                break;
                            }
                        }
                    } else if s_i == MAX_DEPTH {
                        // Upstream compares the shared stack's length,
                        // which keeps its largest size across calls; the
                        // port compares this call's depth.
                        break;
                    } else {
                        stack.truncate(s_i);
                        stack.extend([i, ch as usize, context]);
                        s_i += 3;
                    }
                } else if types[i] == R || types[i] == L {
                    let embed = types[i] == outer;
                    context = if embed { 0 } else { OPPOSITE_BEFORE };
                    let mut s_j = s_i;
                    while s_j >= 3 {
                        s_j -= 3;
                        let cur = stack[s_j + 2];
                        if cur & EMBED_INSIDE != 0 {
                            break;
                        }
                        if embed {
                            stack[s_j + 2] |= EMBED_INSIDE;
                        } else {
                            if cur & OPPOSITE_INSIDE != 0 {
                                break;
                            }
                            stack[s_j + 2] |= OPPOSITE_INSIDE;
                        }
                    }
                }
            }
        }
    }

    // `processNeutrals`.
    fn process_neutrals(&mut self, r_from: usize, r_to: usize, isolates: &[Isolate], outer: u16) {
        let types = &mut self.types;
        let mut prev = outer;
        let mut i_i = 0;
        while i_i <= isolates.len() {
            let (from, mut to) = segment(isolates, i_i, r_from, r_to);
            // N1. A sequence of neutrals takes the direction of the
            // surrounding strong text if the text on both sides has the
            // same direction. European and Arabic numbers act as if they
            // were R in terms of their influence on neutrals.
            // Start-of-level-run (sor) and end-of-level-run (eor) are used
            // at level run boundaries.
            // N2. Any remaining neutrals take the embedding direction.
            // (Left after this: L, R, EN+AN)
            let mut i = from;
            while i < to {
                let ty = types[i];
                if ty == NI {
                    let mut end = i + 1;
                    loop {
                        if end == to {
                            if i_i == isolates.len() {
                                break;
                            }
                            end = isolates[i_i].to;
                            i_i += 1;
                            to = segment(isolates, i_i, r_from, r_to).1;
                        } else if types[end] == NI {
                            end += 1;
                        } else {
                            break;
                        }
                    }
                    let before_l = prev == L;
                    let after_l = (if end < r_to { types[end] } else { outer }) == L;
                    let replace = if before_l == after_l {
                        if before_l {
                            L
                        } else {
                            R
                        }
                    } else {
                        outer
                    };
                    let (mut j, mut j_i) = (end, i_i);
                    let mut from_j = segment(isolates, j_i, r_from, r_to).0;
                    while j > i {
                        if j == from_j {
                            j_i -= 1;
                            j = isolates[j_i].from;
                            from_j = segment(isolates, j_i, r_from, r_to).0;
                        }
                        j -= 1;
                        types[j] = replace;
                    }
                    i = end;
                } else {
                    prev = ty;
                    i += 1;
                }
            }
            i_i += 1;
        }
    }

    // `emitSpans`: find the runs of character types from `from` to `to`
    // and emit spans for them, in reverse order where the level's
    // direction is not the base direction, computing the isolates'
    // spans where they fall.
    fn emit_spans(
        &mut self,
        from: usize,
        to: usize,
        level: u8,
        base_level: u8,
        isolates: &[Isolate],
    ) {
        let our_type = if level % 2 == 1 { R } else { L };

        if level % 2 == base_level % 2 {
            // The base direction: no flipping.
            let (mut i_ch, mut i_i) = (from, 0);
            while i_ch < to {
                // Scan a section of characters in direction `our_type`,
                // unless another type of character comes right after
                // `i_ch`, in which case scan a section of other
                // characters (which, if `our_type` is L, may hold both R
                // and AN).
                let (mut same_dir, mut is_num) = (true, false);
                if i_i == isolates.len() || i_ch < isolates[i_i].from {
                    let next = self.types[i_ch];
                    if next != our_type {
                        same_dir = false;
                        is_num = next == AN;
                    }
                }
                // The isolates to pass to a recursive call, when we must
                // recurse (to tell AN inside a right-to-left section in
                // left-to-right text), or `None` to emit directly.
                let mut recurse: Option<Vec<Isolate>> = (!same_dir && our_type == L).then(Vec::new);
                let local_level = if same_dir { level } else { level + 1 };
                let mut i_scan = i_ch;
                'run: loop {
                    if i_i < isolates.len() && i_scan == isolates[i_i].from {
                        if is_num {
                            break 'run;
                        }
                        let iso = &isolates[i_i];
                        // Check that another character in this direction
                        // comes after the isolates.
                        if !same_dir {
                            let (mut upto, mut j_i) = (iso.to, i_i + 1);
                            loop {
                                if upto == to {
                                    break 'run;
                                }
                                if j_i < isolates.len() && isolates[j_i].from == upto {
                                    upto = isolates[j_i].to;
                                    j_i += 1;
                                } else if self.types[upto] == our_type {
                                    break 'run;
                                } else {
                                    break;
                                }
                            }
                        }
                        i_i += 1;
                        if let Some(recurse) = &mut recurse {
                            recurse.push(iso.clone());
                        } else {
                            if iso.from > i_ch {
                                self.push(i_ch, iso.from, local_level);
                            }
                            let dir_swap =
                                (iso.direction == Direction::Ltr) != (local_level % 2 == 0);
                            self.compute_section_order(
                                if dir_swap { level + 1 } else { level },
                                base_level,
                                &iso.inner,
                                iso.from,
                                iso.to,
                            );
                            i_ch = iso.to;
                        }
                        i_scan = iso.to;
                    } else if i_scan == to
                        || (if same_dir {
                            self.types[i_scan] != our_type
                        } else {
                            self.types[i_scan] == our_type
                        })
                    {
                        break;
                    } else {
                        i_scan += 1;
                    }
                }
                if let Some(recurse) = recurse {
                    self.emit_spans(i_ch, i_scan, level + 1, base_level, &recurse);
                } else if i_ch < i_scan {
                    self.push(i_ch, i_scan, local_level);
                }
                i_ch = i_scan;
            }
        } else {
            // Iterate in reverse to flip the span order: the same code,
            // from the back of the section to the front.
            let (mut i_ch, mut i_i) = (to, isolates.len());
            while i_ch > from {
                let (mut same_dir, mut is_num) = (true, false);
                if i_i == 0 || i_ch > isolates[i_i - 1].to {
                    let next = self.types[i_ch - 1];
                    if next != our_type {
                        same_dir = false;
                        is_num = next == AN;
                    }
                }
                let mut recurse: Option<Vec<Isolate>> = (!same_dir && our_type == L).then(Vec::new);
                let local_level = if same_dir { level } else { level + 1 };
                let mut i_scan = i_ch;
                'run: loop {
                    if i_i > 0 && i_scan == isolates[i_i - 1].to {
                        if is_num {
                            break 'run;
                        }
                        i_i -= 1;
                        let iso = &isolates[i_i];
                        // Check that another character in this direction
                        // comes after the isolates.
                        if !same_dir {
                            let (mut upto, mut j_i) = (iso.from, i_i);
                            loop {
                                if upto == from {
                                    break 'run;
                                }
                                if j_i > 0 && isolates[j_i - 1].to == upto {
                                    j_i -= 1;
                                    upto = isolates[j_i].from;
                                } else if self.types[upto - 1] == our_type {
                                    break 'run;
                                } else {
                                    break;
                                }
                            }
                        }
                        if let Some(recurse) = &mut recurse {
                            recurse.push(iso.clone());
                        } else {
                            if iso.to < i_ch {
                                self.push(iso.to, i_ch, local_level);
                            }
                            let dir_swap =
                                (iso.direction == Direction::Ltr) != (local_level % 2 == 0);
                            self.compute_section_order(
                                if dir_swap { level + 1 } else { level },
                                base_level,
                                &iso.inner,
                                iso.from,
                                iso.to,
                            );
                            i_ch = iso.from;
                        }
                        i_scan = iso.from;
                    } else if i_scan == from
                        || (if same_dir {
                            self.types[i_scan - 1] != our_type
                        } else {
                            self.types[i_scan - 1] == our_type
                        })
                    {
                        break;
                    } else {
                        i_scan -= 1;
                    }
                }
                if let Some(recurse) = recurse {
                    self.emit_spans(i_scan, i_ch, level + 1, base_level, &recurse);
                } else if i_scan < i_ch {
                    self.push(i_scan, i_ch, local_level);
                }
                i_ch = i_scan;
            }
        }
    }

    fn push(&mut self, from: usize, to: usize, level: u8) {
        self.spans.push(BidiSpan { from, to, level });
    }

    // `computeSectionOrder`.
    fn compute_section_order(
        &mut self,
        level: u8,
        base_level: u8,
        isolates: &[Isolate],
        from: usize,
        to: usize,
    ) {
        let outer = if level % 2 == 1 { R } else { L };
        self.compute_char_types(from, to, isolates, outer);
        self.process_bracket_pairs(from, to, isolates, outer);
        self.process_neutrals(from, to, isolates, outer);
        self.emit_spans(from, to, level, base_level, isolates);
    }
}

// The isolates with character indices for byte offsets.
fn isolates_by_char(isolates: &[Isolate], starts: &[usize]) -> Vec<Isolate> {
    let index = |pos: usize| starts.partition_point(|&start| start < pos);
    isolates
        .iter()
        .map(|iso| Isolate {
            from: index(iso.from),
            to: index(iso.to),
            direction: iso.direction,
            inner: isolates_by_char(&iso.inner, starts),
        })
        .collect()
}

/// The bidi spans of `line` in visual order for `direction`, with
/// `isolates` ordered on their own (`computeOrder`): left to right for a
/// left-to-right direction, right to left otherwise.
pub fn compute_order(line: &str, direction: Direction, isolates: &[Isolate]) -> Vec<BidiSpan> {
    if line.is_empty() {
        return vec![BidiSpan {
            from: 0,
            to: 0,
            level: direction as u8,
        }];
    }
    if direction == Direction::Ltr && isolates.is_empty() && !line.chars().any(is_bidi_char) {
        return trivial_order(line.len());
    }
    let chars: Vec<char> = line.chars().collect();
    let starts: Vec<usize> = line
        .char_indices()
        .map(|(i, _)| i)
        .chain([line.len()])
        .collect();
    let isolates = isolates_by_char(isolates, &starts);
    let mut order = Order {
        chars: &chars,
        types: vec![NI; chars.len()],
        spans: Vec::new(),
    };
    let level = direction as u8;
    order.compute_section_order(level, level, &isolates, 0, chars.len());
    order
        .spans
        .into_iter()
        .map(|span| BidiSpan {
            from: starts[span.from],
            to: starts[span.to],
            level: span.level,
        })
        .collect()
}

/// One left-to-right span over a line of `length` (`trivialOrder`).
pub fn trivial_order(length: usize) -> Vec<BidiSpan> {
    vec![BidiSpan {
        from: 0,
        to: length,
        level: 0,
    }]
}

/// Move `start` one cluster visually, by the line's bidi `order` in
/// direction `dir` (`moveVisually`), or return `None` at the line's
/// visual start or end.
///
/// This moves strictly visually, without making sure that every logical
/// position is visited. Simple input still visits them all, but, for
/// example, several isolates of the same level next to each other make
/// some positions unreachable. Each visible cursor position is that of
/// the lower-level span touching it, except at the start and end of a
/// line whose text there is not in the dominant direction, where the
/// cursor sits at the line's start or end with an `assoc` pointing out
/// of the line.
pub fn move_visually(
    line: &Line,
    order: &[BidiSpan],
    dir: Direction,
    start: &SelectionRange,
    forward: bool,
) -> Option<SelectionRange> {
    move_visually_over(line, order, dir, start, forward).map(|(range, _)| range)
}

/// [`move_visually`], also returning the text moved over (upstream's
/// `movedOver`).
pub(crate) fn move_visually_over<'a>(
    line: &Line<'a>,
    order: &[BidiSpan],
    dir: Direction,
    start: &SelectionRange,
    forward: bool,
) -> Option<(SelectionRange, &'a str)> {
    if line.is_empty() {
        return None;
    }
    let mut start_index = start.head() - line.from;
    let mut span_i;
    if start.head() == line.from && start.assoc() < 0 {
        // The start of the line.
        if !forward {
            return None;
        }
        span_i = 0;
        start_index = order[span_i].side(false, dir);
    } else if start.head() == line.to && start.assoc() > 0 {
        // The end of the line.
        if forward {
            return None;
        }
        span_i = order.len() - 1;
        start_index = order[span_i].side(true, dir);
    } else {
        span_i = BidiSpan::find(
            order,
            start_index,
            start.bidi_level().map_or(-1, i32::from),
            i32::from(start.assoc()),
        );
    }
    let mut span = order[span_i];
    let mut span_end = span.side(forward, dir);
    // The end of the span.
    if start_index == span_end {
        let next_i = if forward {
            span_i + 1
        } else {
            span_i.checked_sub(1)?
        };
        if next_i >= order.len() {
            return None;
        }
        span_i = next_i;
        span = order[span_i];
        start_index = span.side(!forward, dir);
        span_end = span.side(forward, dir);
    }
    let mut next_index =
        find_cluster_break(line.text, start_index, span.forward(forward, dir), true);
    if next_index < span.from || next_index > span.to {
        next_index = span_end;
    }
    let moved_over = &line.text[start_index.min(next_index)..start_index.max(next_index)];

    let next_span = if span_i == (if forward { order.len() - 1 } else { 0 }) {
        None
    } else {
        Some(order[if forward { span_i + 1 } else { span_i - 1 }])
    };
    if next_index == span_end {
        // The end of the line.
        let Some(next_span) = next_span else {
            let cursor = if forward {
                EditorSelection::cursor_with(line.to, 1, None, None)
            } else {
                EditorSelection::cursor_with(line.from, -1, None, None)
            };
            return Some((cursor, moved_over));
        };
        if next_span.level + u8::from(!forward) < span.level {
            let cursor = EditorSelection::cursor_with(
                next_span.side(!forward, dir) + line.from,
                if next_span.forward(forward, dir) {
                    1
                } else {
                    -1
                },
                Some(next_span.level),
                None,
            );
            return Some((cursor, moved_over));
        }
    }
    let cursor = EditorSelection::cursor_with(
        next_index + line.from,
        if span.forward(forward, dir) { -1 } else { 1 },
        Some(span.level),
        None,
    );
    Some((cursor, moved_over))
}

/// The direction of the first strong character in `text[from..to]`, if
/// any (`autoDirection`).
pub fn auto_direction(text: &str, from: usize, to: usize) -> Option<Direction> {
    for ch in text[from..to].chars() {
        match char_type(ch) {
            L => return Some(Direction::Ltr),
            R | AL => return Some(Direction::Rtl),
            _ => {}
        }
    }
    None
}
