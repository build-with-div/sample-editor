//! Structural fidelity: the `.cm-line` elements the port draws on Div,
//! against those CodeMirror 6 drew in pinned Chromium 147 for the
//! reference cases (`fidelity/fixtures/reference-v1`, made by
//! `fidelity/capture.py` from `fidelity/reference/src/main.mjs`).
//!
//! Chromium drew only the lines in its viewport, with a `.cm-gap` spacer
//! for the rest; the port has no viewport yet and draws every line, so
//! the captured lines are compared with the same lines of the port's
//! output. The port's lines are read from Div's DOM in tile order, with
//! the attributes CodeMirror and the cases write, by name (div-gap 23,
//! 24, 25; see `ContentTiles::block_html`).

use div::{Document, Element};
use editor_state::{RangeSetBuilder, Text};
use editor_view::__test::ContentTiles;
use editor_view::{
    Decoration, DecorationSet, LineSpec, MarkSpec, ReplaceSpec, Widget, WidgetSpec, WidgetType,
};

const FIXTURES: &str = "../../fidelity/fixtures/reference-v1";

struct Case {
    name: &'static str,
    html: &'static str,
    json: &'static str,
}

macro_rules! case {
    ($name:literal) => {
        Case {
            name: $name,
            html: include_str!(concat!(
                "../../fidelity/fixtures/reference-v1/",
                $name,
                "/editor.html"
            )),
            json: include_str!(concat!(
                "../../fidelity/fixtures/reference-v1/",
                $name,
                "/case.json"
            )),
        }
    };
}

// The `"doc"` string of a case's JSON.
fn case_doc(json: &str) -> String {
    let start = json.find("\"doc\": \"").expect("a doc") + "\"doc\": \"".len();
    let mut out = String::new();
    let mut chars = json[start..].chars();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => return out,
            '\\' => match chars.next().unwrap() {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
                }
                other => out.push(other),
            },
            ch => out.push(ch),
        }
    }
    panic!("unterminated doc string");
}

// The `.cm-line` elements in the captured `.cm-content`.
fn captured_lines(html: &str) -> Vec<&str> {
    let content = &html[html.find("class=\"cm-content\"").expect("a .cm-content")..];
    let mut lines = Vec::new();
    let mut rest = content;
    while let Some(start) = rest.find("<div class=\"cm-line") {
        // Lines hold no nested `div`s.
        let end = rest[start..].find("</div>").unwrap() + start + "</div>".len();
        lines.push(&rest[start..end]);
        rest = &rest[end..];
    }
    lines
}

// The first line number in the captured gutter, when there is one.
fn first_gutter_line(html: &str) -> Option<usize> {
    let marker = "class=\"cm-gutterElement\" style=\"height: 18px";
    let start = html.find(marker)? + marker.len();
    let text_start = html[start..].find('>')? + start + 1;
    let text_end = html[text_start..].find('<')? + text_start;
    html[text_start..text_end].parse().ok()
}

// The case's `TextWidget`: a span with the widget's text.
struct TextWidget(&'static str);

impl WidgetType for TextWidget {
    fn to_dom(&self, document: &Document) -> Result<Element, div::Error> {
        let span = Element::create(document, "span")?;
        span.set_class("cm-ref-widget")?;
        span.set_text(self.0)?;
        Ok(span)
    }

    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

// The decorations of the `decorations` case (`decorations()` in
// `main.mjs`).
fn reference_decorations(doc: &Text) -> DecorationSet {
    let line = |n: usize| doc.line(n);
    let mut builder = RangeSetBuilder::new();
    let line2 = line(2).from;
    builder.add(
        line2,
        line2,
        Decoration::line(LineSpec {
            attributes: None,
            class: Some("cm-ref-line".into()),
            extra: None,
        }),
    );
    builder.add(
        line2 + 4,
        line2 + 7,
        Decoration::mark(MarkSpec {
            class: Some("cm-ref-mark".into()),
            ..Default::default()
        }),
    );
    let line3 = line(3).from + 2;
    builder.add(
        line3,
        line3,
        Decoration::widget(WidgetSpec {
            side: 1,
            ..WidgetSpec::new(TextWidget("[w]"))
        }),
    );
    let (from4, to4) = (line(4).from + 8, line(4).to - 2);
    builder.add(
        from4,
        to4,
        Decoration::replace(ReplaceSpec {
            widget: Some(Widget::new(TextWidget("\u{2026}"))),
            ..Default::default()
        }),
    );
    builder.finish()
}

fn draw(case: &Case) -> (Vec<String>, Text) {
    let source = case_doc(case.json);
    let doc = Text::of(&source.split('\n').collect::<Vec<_>>()).unwrap();
    let decorations = if case.name == "decorations" {
        vec![reference_decorations(&doc)]
    } else {
        Vec::new()
    };
    let document = Document::new(800, 600).unwrap();
    let content = Element::create(&document, "div").unwrap();
    document.body().append_child(&content).unwrap();
    let view = ContentTiles::new(&document, content, &doc, decorations, Vec::new()).unwrap();
    (view.block_html().unwrap(), doc)
}

// Compare a case's captured lines with the port's, from line `first`
// (1-based), and return how many matched.
fn compare(case: &Case, first: usize) -> usize {
    let expected = captured_lines(case.html);
    let (drawn, doc) = draw(case);
    assert_eq!(
        drawn.len(),
        doc.lines(),
        "{}: one block per line",
        case.name
    );
    assert!(!expected.is_empty(), "{}: captured lines", case.name);
    let mut matched = 0;
    for (i, line) in expected.iter().enumerate() {
        assert_eq!(
            drawn[first - 1 + i],
            *line,
            "{}: line {} differs from {FIXTURES}",
            case.name,
            first + i
        );
        matched += 1;
    }
    matched
}

#[test]
fn plain_lines_match_chromium() {
    assert_eq!(compare(&case!("plain"), 1), 36);
}

#[test]
fn gutter_case_lines_match_chromium() {
    let case = case!("gutter");
    assert_eq!(first_gutter_line(case.html), Some(1));
    assert_eq!(compare(&case, 1), 36);
}

#[test]
fn decorated_lines_match_chromium() {
    assert_eq!(compare(&case!("decorations"), 1), 36);
}

#[test]
fn scrolled_lines_match_chromium() {
    let case = case!("scrolled");
    let first = first_gutter_line(case.html).expect("a gutter");
    assert_eq!(first, 122);
    assert_eq!(compare(&case, first), 90);
}
