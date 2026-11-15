//! Port of the drawing tests of @codemirror/view 6.43.15,
//! `test/webtest-draw.ts` and `test/webtest-draw-decoration.ts`, that need
//! only the content's DOM, on Div's DOM.
//!
//! Upstream runs them on an editor view in a browser. These run the part
//! of the view that draws ([`ContentTiles`]) on a Div document, and read
//! the result from Div's DOM. Differences from upstream, each for a Div
//! gap or for a part of the view not yet ported:
//!
//! - Div names elements by kind (div-gap 25): `em`, `strong` and `span`
//!   all read as `span`, and tags Div does not know (`b`, `var`,
//!   `navigation`, custom elements) cannot be built. Tests that tell
//!   elements apart by tag use classes or `data-test` attributes instead.
//! - Div cannot walk or hold text nodes from the DOM (div-gap 24), so
//!   checks that a text node was kept check its line element instead, and
//!   the DOM is read through [`ContentTiles::block_html`].
//! - There is no viewport yet, so the whole document is drawn, and
//!   upstream's random tests compare against the whole document. Their
//!   random numbers come from a fixed-seed generator.
//! - Tests of selection, coordinates, scrolling, measuring, focus and
//!   plugins wait for the parts of the view they need.

use std::sync::{Arc, Mutex};

use div::{Document, Element};
use editor_state::{ChangeSet, ChangeSpec, Range, RangeSet, RangeSetUpdate, RangeValue, Text};
use editor_view::__test::ContentTiles;
use editor_view::{
    Attrs, BlockWrapper, BlockWrapperSpec, Decoration, DecorationSet, LineSpec, MarkSpec,
    ReplaceSpec, WidgetSpec, WidgetType,
};

fn doc_text(doc: &str) -> Text {
    Text::of(&doc.split('\n').collect::<Vec<_>>()).unwrap()
}

struct Ed {
    document: Document,
    content: Element,
    doc: Text,
    decos: Vec<DecorationSet>,
    wrappers: Vec<RangeSet<BlockWrapper>>,
    // How many of the decoration sets, from the first, are upstream's test
    // fields, which the `filterDeco` and `addDeco` effects change.
    fields: usize,
    view: Option<ContentTiles>,
}

fn temp_view_with(
    doc: &str,
    decos: Vec<DecorationSet>,
    wrappers: Vec<RangeSet<BlockWrapper>>,
) -> Ed {
    let document = Document::new(800, 600).unwrap();
    let content = Element::create(&document, "div").unwrap();
    document.body().append_child(&content).unwrap();
    let doc = doc_text(doc);
    let view = ContentTiles::new(
        &document,
        content.clone(),
        &doc,
        decos.clone(),
        wrappers.clone(),
    )
    .unwrap();
    Ed {
        document,
        content,
        doc,
        fields: decos.len(),
        decos,
        wrappers,
        view: Some(view),
    }
}

fn temp_view(doc: &str, decos: Vec<DecorationSet>) -> Ed {
    temp_view_with(doc, decos, Vec::new())
}

fn deco_editor(doc: &str, ranges: Vec<Range<Decoration>>) -> Ed {
    temp_view(doc, vec![Decoration::set(ranges, true)])
}

type Filter<'a> = &'a mut dyn FnMut(usize, usize, &Arc<Decoration>) -> bool;

impl Ed {
    fn view(&self) -> &ContentTiles {
        self.view.as_ref().unwrap()
    }

    // `cm.dispatch`: apply `changes`, map the decorations through them as
    // upstream's test field does, then let `effects` change them.
    fn dispatch_with(
        &mut self,
        changes: Option<ChangeSpec>,
        effects: impl FnOnce(&mut Vec<DecorationSet>, &mut Vec<RangeSet<BlockWrapper>>),
    ) {
        let set = match changes {
            Some(changes) => ChangeSet::of(changes, self.doc.len(), None).unwrap(),
            None => ChangeSet::empty(self.doc.len()),
        };
        self.doc = set.apply(&self.doc);
        self.decos = self.decos.iter().map(|d| d.map(set.desc())).collect();
        self.wrappers = self.wrappers.iter().map(|w| w.map(set.desc())).collect();
        effects(&mut self.decos, &mut self.wrappers);
        let (decos, wrappers) = (self.decos.clone(), self.wrappers.clone());
        self.view
            .as_mut()
            .unwrap()
            .update(&self.doc, &set, decos, wrappers)
            .unwrap();
    }

    fn change(&mut self, changes: ChangeSpec) {
        self.dispatch_with(Some(changes), |_, _| {});
    }

    // The test fields' `filterDeco` and `addDeco` effects.
    fn deco_effects(
        &mut self,
        changes: Option<ChangeSpec>,
        mut filter: Option<Filter<'_>>,
        add: Vec<Range<Decoration>>,
    ) {
        let fields = self.fields;
        self.dispatch_with(changes, |decos, _| {
            for set in &mut decos[..fields] {
                if let Some(filter) = &mut filter {
                    *set = set.update(RangeSetUpdate {
                        filter: Some(&mut **filter),
                        ..Default::default()
                    });
                }
                if !add.is_empty() {
                    *set = set.update(RangeSetUpdate {
                        add: add.clone(),
                        sort: true,
                        ..Default::default()
                    });
                }
            }
        });
    }

    fn filter_all(&mut self) {
        self.deco_effects(None, Some(&mut |_, _, _| false), Vec::new());
    }

    fn add(&mut self, add: Vec<Range<Decoration>>) {
        self.deco_effects(None, None, add);
    }

    fn replace_all(&mut self, add: Vec<Range<Decoration>>) {
        self.deco_effects(None, Some(&mut |_, _, _| false), add);
    }

    fn blocks(&self) -> Vec<String> {
        self.view().block_html().unwrap()
    }

    // `contentDOM.innerHTML`.
    fn html(&self) -> String {
        self.blocks().concat()
    }

    // `childNodes[i].innerHTML` of a line.
    fn inner(&self, i: usize) -> String {
        let block = &self.blocks()[i];
        let open = block.find('>').unwrap() + 1;
        let close = block.rfind("</").unwrap();
        block[open..close].to_string()
    }

    fn count(&self, needle: &str) -> usize {
        self.html().matches(needle).count()
    }

    // Upstream's `domText`: the text of the lines, joined by newlines,
    // leaving out `<br>` and uneditable elements.
    fn dom_text(&self) -> String {
        self.blocks()
            .iter()
            .map(|block| strip_tags(block))
            .collect::<Vec<_>>()
            .join("\n")
    }

    // `textContent` of the content.
    fn text(&self) -> String {
        self.content.text_content().unwrap()
    }

