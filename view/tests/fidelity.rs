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

use std::sync::OnceLock;

use div::typed_style::{parse_literal, Overflow, StyleProperty};
use div::{Document, Element, FontAxisRange, FontFaceDescriptor, FontStyleRange};
use editor_state::{
    EditorSelection, EditorState, EditorStateConfig, Extension, Insertion, RangeSetBuilder, Text,
};
use editor_view::__test::ContentTiles;
use editor_view::{
    Decoration, DecorationSet, EditorView, EditorViewConfig, LineSpec, MarkSpec, ReplaceSpec,
    ScrollStrategy, StyleSpec, Viewport, Widget, WidgetSpec, WidgetType, WindowSize,
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

// The editor view, mounted as `main.mjs` mounts each case.

// Geist Mono, the file the reference page loads (`fidelity/reference`).
const GEIST_MONO: &[u8] =
    include_bytes!("../../fidelity/reference/pages/fonts/GeistMono-Variable.woff2");

// A number field of the case's JSON.
fn case_number(json: &str, field: &str) -> Option<usize> {
    let key = format!("\"{field}\": ");
    let start = json.find(&key)? + key.len();
    let end = json[start..].find(|c: char| !c.is_ascii_digit())? + start;
    json[start..end].parse().ok()
}

// The case's theme (`theme` in `main.mjs`). Created once, so that its
// class is the first theme name after upstream's own, `ͼ5`, as in the
// reference page.
fn case_theme() -> Extension {
    static THEME: OnceLock<Extension> = OnceLock::new();
    THEME
        .get_or_init(|| {
            EditorView::theme(
                &StyleSpec::new()
                    .nest("&", StyleSpec::new().set("height", "600px"))
                    .nest(
                        ".cm-scroller",
                        StyleSpec::new()
                            .set("fontFamily", "'Geist Mono', monospace")
                            .set("fontSize", "13px")
                            .set("lineHeight", "18px"),
                    )
                    .nest(".cm-ref-mark", StyleSpec::new().set("color", "#b00020"))
                    .nest(
                        ".cm-ref-line",
                        StyleSpec::new().set("backgroundColor", "#eef3ff"),
                    )
                    .nest(".cm-ref-widget", StyleSpec::new().set("color", "#555555")),
                false,
            )
            .expect("the case theme")
        })
        .clone()
}

// Set a style property on an element of the test page.
fn page_style(element: &Element, name: &str, value: &str) {
    let property = StyleProperty::from_css_name(name).unwrap();
    element
        .set_property(property, parse_literal(property, value).unwrap())
        .unwrap();
}

fn mount(case: &Case) -> (EditorView, Document) {
    let source = case_doc(case.json);
    let doc = Text::of(&source.split('\n').collect::<Vec<_>>()).unwrap();
    let document = Document::new(800, 600).unwrap();
    let mut descriptor = FontFaceDescriptor::new("Geist Mono");
    descriptor.style = FontStyleRange::Normal;
    descriptor.weight = FontAxisRange::new(100.0, 900.0);
    document.register_font_face(GEIST_MONO, descriptor).unwrap();
    // The page's own stylesheet, `html, body { margin: 0 } body { width:
    // 800px; overflow: hidden }`, as inline styles (div-gap 14a).
    let body = document.body();
    page_style(&body, "margin", "0");
    page_style(&body, "width", "800px");
    page_style(&body, "overflow", "hidden");

    let mut extensions = vec![case_theme()];
    if case.name == "decorations" {
        extensions.push(EditorView::decorations(reference_decorations(&doc)));
    }
    let anchor = case_number(case.json, "anchor").expect("an anchor");
    let scroll_to = case_number(case.json, "scrollLine").map(|line| {
        EditorView::scroll_into_view_with(
            EditorSelection::cursor(doc.line(line).from),
            ScrollStrategy::Start,
            ScrollStrategy::Nearest,
            5.0,
            5.0,
        )
    });
    let state = EditorState::create(EditorStateConfig {
        doc: Some(Insertion::Text(doc)),
        selection: Some(EditorSelection::single(anchor, anchor)),
        extensions: Extension::from(extensions),
    })
    .unwrap();
    let view = EditorView::new(
        &document,
        EditorViewConfig {
            state,
            parent: Some(body),
            scroll_to,
            window: WindowSize {
                width: 800.0,
                height: 600.0,
            },
        },
    )
    .unwrap();
    (view, document)
}

// The captured `.cm-content` element: the scroller's last child.
fn captured_content(html: &str) -> &str {
    let html = html.trim_end();
    let start = html.find("<div spellcheck=").expect("a .cm-content");
    let end = html.len() - "</div></div>".len();
    &html[start..end]
}

// The markup with the `.cm-content` element's attributes sorted by name.
// Attribute order is not compared: Div has no attribute enumeration
// (div-gap 23), so the order the view's serializer writes is its own, not
// one Div keeps. Chromium lists `style` between `contenteditable` and
// `class`; upstream's `updateAttrs` writes `class` first.
fn content_attributes_sorted(html: &str) -> String {
    let class = html.find("class=\"cm-content\"").expect("a .cm-content");
    let start = html[..class].rfind("<div ").expect("its open tag") + "<div ".len();
    let end = class + html[class..].find('>').expect("its open tag");
    let mut attributes: Vec<&str> = html[start..end]
        .split("\" ")
        .map(|attribute| attribute.trim_end_matches('"'))
        .collect();
    attributes.sort_unstable();
    let attributes: Vec<String> = attributes.iter().map(|a| format!("{a}\"")).collect();
    format!("{}{}{}", &html[..start], attributes.join(" "), &html[end..])
}

#[test]
fn plain_editor_matches_chromium() {
    let case = case!("plain");
    let (view, _document) = mount(&case);
    assert_eq!(
        view.viewport(),
        Viewport {
            from: 0,
            to: case_doc(case.json)
                .split('\n')
                .take(36)
                .map(|l| l.len() + 1)
                .sum::<usize>()
                - 1
        }
    );
    assert_eq!(
        content_attributes_sorted(&view.outer_html().unwrap()),
        content_attributes_sorted(case.html.trim_end())
    );
}

#[test]
fn editor_content_matches_chromium() {
    for case in [case!("plain"), case!("gutter"), case!("decorations")] {
        let (view, _document) = mount(&case);
        assert_eq!(
            content_attributes_sorted(&view.content_html().unwrap()),
            content_attributes_sorted(captured_content(case.html)),
            "{}: .cm-content differs from {FIXTURES}",
            case.name
        );
    }
}

// Upstream scrolls only an element whose computed `overflow-y` is `auto`
// or `scroll`. The scroller sets `overflow-x: auto`, which per CSS
// Overflow 3 makes its `visible` `overflow-y` compute to `auto`; Div keeps
// `visible`, so `scrollRectIntoView` passes over the scroller, and the
// next measure moves the viewport back to the top. With `overflow-y:
// auto` added to the case theme, both tests below pass: the experiment
// that the design document's measurements record.
// div-gap 28, as measured at the pinned Div: when this fails, Div
// computes the overflow pair, and the two tests below should pass.
#[test]
fn scroller_overflow_y_stays_visible_in_div() {
    let (view, _document) = mount(&case!("plain"));
    let style = view.scroll_dom().computed_style().unwrap();
    assert_eq!(*style.overflow_x(), Overflow::Auto);
    assert_eq!(*style.overflow_y(), Overflow::Visible);
}

#[test]
#[ignore = "div-gap 28: overflow-y keeps `visible` beside overflow-x `auto`"]
fn scrolled_editor_content_matches_chromium() {
    let case = case!("scrolled");
    let (view, _document) = mount(&case);
    assert_eq!(
        content_attributes_sorted(&view.content_html().unwrap()),
        content_attributes_sorted(captured_content(case.html)),
    );
}

#[test]
#[ignore = "div-gap 28: overflow-y keeps `visible` beside overflow-x `auto`"]
fn scrolled_editor_scrolls_as_chromium() {
    let case = case!("scrolled");
    let (view, _document) = mount(&case);
    let expected = case_number(case.json, "scrollTop").unwrap() as f64;
    let scroll_top = view.scroll_dom().scroll_top().unwrap();
    assert!(
        (scroll_top - expected).abs() < 1.0,
        "scrollTop {scroll_top}, Chromium {expected}"
    );
}

// The selectors of the rules in the reference page's first stylesheet,
// the one style-mod fills, at the top level.
fn captured_selectors(css: &str) -> Vec<String> {
    let sheet = css
        .split("/* document.styleSheets[1] */")
        .next()
        .unwrap()
        .trim_start_matches("/* document.styleSheets[0] */\n");
    sheet
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with(' ') && !line.starts_with('}'))
        .map(|line| line[..line.find('{').unwrap()].trim_end().to_string())
        .collect()
}

// CSSOM serializes attribute selector values quoted:
// `[contenteditable=true]` reads back as `[contenteditable="true"]`.
fn quote_attribute_values(selector: &str) -> String {
    let mut out = String::new();
    let mut rest = selector;
    while let Some(eq) = rest.find('=') {
        out.push_str(&rest[..=eq]);
        rest = &rest[eq + 1..];
        if !rest.starts_with('"') {
            let end = rest.find(']').unwrap();
            out.push('"');
            out.push_str(&rest[..end]);
            out.push('"');
            rest = &rest[end..];
        }
    }
    out.push_str(rest);
    out
}

#[test]
fn mounted_rules_match_chromium() {
    let case = case!("plain");
    let (view, _document) = mount(&case);
    let css = include_str!("../../fidelity/fixtures/reference-v1/plain/styles.css");
    let expected = captured_selectors(css);
    let text = view.mounted_style_text();
    let mounted: Vec<String> = text
        .lines()
        .filter(|line| !line.is_empty())
        .map(|rule| quote_attribute_values(rule[..rule.find(" {").unwrap()].trim_end()))
        .collect();
    assert_eq!(mounted, expected);
}