    fn lines(&self) -> Vec<Element> {
        let mut lines = Vec::new();
        let mut next = self.content.first_child().unwrap();
        while let Some(line) = next {
            next = line.next_sibling().unwrap();
            lines.push(line);
        }
        lines
    }

    fn with_class(&self, class: &str) -> Vec<Element> {
        self.document.elements_with_class(class).unwrap()
    }

    fn contains(&self, element: &Element) -> bool {
        self.content.contains(element).unwrap()
    }

    fn describe(&self) -> String {
        self.view().describe()
    }

    fn destroy(&mut self) {
        self.view.take().unwrap().destroy().unwrap();
    }
}

fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    let mut hidden = 0usize;
    let mut depth = 0usize;
    while let Some(start) = rest.find('<') {
        if hidden == 0 {
            out.push_str(&rest[..start]);
        }
        let end = rest[start..].find('>').unwrap() + start;
        let tag = &rest[start + 1..end];
        if let Some(_close) = tag.strip_prefix('/') {
            if hidden > 0 && depth == hidden {
                hidden = 0;
            }
            depth -= 1;
        } else if !(tag.starts_with("img") || tag.starts_with("br")) {
            depth += 1;
            if hidden == 0 && tag.contains("contenteditable=\"false\"") {
                hidden = depth;
            }
        }
        rest = &rest[end + 1..];
    }
    if hidden == 0 {
        out.push_str(rest);
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", "\u{a0}")
        .replace("&amp;", "&")
}

fn attrs(pairs: &[(&str, &str)]) -> Attrs {
    pairs.iter().copied().collect()
}

fn mark_spec(class: Option<&str>) -> MarkSpec {
    MarkSpec {
        class: class.map(Into::into),
        ..Default::default()
    }
}

// `d(from, to, {class})`.
fn d(from: usize, to: usize, class: &str) -> Range<Decoration> {
    Decoration::mark(mark_spec(Some(class)))
        .range(from, to)
        .unwrap()
}

fn dm(from: usize, to: usize, spec: MarkSpec) -> Range<Decoration> {
    Decoration::mark(spec).range(from, to).unwrap()
}

// `l(pos, class)`.
fn l(pos: usize, class: &str) -> Range<Decoration> {
    Decoration::line(LineSpec {
        attributes: Some(attrs(&[("class", class)])),
        class: None,
        extra: None,
    })
    .range(pos, pos)
    .unwrap()
}

// `r(from, to, spec)`.
fn r(from: usize, to: usize) -> Range<Decoration> {
    Decoration::replace(ReplaceSpec::default())
        .range(from, to)
        .unwrap()
}

fn rw(from: usize, to: usize, word: &str) -> Range<Decoration> {
    Decoration::replace(ReplaceSpec {
        widget: Some(editor_view::Widget::new(WordWidget::new(word))),
        ..Default::default()
    })
    .range(from, to)
    .unwrap()
}

type Log = Arc<Mutex<Vec<String>>>;

// Upstream's `WordWidget`: a `strong` element holding the word. It has a
// `word` class here, since Div reads `strong` as `span`.
struct WordWidget {
    word: String,
    destroyed: Option<Log>,
}

impl WordWidget {
    fn new(word: &str) -> WordWidget {
        WordWidget {
            word: word.into(),
            destroyed: None,
        }
    }

    fn logged(word: &str, log: &Log) -> WordWidget {
        WordWidget {
            word: word.into(),
            destroyed: Some(log.clone()),
        }
    }
}

impl WidgetType for WordWidget {
    fn to_dom(&self, document: &Document) -> Result<Element, div::Error> {
        let dom = Element::create(document, "strong")?;
        dom.set_attribute("class", "word")?;
        dom.set_text(&self.word)?;
        Ok(dom)
    }

    fn eq(&self, other: &Self) -> bool {
        self.word.to_lowercase() == other.word.to_lowercase()
    }

    fn destroy(&self, _dom: &Element) {
        if let Some(log) = &self.destroyed {
            log.lock().unwrap().push(self.word.clone());
        }
    }
}

struct OtherWidget;

impl WidgetType for OtherWidget {
    fn to_dom(&self, document: &Document) -> Result<Element, div::Error> {
        Element::create(document, "img")
    }
}

fn widget_deco(widget: impl WidgetType, side: i32) -> Arc<Decoration> {
    Decoration::widget(WidgetSpec {
        side,
        ..WidgetSpec::new(widget)
    })
}

// `w(pos, widget, side)`.
fn w(pos: usize, word: &str, side: i32) -> Range<Decoration> {
    widget_deco(WordWidget::new(word), side)
        .range(pos, pos)
        .unwrap()
}

// A deterministic stand-in for `Math.random`.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() * n as f64).floor() as usize
    }
}

// webtest-draw.ts

#[test]
fn follows_updates_to_the_document() {
    let mut cm = temp_view("one\ntwo", Vec::new());
    assert_eq!(cm.dom_text(), "one\ntwo");
    cm.change(ChangeSpec::replace(1, 2, "x"));
    assert_eq!(cm.dom_text(), "oxe\ntwo");
    cm.change(ChangeSpec::replace(2, 5, "1\n2\n3"));
    assert_eq!(cm.dom_text(), "ox1\n2\n3wo");
    cm.change(ChangeSpec::delete(1, 8));
    assert_eq!(cm.dom_text(), "oo");
}

#[test]
fn works_in_multiple_lines() {
    let mut doc = "abcdefghijklmnopqrstuvwxyz\n".repeat(10);
    let mut cm = temp_view("", Vec::new());
    cm.change(ChangeSpec::insert(0, doc.clone()));
    assert_eq!(cm.dom_text(), doc);
    cm.change(ChangeSpec::insert(0, "/"));
    doc = format!("/{doc}");
    assert_eq!(cm.dom_text(), doc);
    cm.change(ChangeSpec::replace(100, 104, "$"));
    doc = format!("{}${}", &doc[..100], &doc[104..]);
    assert_eq!(cm.dom_text(), doc);
    cm.change(ChangeSpec::delete(200, 268));
    doc.truncate(200);
    assert_eq!(cm.dom_text(), doc);
}

#[test]
fn can_split_a_line() {
    let mut cm = temp_view("abc\ndef\nghi", Vec::new());
    cm.change(ChangeSpec::insert(4, "xyz\nk"));
    assert_eq!(cm.dom_text(), "abc\nxyz\nkdef\nghi");
}

#[test]
fn redraws_lazily() {
    let mut cm = temp_view("one\ntwo\nthree", Vec::new());
    let lines = cm.lines();
    cm.change(ChangeSpec::insert(5, "x"));
    // Upstream also checks that the first and last lines keep their text
    // nodes (div-gap 24).
    for line in &lines {
        assert!(cm.contains(line));
    }
    assert_eq!(cm.dom_text(), "one\ntxwo\nthree");
}

#[test]
fn notices_the_doc_needs_to_be_redrawn_when_only_inserting_empty_lines() {
    let mut cm = temp_view("", Vec::new());
    cm.change(ChangeSpec::insert(0, "\n\n\n"));
    assert_eq!(cm.dom_text(), "\n\n\n");
}

#[test]
fn draws_br_nodes_on_empty_lines() {
    let mut cm = temp_view("one\n\ntwo", Vec::new());
    assert_eq!(cm.blocks()[1], "<div class=\"cm-line\"><br></div>");
    cm.change(ChangeSpec::insert(4, "x"));
    assert_eq!(cm.blocks()[1], "<div class=\"cm-line\">x</div>");
}

#[test]
fn can_handle_replace_all_like_events() {
    let mut rng = Rng(0x5eed_0001);
    let chars: Vec<char> = "abcdefghijklmn    \n".chars().collect();
    let content: String = (0..5000).map(|_| chars[rng.below(chars.len())]).collect();
    let mut cm = temp_view(&content, Vec::new());
    let mut changes = Vec::new();
    let mut last = usize::MAX;
    for _ in (0..=content.len() / 100).rev() {
        let from = rng.below(cm.doc.len() - 10);
        let to = from + rng.below(10);
        // Upstream lets `ChangeSet.of` sort overlapping ranges; keep them
        // apart so the set is valid.
        if last != usize::MAX && to + 1 >= last {
            continue;
        }
        last = from;
        changes.push(ChangeSpec::replace(from, to, "XYZ"));
    }
    changes.reverse();
    cm.change(ChangeSpec::List(changes));
    assert_eq!(cm.dom_text(), cm.doc.to_string());
}

#[test]
fn can_replace_across_line_boundaries() {
    let mut cm = temp_view("ab\ncd\nef", Vec::new());
    cm.change(ChangeSpec::replace(1, 4, "XYZ"));
    assert_eq!(cm.dom_text(), cm.doc.to_string());
}

#[test]
fn can_handle_deleting_a_lines_content() {
    let mut cm = temp_view("foo\nbaz", Vec::new());
    cm.change(ChangeSpec::delete(4, 7));
    assert_eq!(cm.dom_text(), "foo\n");
}

#[test]
fn can_insert_blank_lines_at_the_end_of_the_document() {
    let mut cm = temp_view("foo", Vec::new());
    cm.change(ChangeSpec::insert(3, "\n\nx"));
    assert_eq!(cm.dom_text(), "foo\n\nx");
}

#[test]
fn can_handle_deleting_the_end_of_a_line() {
    let mut cm = temp_view("a\nbc\n", Vec::new());
    cm.change(ChangeSpec::delete(3, 4));
    cm.change(ChangeSpec::insert(3, "d"));
    assert_eq!(cm.dom_text(), "a\nbd\n");
}

#[test]
fn correctly_handles_very_complicated_transactions() {
    let mut rng = Rng(0x5eed_0002);
    let chars: Vec<char> = "abcdef  \n".chars().collect();
    let mut cm = temp_view("foo\nbar\nbaz", Vec::new());
    for _ in 0..200 {
        let doc_len = cm.doc.len();
        let pos = doc_len.min(20);
        let choice = rng.next();
        let r = rng.next();
        let change = if choice < 0.15 {
            None
        } else if choice < 0.5 {
            let from = pos.saturating_sub((r * 2.0).floor() as usize);
            let to = doc_len.min(pos + (r * 4.0).floor() as usize);
            Some(ChangeSpec::delete(from, to))
        } else {
            let count = (r * 6.0).floor() as usize + 1;
            let text: String = (0..count).map(|_| chars[rng.below(chars.len())]).collect();
            Some(ChangeSpec::insert(pos, text))
        };
        cm.dispatch_with(change, |_, _| {});
        assert_eq!(cm.dom_text(), cm.doc.to_string());
    }
}

#[test]
fn reuses_line_nodes() {
    let mut cm = temp_view("ab\ncd", Vec::new());
    let lines = cm.lines();
    let (line1, line2) = (&lines[0], &lines[1]);
    cm.change(ChangeSpec::List(vec![
        ChangeSpec::insert(0, "!"),
        ChangeSpec::insert(5, "?"),
    ]));
    assert!(cm.lines()[0].is_same_node(line1).unwrap());
    assert!(cm.lines()[1].is_same_node(line2).unwrap());
    cm.change(ChangeSpec::delete(0, 3));
    assert!(cm.lines()[0].is_same_node(line1).unwrap());
    cm.change(ChangeSpec::delete(0, 1));
    assert!(cm.lines()[0].is_same_node(line1).unwrap());
}

#[test]
fn splits_long_stretches_of_text_into_chunks() {
    let cm = temp_view(
        &"a".repeat(800),
        vec![Decoration::set(vec![d(0, 800, "x")], false)],
    );
    assert_eq!(cm.count("<span"), 1);
    let expect = format!(
        "DocTile(LineTile(MarkTile(\"{}\",\"{}\")))",
        "a".repeat(512),
        "a".repeat(288)
    );
    assert_eq!(cm.describe(), expect);
}

// webtest-draw-decoration.ts

#[test]
fn renders_attributes() {
    let cm = deco_editor(
        "foo bar",
        vec![
            dm(
                0,
                3,
                MarkSpec {
                    attributes: Some(attrs(&[("title", "t")])),
                    ..Default::default()
                },
            ),
            dm(
                4,
                7,
                MarkSpec {
                    attributes: Some(attrs(&[("lang", "nl")])),
                    ..Default::default()
                },
            ),
        ],
    );
    assert_eq!(cm.count("title=\"t\""), 1);
    assert_eq!(cm.count("lang=\"nl\""), 1);
}

#[test]
fn updates_for_added_decorations() {
    let mut cm = deco_editor("hello\ngoodbye", Vec::new());
    cm.add(vec![d(2, 8, "c")]);
    assert_eq!(
        cm.html(),
        "<div class=\"cm-line\">he<span class=\"c\">llo</span></div>\
         <div class=\"cm-line\"><span class=\"c\">go</span>odbye</div>"
    );
}

#[test]
fn updates_for_removed_decorations() {
    let strong = MarkSpec {
        tag_name: Some("strong".into()),
        ..Default::default()
    };
    let mut cm = deco_editor("one\ntwo\nthree", vec![d(1, 12, "x"), dm(4, 7, strong)]);
    cm.deco_effects(None, Some(&mut |from, _, _| from == 4), Vec::new());
    assert_eq!(cm.count("class=\"x\""), 0);
    assert_eq!(cm.inner(1), "<span>two</span>");
}

#[test]
fn doesnt_update_dom_that_doesnt_need_to_change() {
    let em = MarkSpec {
        tag_name: Some("em".into()),
        ..Default::default()
    };
    let mut cm = deco_editor("one\ntwo", vec![dm(0, 3, em)]);
    let second = cm.lines()[1].clone();
    cm.filter_all();
    assert!(cm.lines()[1].is_same_node(&second).unwrap());
}

#[test]
fn nests_decoration_elements() {
    let cm = temp_view(
        "abcdef",
        vec![
            Decoration::set(vec![d(2, 6, "b")], false),
            Decoration::set(vec![d(0, 4, "a")], false),
        ],
    );
    assert_eq!(
        cm.inner(0),
        "<span class=\"a\">ab<span class=\"b\">cd</span></span><span class=\"b\">ef</span>"
    );
}

#[test]
fn drops_entirely_deleted_decorations() {
    let strong = MarkSpec {
        inclusive_start: Some(true),
        inclusive_end: Some(true),
        class: Some("strong".into()),
        ..Default::default()
    };
    let mut cm = deco_editor("abc", vec![dm(1, 2, strong)]);
    cm.change(ChangeSpec::replace(0, 3, "a"));
    assert_eq!(cm.count("strong"), 0);
}

#[test]
fn doesnt_merge_separate_decorations() {
    let mut cm = deco_editor("abcd", vec![d(0, 2, "a"), d(2, 4, "a")]);
    assert_eq!(cm.count("class=\"a\""), 2);
    cm.change(ChangeSpec::delete(1, 3));
    assert_eq!(cm.count("class=\"a\""), 2);
}

#[test]
fn merges_joined_decorations() {
    let mut cm = deco_editor("ab cd", vec![d(0, 2, "a"), d(3, 5, "a")]);
    cm.deco_effects(
        Some(ChangeSpec::replace(2, 3, "x")),
        Some(&mut |_, _, _| false),
        vec![d(0, 5, "a")],
    );
    assert_eq!(cm.count("class=\"a\""), 1);
}

#[test]
fn properly_joins_decorations_when_partially_reusing_them() {
    let mk_deco = |doc: &Text| {
        let mut deco = vec![d(0, doc.len(), "w")];
        let mut i = 0;
        while i + 1 < doc.len() {
            deco.push(d(i, i + 1, "l"));
            i += 2;
        }
        Decoration::set(deco, false)
    };
    let mut cm = temp_view("Start", vec![mk_deco(&doc_text("Start"))]);
    cm.dispatch_with(Some(ChangeSpec::insert(3, "x")), |decos, _| {
        decos[0] = mk_deco(&doc_text("Staxrt"));
    });
    assert_eq!(cm.count("class=\"w\""), 1);
}

#[test]
fn keeps_identical_but_separate_decorations_separate() {
    let m = Decoration::mark(MarkSpec {
        tag_name: Some("em".into()),
        ..Default::default()
    });
    let mut cm = deco_editor(
        "aaabbb",
        vec![m.range(0, 3).unwrap(), m.range(3, 6).unwrap()],
    );
    cm.change(ChangeSpec::insert(6, "!"));
    // `em`, read by kind (div-gap 25).
    assert_eq!(cm.inner(0), "<span>aaa</span><span>bbb</span>!");
}

#[test]
fn merges_stacked_decorations() {
    let mut cm = temp_view(
        "one",
        vec![
            Decoration::set(Vec::new(), true),
            Decoration::set(vec![d(0, 3, "a")], false),
        ],
    );
    cm.fields = 1;
    cm.add(vec![d(1, 2, "b")]);
    assert_eq!(cm.count("class=\"a\""), 1);
}

#[test]
fn keeps_decorations_together_when_deleting_inside_of_them() {
    let mut cm = deco_editor("one\ntwo", vec![d(1, 6, "a")]);
    assert_eq!(cm.count("class=\"a\""), 2);
    cm.change(ChangeSpec::delete(2, 5));
    assert_eq!(cm.count("class=\"a\""), 1);
}

#[test]
fn does_merge_recreated_decorations() {
    let mut cm = deco_editor("abcde", vec![d(1, 4, "c")]);
    cm.deco_effects(
        Some(ChangeSpec::replace(2, 5, "CDE")),
        Some(&mut |_, _, _| false),
        vec![d(1, 4, "c")],
    );
    assert_eq!(cm.inner(0), "a<span class=\"c\">bCD</span>E");
}

#[test]
fn breaks_high_precedence_ranges_for_low_precedence_wrappers() {
    let cm = temp_view(
        "abc",
        vec![
            Decoration::set(vec![d(1, 3, "b")], false),
            Decoration::set(vec![d(0, 2, "a")], false),
        ],
    );
    assert_eq!(
        cm.inner(0),
        "<span class=\"a\">a<span class=\"b\">b</span></span><span class=\"b\">c</span>"
    );
}

#[test]
fn reuses_mark_nodes() {
    let strong = |inclusive| MarkSpec {
        class: Some("strong".into()),
        inclusive: Some(inclusive),
        ..Default::default()
    };
    let mut cm = deco_editor(
        "one two three",
        vec![dm(0, 3, strong(false)), dm(8, 13, strong(true))],
    );
    let marks = cm.with_class("strong");
    assert_eq!(marks.len(), 2);
    cm.change(ChangeSpec::List(vec![
        ChangeSpec::delete(0, 1),
        ChangeSpec::insert(8, "!"),
    ]));
    assert!(marks.iter().all(|m| cm.contains(m)));
    cm.change(ChangeSpec::List(vec![
        ChangeSpec::delete(1, 3),
        ChangeSpec::replace(6, 7, "-"),
    ]));
    assert!(marks.iter().all(|m| cm.contains(m)));
}

#[test]
fn properly_handles_random_decorations_and_changes() {
    let mut rng = Rng(0x5eed_0003);
    let names = ["a", "b", "c"];
    let marks: Vec<_> = names
        .iter()
        .map(|name| Decoration::mark(mark_spec(Some(name))))
        .collect();
    let mut doc = String::from("abcd efgh ijkl mnopq rstu vwxy z");
    let mut cm = deco_editor(&doc, Vec::new());
    for _ in 0..50 {
        let mut change = None;
        if rng.below(5) < 3 {
            let from = rng.below(doc.len().saturating_sub(3));
            let to = from + rng.below(3.min(doc.len() - from));
            let insert = "#".repeat(rng.below(4));
            doc = format!("{}{insert}{}", &doc[..from], &doc[to..]);
            change = Some(ChangeSpec::replace(from, to, insert));
        }
        let mut deco: Vec<(usize, usize, usize)> = Vec::new();
        let count = rng.below(marks.len());
        for j in 0..count {
            let from = rng.below(doc.len() - 3);
            let to = from + 1 + rng.below(doc.len() - 1 - from);
            if !deco.iter().any(|&(f, t, _)| f == from && t == to) {
                deco.push((from, to, j));
            }
        }
        deco.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
        let ranges = deco
            .iter()
            .map(|&(from, to, j)| marks[j].range(from, to).unwrap())
            .collect();
        cm.deco_effects(change, Some(&mut |_, _, _| false), ranges);

        let mut expect = String::new();
        let mut pos = 0;
        let mut j = 0;
        let mut active: Vec<(usize, usize, usize)> = Vec::new();
        loop {
            let next = deco.get(j).copied();
            let next_stop = active.iter().map(|m| m.1).min().unwrap_or(1_000_000_000);
            let next_pos = next_stop.min(next.map_or(doc.len(), |n| n.0));
            if next_pos > pos {
                expect.push_str(&doc[pos..next_pos]);
                pos = next_pos;
            }
            let mut reopen = Vec::new();
            if next_stop <= pos || next.is_some_and(|n| active.iter().any(|a| a.1 < n.1)) {
                let close_to = active
                    .iter()
                    .position(|m| m.1 == pos || next.is_some_and(|n| m.1 < n.1))
                    .unwrap();
                while active.len() > close_to {
                    let close = active.pop().unwrap();
                    expect.push_str("</span>");
                    if close.1 > pos {
                        reopen.insert(0, close);
                    }
                }
            }
            if let Some(n) = next.filter(|n| n.0 == pos) {
                j += 1;
                expect.push_str(&format!("<span class=\"{}\">", names[n.2]));
                active.push(n);
            }
            for m in reopen {
                expect.push_str(&format!("<span class=\"{}\">", names[m.2]));
                active.push(m);
            }
            if pos == doc.len() {
                break;
            }
        }
        assert_eq!(cm.inner(0), expect);
    }
}

// widget

#[test]
fn draws_widgets() {
    let cm = deco_editor("hello", vec![w(4, "hi", 0)]);
    let elt = &cm.with_class("word")[0];
    assert_eq!(elt.text_content().unwrap(), "hi");
    assert_eq!(
        elt.get_attribute("contenteditable").unwrap().as_deref(),
        Some("false")
    );
    assert_eq!(cm.text(), "hellhio");
}

#[test]
fn compares_widgets_with_their_eq_method() {
    let mut cm = deco_editor("hello", vec![w(4, "hi", 0)]);
    let elt = cm.with_class("word")[0].clone();
    cm.replace_all(vec![w(4, "HI", 0)]);
    assert!(cm.with_class("word")[0].is_same_node(&elt).unwrap());
}

#[test]
fn notices_replaced_replacement_decorations() {
    let mut cm = deco_editor("abc", vec![rw(1, 2, "X")]);
    cm.replace_all(vec![rw(1, 2, "Y")]);
    assert_eq!(cm.text(), "aYc");
}

#[test]
fn allows_replacements_to_shadow_inner_replacements() {
    let mut cm = deco_editor("one\ntwo\nthree\nfour", vec![rw(5, 12, "INNER")]);
    cm.add(vec![rw(1, 17, "OUTER")]);
    assert_eq!(cm.text(), "oOUTERr");
}

#[test]
fn doesnt_consider_different_widget_types_equivalent() {
    let mut cm = deco_editor("hello", vec![w(4, "hi", 0)]);
    assert_eq!(cm.with_class("word").len(), 1);
    cm.replace_all(vec![widget_deco(OtherWidget, 0).range(4, 4).unwrap()]);
    assert!(cm.with_class("word").is_empty());
    assert_eq!(
        cm.inner(0),
        "hell<img contenteditable=\"false\"><img class=\"cm-widgetBuffer\" aria-hidden=\"true\">o"
    );
}

#[test]
fn orders_widgets_by_side() {
    let cm = deco_editor("hello", vec![w(4, "A", -1), w(4, "B", 0), w(4, "C", 10)]);
    let words: Vec<String> = cm
        .with_class("word")
        .iter()
        .map(|e| e.text_content().unwrap())
        .collect();
    assert_eq!(words, ["A", "B", "C"]);
}

#[test]
fn reuses_widgets_on_empty_lines() {
    let mut cm = deco_editor("a\n\nb", vec![w(2, "x", 0)]);
    let dom = cm.with_class("word")[0].clone();
    cm.replace_all(vec![w(2, "x", 0)]);
    assert!(cm.with_class("word")[0].is_same_node(&dom).unwrap());
}

#[test]
fn can_update_widgets_in_an_empty_document() {
    let mut cm = deco_editor("", vec![w(0, "A", 0)]);
    cm.add(vec![w(0, "B", 0)]);
    assert_eq!(cm.with_class("word").len(), 2);
}

#[test]
fn doesnt_duplicate_widgets_on_line_splitting() {
    let mut cm = deco_editor("a", vec![w(1, "W", 1)]);
    cm.change(ChangeSpec::insert(1, "\n"));
    assert_eq!(cm.with_class("word").len(), 1);
}

#[test]
fn can_remove_widgets_at_the_end_of_a_line() {
    let mut cm = deco_editor("one\ntwo", vec![w(3, "A", 0)]);
    cm.replace_all(vec![w(5, "B", 0)]);
    assert_eq!(cm.with_class("word").len(), 1);
}

#[test]
fn can_wrap_widgets_in_marks() {
    let mut cm = temp_view(
        "abcd",
        vec![
            Decoration::set(vec![d(1, 3, "b")], false),
            Decoration::set(vec![w(2, "hi", 0)], false),
            Decoration::set(vec![d(0, 4, "a")], false),
        ],
    );
    let a = cm.with_class("a");
    let b = cm.with_class("b");
    let word = cm.with_class("word");
    assert_eq!((a.len(), b.len(), word.len()), (1, 2, 1));
    assert!(word[0]
        .parent()
        .unwrap()
        .unwrap()
        .is_same_node(&a[0])
        .unwrap());
    assert!(b[0].parent().unwrap().unwrap().is_same_node(&a[0]).unwrap());
    assert_eq!(b[0].text_content().unwrap(), "b");
    assert_eq!(b[1].text_content().unwrap(), "c");
    cm.deco_effects(None, Some(&mut |from, _, _| from != 2), Vec::new());
    assert_eq!(cm.count("class=\"b\""), 1);
}

#[test]
fn includes_negative_side_widgets_in_marks_that_end_at_their_position() {
    let cm = temp_view(
        "123",
        vec![
            Decoration::set(vec![w(2, "x", -1)], false),
            Decoration::set(
                vec![dm(
                    0,
                    2,
                    MarkSpec {
                        class: Some("em".into()),
                        inclusive: Some(true),
                        ..Default::default()
                    },
                )],
                false,
            ),
        ],
    );
    assert_eq!(cm.with_class("em")[0].text_content().unwrap(), "12x");
}

#[test]
fn includes_positive_side_widgets_in_marks_that_start_at_their_position() {
    let cm = temp_view(
        "123",
        vec![
            Decoration::set(vec![w(1, "x", 1)], false),
            Decoration::set(
                vec![dm(
                    1,
                    3,
                    MarkSpec {
                        class: Some("em".into()),
                        inclusive: Some(true),
                        ..Default::default()
                    },
                )],
                false,
            ),
        ],
    );
    assert_eq!(cm.with_class("em")[0].text_content().unwrap(), "x23");
}

#[test]
fn merges_text_around_a_removed_widget() {
    let mut cm = temp_view("1234", vec![Decoration::set(vec![w(2, "x", 0)], false)]);
    cm.filter_all();
    assert_eq!(cm.describe(), "DocTile(LineTile(\"1234\"))");
    assert_eq!(cm.inner(0), "1234");
}

#[test]
fn draws_buffers_around_widgets() {
    let cm = temp_view(
        "1234",
        vec![Decoration::set(vec![w(1, "x", 1), w(3, "y", -1)], false)],
    );
    let html = cm
        .inner(0)
        .replace("<img class=\"cm-widgetBuffer\" aria-hidden=\"true\">", "#");
    assert_eq!(
        strip_tags(&html.replace("contenteditable=\"false\"", "")),
        "1#x23y#4"
    );
}

#[test]
fn doesnt_draw_unnecessary_buffers_between_adjacent_widgets() {
    let cm = temp_view(
        "1234",
        vec![Decoration::set(
            vec![w(1, "x", 1), w(1, "x", 1), w(3, "x", -1), w(3, "x", -1)],
            false,
        )],
    );
    let html = cm
        .inner(0)
        .replace("<img class=\"cm-widgetBuffer\" aria-hidden=\"true\">", "#");
    assert_eq!(
        strip_tags(&html.replace("contenteditable=\"false\"", "")),
        "1#xx23xx#4"
    );
}

#[test]
fn doesnt_wrap_buffers_at_the_start_of_a_mark_in_the_mark() {
    let cm = temp_view(
        "abc",
        vec![Decoration::set(vec![w(1, "x", 0), d(1, 2, "m")], false)],
    );
    assert_eq!(cm.count("class=\"m\""), 1);
}

#[test]
fn puts_a_buffer_in_front_of_widgets_spanned_by_marks() {
    let cm = temp_view(
        "a\n\nc",
        vec![
            Decoration::set(vec![d(0, 4, "m")], false),
            Decoration::set(vec![w(2, "Q", 1)], false),
        ],
    );
    assert_eq!(cm.count("cm-widgetBuffer"), 1);
}

#[test]
fn reuses_buffers_on_redraw() {
    let mut cm = deco_editor("abcd", vec![r(1, 2)]);
    let buffers = cm.with_class("cm-widgetBuffer");
    assert_eq!(buffers.len(), 2);
    cm.change(ChangeSpec::insert(1, ".."));
    assert!(buffers.iter().all(|b| cm.contains(b)));
}

#[test]
fn calls_the_destroy_method_on_destroyed_widgets() {
    let log: Log = Default::default();
    let w1 = widget_deco(WordWidget::logged("A", &log), 0);
    let w2 = widget_deco(WordWidget::logged("B", &log), 0);
    let mut cm = temp_view(
        "abcde",
        vec![Decoration::set(
            vec![
                w1.range(1, 1).unwrap(),
                w2.range(2, 2).unwrap(),
                w2.range(4, 4).unwrap(),
            ],
            false,
        )],
    );
    cm.change(ChangeSpec::delete(0, 3));
    let sorted = |log: &Log| {
        let mut v = log.lock().unwrap().clone();
        v.sort();
        v.join(",")
    };
    assert_eq!(sorted(&log), "A,B");
    cm.change(ChangeSpec::delete(0, 2));
    assert_eq!(sorted(&log), "A,B,B");
}

#[test]
fn calls_the_destroy_method_widgets_when_the_editor_is_destroyed() {
    let log: Log = Default::default();
    let mut cm = temp_view(
        "abcde",
        vec![Decoration::set(
            vec![
                widget_deco(WordWidget::logged("A", &log), 0)
                    .range(1, 1)
                    .unwrap(),
                widget_deco(WordWidget::logged("B", &log), 0)
                    .range(2, 2)
                    .unwrap(),
            ],
            false,
        )],
    );
    cm.destroy();
    let mut destroyed = log.lock().unwrap().clone();
    destroyed.sort();
    assert_eq!(destroyed.join(","), "A,B");
    assert!(cm.content.first_child().unwrap().is_none());
}

#[test]
fn calls_destroy_on_updated_widgets() {
    let log: Log = Default::default();
    let mut cm = temp_view(
        "abcde",
        vec![Decoration::set(
            vec![widget_deco(WordWidget::logged("A", &log), 0)
                .range(1, 1)
                .unwrap()],
            false,
        )],
    );
    cm.replace_all(vec![widget_deco(WordWidget::logged("B", &log), 0)
        .range(1, 1)
        .unwrap()]);
    assert_eq!(log.lock().unwrap().join(","), "A");
}

// replaced

#[test]
fn omits_replaced_content() {
    let cm = deco_editor("foobar", vec![r(1, 4)]);
    assert_eq!(cm.text(), "far");
}

#[test]
fn can_replace_across_lines() {
    let cm = deco_editor("foo\nbar\nbaz\nbug", vec![r(1, 14)]);
    assert_eq!(cm.blocks().len(), 1);
    assert_eq!(cm.text(), "fg");
}

#[test]
fn draws_replacement_widgets() {
    let cm = deco_editor("foo\nbar\nbaz", vec![rw(6, 9, "X")]);
    assert_eq!(cm.text(), "foobaXaz");
}

#[test]
fn can_handle_multiple_overlapping_replaced_ranges() {
    let cm = deco_editor("foo\nbar\nbaz\nbug", vec![r(1, 6), r(6, 9), r(8, 14)]);
    assert_eq!(cm.blocks().len(), 1);
    assert_eq!(cm.text(), "fg");
}

#[test]
fn allows_splitting_a_replaced_range() {
    let mut cm = deco_editor("1234567890", vec![r(1, 9)]);
    cm.deco_effects(
        Some(ChangeSpec::replace(2, 8, "abcdef")),
        Some(&mut |_, _, _| false),
        vec![r(1, 3), r(7, 9)],
    );
    assert_eq!(cm.text(), "1bcde0");
}

#[test]
fn allows_replacing_a_single_replaced_range_with_two_adjacent_ones() {
    let mut cm = deco_editor("1234567890", vec![r(1, 9)]);
    cm.deco_effects(
        Some(ChangeSpec::replace(2, 8, "cdefgh")),
        Some(&mut |_, _, _| false),
        vec![r(1, 5), r(5, 9)],
    );
    assert_eq!(cm.text(), "10");
    assert_eq!(cm.inner(0).matches("<span").count(), 2);
}

#[test]
fn can_handle_changes_inside_replaced_content() {
    let mut cm = deco_editor("abcdefghij", vec![r(2, 8)]);
    cm.change(ChangeSpec::replace(4, 6, "n"));
    assert_eq!(cm.text(), "abij");
}

// line attributes

fn classes(cm: &Ed, lines: &[&str]) {
    let lines_dom = cm.lines();
    for (i, expect) in lines.iter().enumerate() {
        let class = lines_dom[i]
            .get_attribute("class")
            .unwrap()
            .unwrap_or_default();
        let mut names: Vec<&str> = class
            .split(' ')
            .filter(|c| *c != "cm-line" && !c.is_empty())
            .collect();
        names.sort();
        assert_eq!(names.join(" "), *expect, "line {i}");
    }
}

#[test]
fn adds_line_attributes() {
    let cm = deco_editor(
        "abc\ndef\nghi",
        vec![l(0, "a"), l(0, "b"), l(1, "c"), l(8, "d")],
    );
    classes(&cm, &["a b", "", "d"]);
}

#[test]
fn updates_when_line_attributes_are_added() {
    let mut cm = deco_editor("foo\nbar", vec![l(0, "a")]);
    let lines = cm.lines();
    cm.add(vec![l(0, "b"), l(4, "c")]);
    classes(&cm, &["a b", "c"]);
    assert!(cm.lines()[0].is_same_node(&lines[0]).unwrap());
    assert!(cm.lines()[1].is_same_node(&lines[1]).unwrap());
}

#[test]
fn updates_when_line_attributes_are_removed() {
    let ds = [l(0, "a"), l(0, "b"), l(4, "c")];
    let removed: Vec<_> = ds[1..].iter().map(|r| r.value.clone()).collect();
    let mut cm = deco_editor("foo\nbar", ds.to_vec());
    cm.deco_effects(
        None,
        Some(&mut |_, _, deco| !removed.iter().any(|r| Arc::ptr_eq(r, deco))),
        Vec::new(),
    );
    classes(&cm, &["a", ""]);
}

#[test]
fn handles_line_joining_properly() {
    let mut cm = deco_editor("x\ny\nz", vec![l(0, "a"), l(2, "b"), l(4, "c")]);
    cm.change(ChangeSpec::delete(1, 4));
    classes(&cm, &["a"]);
}

#[test]
fn handles_line_splitting_properly() {
    let mut cm = deco_editor("abc", vec![l(0, "a")]);
    cm.change(ChangeSpec::replace(1, 2, "\n"));
    classes(&cm, &["a", ""]);
}

#[test]
fn can_handle_insertion() {
    let mut cm = deco_editor("x\ny\nz", vec![l(2, "a"), l(4, "b")]);
    cm.change(ChangeSpec::insert(2, "hi"));
    classes(&cm, &["", "a", "b"]);
}

// block widgets

// Upstream's `BlockWidget`, an `hr` with a `data-name`; here a `div` with
// a `data-test` attribute the serializer reads (div-gap 23, 25).
struct BlockWidget(String);

impl WidgetType for BlockWidget {
    fn to_dom(&self, document: &Document) -> Result<Element, div::Error> {
        let elt = Element::create(document, "hr")?;
        elt.set_attribute("class", "bw")?;
        elt.set_attribute("data-test", &self.0)?;
        Ok(elt)
    }

    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

fn bw(pos: usize, side: i32, name: &str) -> Range<Decoration> {
    Decoration::widget(WidgetSpec {
        side,
        block: true,
        ..WidgetSpec::new(BlockWidget(name.into()))
    })
    .range(pos, pos)
    .unwrap()
}

fn br(from: usize, to: usize, name: &str) -> Range<Decoration> {
    Decoration::replace(ReplaceSpec {
        widget: Some(editor_view::Widget::new(BlockWidget(name.into()))),
        block: true,
        ..Default::default()
    })
    .range(from, to)
    .unwrap()
}

// Upstream's `widgets`: the block widgets between the lines.
fn widgets(cm: &Ed, groups: &[&[&str]]) {
    let mut found: Vec<Vec<String>> = vec![Vec::new()];
    for line in cm.lines() {
        if line.get_attribute("class").unwrap().as_deref() == Some("bw") {
            let name = line.get_attribute("data-test").unwrap().unwrap();
            found.last_mut().unwrap().push(name);
        } else {
            found.push(Vec::new());
        }
    }
    let expect: Vec<Vec<String>> = groups
        .iter()
        .map(|g| g.iter().map(|s| s.to_string()).collect())
        .collect();
    assert_eq!(found, expect);
}

#[test]
fn draws_block_widgets_in_the_right_place() {
    let cm = deco_editor(
        "foo\nbar",
        vec![
            bw(0, -1, "A"),
            bw(3, 1, "B"),
            bw(3, 2, "C"),
            bw(4, -2, "D"),
            bw(4, -1, "E"),
            bw(7, 1, "F"),
        ],
    );
    widgets(&cm, &[&["A"], &["B", "C", "D", "E"], &["F"]]);
}

#[test]
fn adds_widgets_when_they_appear() {
    let mut cm = deco_editor("foo\nbar", vec![bw(7, 1, "Y")]);
    cm.add(vec![bw(0, -1, "X"), bw(7, 2, "Z")]);
    widgets(&cm, &[&["X"], &[], &["Y", "Z"]]);
}

#[test]
fn removes_widgets_when_they_vanish() {
    let mut cm = deco_editor(
        "foo\nbar",
        vec![bw(0, -1, "A"), bw(3, 1, "B"), bw(4, -1, "C"), bw(7, 1, "D")],
    );
    widgets(&cm, &[&["A"], &["B", "C"], &["D"]]);
    cm.deco_effects(
        None,
        Some(&mut |_, _, deco| deco.start_side() < 0),
        Vec::new(),
    );
    widgets(&cm, &[&["A"], &["C"], &[]]);
}

#[test]
fn draws_block_ranges() {
    let cm = deco_editor("one\ntwo\nthr\nfou", vec![br(4, 11, "A")]);
    widgets(&cm, &[&[], &["A"], &[]]);
}

#[test]
fn can_add_widgets_at_the_end_and_start_of_the_doc() {
    let mut cm = deco_editor("one\ntwo", Vec::new());
    cm.add(vec![bw(0, -1, "X"), bw(7, 1, "Y")]);
    widgets(&cm, &[&["X"], &[], &["Y"]]);
}

#[test]
fn can_add_widgets_around_inner_lines() {
    let mut cm = deco_editor("one\ntwo", Vec::new());
    cm.add(vec![bw(3, 1, "X"), bw(4, -1, "Y")]);
    widgets(&cm, &[&[], &["X", "Y"], &[]]);
}

#[test]
fn can_replace_an_empty_line_with_a_range() {
    let cm = deco_editor("one\n\ntwo", vec![br(4, 4, "A")]);
    widgets(&cm, &[&[], &["A"], &[]]);
}

#[test]
fn can_put_a_block_range_in_the_middle_of_a_line() {
    let mut cm = deco_editor("hello", vec![br(2, 3, "X")]);
    widgets(&cm, &[&[], &["X"], &[]]);
    cm.deco_effects(
        Some(ChangeSpec::replace(1, 2, "u")),
        None,
        vec![br(2, 3, "X")],
    );
    widgets(&cm, &[&[], &["X"], &[]]);
}

// block wrappers

fn wrapper(name: &str, rank: Option<f64>) -> Arc<BlockWrapper> {
    BlockWrapper::create(BlockWrapperSpec {
        tag_name: "section".into(),
        attributes: Some(attrs(&[("data-test", name)])),
        rank,
    })
}

fn wrap_editor(doc: &str, blocks: Vec<Range<BlockWrapper>>) -> Ed {
    temp_view_with(doc, Vec::new(), vec![RangeSet::of(blocks, false)])
}

// Upstream's `html`: the content without classes, attributes or buffers.
// Wrappers are `section` elements told apart by `data-test` (div-gap 25).
fn wrapped(cm: &Ed) -> String {
    let mut html = cm.html().replace(" class=\"cm-line\"", "");
    for name in ["section", "navigation", "wrap-a", "wrap-b", "wrap-c"] {
        html = html.replace(&format!("<div data-test=\"{name}\">"), &format!("<{name}>"));
    }
    // The closing tags follow the opening ones, innermost first.
    let mut out = String::new();
    let mut stack: Vec<String> = Vec::new();
    let mut rest = html.as_str();
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let end = rest[start..].find('>').unwrap() + start;
        let tag = &rest[start + 1..end];
        if tag.starts_with('/') {
            let open = stack.pop().unwrap();
            out.push_str(&format!("</{open}>"));
        } else {
            stack.push(tag.to_string());
            out.push_str(&format!("<{tag}>"));
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

#[test]
fn can_wrap_a_line() {
    let cm = wrap_editor("a\nb\nc", vec![wrapper("section", None).range(2, 2)]);
    assert_eq!(
        wrapped(&cm),
        "<div>a</div><section><div>b</div></section><div>c</div>"
    );
}

#[test]
fn can_wrap_multiple_lines() {
    let cm = wrap_editor("a\nb\nc", vec![wrapper("section", None).range(0, 2)]);
    assert_eq!(
        wrapped(&cm),
        "<section><div>a</div><div>b</div></section><div>c</div>"
    );
}

#[test]
fn only_takes_effect_at_the_start_of_a_line() {
    let cm = wrap_editor("ab\ncd", vec![wrapper("section", None).range(1, 1)]);
    assert!(!cm.html().contains("data-test"));
}

#[test]
fn can_nest_wrappers() {
    let mut cm = wrap_editor(
        "a\nb\nc",
        vec![
            wrapper("navigation", None).range(0, 5),
            wrapper("section", None).range(2, 2),
        ],
    );
    assert_eq!(
        wrapped(&cm),
        "<navigation><div>a</div><section><div>b</div></section><div>c</div></navigation>"
    );
    cm.change(ChangeSpec::insert(2, "?"));
    assert_eq!(
        wrapped(&cm),
        "<navigation><div>a</div><section><div>?b</div></section><div>c</div></navigation>"
    );
}

#[test]
fn uses_precedence_to_determine_nesting_order() {
    let cm = temp_view_with(
        "ab\ncd\nef",
        Vec::new(),
        vec![
            RangeSet::of(vec![wrapper("navigation", None).range(0, 8)], false),
            RangeSet::of(vec![wrapper("section", None).range(3, 5)], false),
        ],
    );
    assert_eq!(
        wrapped(&cm),
        "<navigation><div>ab</div></navigation><section><navigation><div>cd</div></navigation></section><navigation><div>ef</div></navigation>"
    );
}

#[test]
fn uses_rank_to_determine_nesting_order_within_a_source() {
    let cm = temp_view_with(
        "a\nb\nc",
        Vec::new(),
        vec![RangeSet::of(
            vec![
                wrapper("wrap-a", Some(2.0)).range(2, 3),
                wrapper("wrap-b", Some(1.0)).range(2, 3),
                wrapper("wrap-c", Some(3.0)).range(2, 3),
            ],
            false,
        )],
    );
    assert_eq!(
        wrapped(&cm),
        "<div>a</div><wrap-c><wrap-a><wrap-b><div>b</div></wrap-b></wrap-a></wrap-c><div>c</div>"
    );
}

#[test]
fn can_handle_changes_in_wrappers() {
    let mut cm = wrap_editor(
        "ab\ncd\nef",
        vec![
            wrapper("navigation", None).range(0, 8),
            wrapper("section", None).range(3, 5),
        ],
    );
    cm.change(ChangeSpec::List(vec![
        ChangeSpec::delete(0, 1),
        ChangeSpec::insert(3, "."),
        ChangeSpec::delete(4, 7),
    ]));
    assert_eq!(
        wrapped(&cm),
        "<navigation><div>b</div><section><div>.cf</div></section></navigation>"
    );
}

#[test]
fn can_handle_replacements_at_end_of_wrappers() {
    let mut cm = wrap_editor("ab\ncd\nef", vec![wrapper("section", None).range(0, 5)]);
    cm.change(ChangeSpec::delete(4, 7));
    assert_eq!(
        wrapped(&cm),
        "<section><div>ab</div><div>cf</div></section>"
    );
}

// Port-specific: an update frees the tiles it drops, and the DOM nodes
// that go with them.
#[test]
fn updates_free_dropped_tiles() {
    let mut cm = deco_editor("one\ntwo\nthree", vec![d(1, 6, "a"), w(9, "x", 0)]);
    let before = cm.view().tile_count();
    for _ in 0..20 {
        cm.change(ChangeSpec::insert(5, "x"));
        cm.change(ChangeSpec::delete(5, 6));
    }
    assert_eq!(cm.view().tile_count(), before);
    assert_eq!(cm.dom_text(), "one\ntwo\nthree");
}

#[test]
fn resolves_blocks() {
    let cm = deco_editor("one\ntwo", vec![bw(4, -1, "A")]);
    assert_eq!(
        cm.view().resolve_block(5, 1),
        ("LineTile(\"two\")".to_string(), 1)
    );
    assert_eq!(cm.view().resolve_block(3, -1).0, "LineTile(\"one\")#");
}
