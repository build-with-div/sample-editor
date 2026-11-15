//! View plugins and gutters on Div.
//!
//! The first tests port upstream's `test/webtest-extension.ts`. Upstream
//! tests gutters only in the browser through its demos, so the gutter
//! tests are this port's own, checked against `gutter.ts`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use div::{Document, Element};
use editor_state::{
    facets, ChangeSpec, Compartment, EditorSelection, EditorState, EditorStateConfig, Extension,
    Insertion, Text, TransactionSpec,
};
use editor_view::__test::set_style;
use editor_view::{
    highlight_active_line_gutter, line_numbers, Attrs, EditorView, EditorViewConfig,
    LineNumberConfig, PluginSpec, PluginValue, ViewPlugin, ViewUpdate, Viewport, WindowSize,
};

// `tempView`: a view of `doc` with `extensions`, in an 800x600 document.
fn temp_view(doc: &str, extensions: Extension) -> (EditorView, Document) {
    let document = Document::new(800, 600).unwrap();
    let doc = Text::of(&doc.split('\n').collect::<Vec<_>>()).unwrap();
    let state = EditorState::create(EditorStateConfig {
        doc: Some(Insertion::Text(doc)),
        selection: None,
        extensions,
    })
    .unwrap();
    let view = EditorView::new(
        &document,
        EditorViewConfig {
            state,
            parent: Some(document.body()),
            scroll_to: None,
            window: WindowSize {
                width: 800.0,
                height: 600.0,
            },
        },
    )
    .unwrap();
    (view, document)
}

fn dispatch(view: &mut EditorView, spec: TransactionSpec) {
    let tr = view.state().update(spec).unwrap();
    view.dispatch(tr).unwrap();
}

// The element children of `parent`.
fn children(parent: &Element) -> Vec<Element> {
    let mut out = Vec::new();
    let mut child = parent.first_child().unwrap();
    while let Some(next) = child {
        child = next.next_sibling().unwrap();
        out.push(next);
    }
    out
}

fn has_class(element: &Element, class: &str) -> bool {
    element
        .get_attribute("class")
        .unwrap()
        .is_some_and(|list| list.split_whitespace().any(|c| c == class))
}

struct ViewportRecorder(Arc<Mutex<Vec<Viewport>>>);

impl PluginValue for ViewportRecorder {
    fn update(&mut self, update: &ViewUpdate, view: &EditorView) -> Result<(), div::Error> {
        if update.viewport_changed() {
            self.0.lock().unwrap().push(view.viewport());
        }
        Ok(())
    }
}

#[test]
fn calls_update_when_the_viewport_changes() {
    let viewports = Arc::new(Mutex::new(Vec::new()));
    let record = viewports.clone();
    let plugin = ViewPlugin::define(
        move |view: &EditorView| {
            record.lock().unwrap().push(view.viewport());
            Ok(ViewportRecorder(record.clone()))
        },
        PluginSpec::default(),
    );
    let (mut cm, _document) = temp_view(&"x\n".repeat(500), plugin.extension());
    {
        let viewports = viewports.lock().unwrap();
        // Upstream measures in an animation frame, after `tempView`
        // returns; here the first measure runs in the constructor
        // (div-gap 13), and the viewport it settles on may already have
        // been recorded.
        assert!(!viewports.is_empty());
        assert_eq!(viewports[0].from, 0);
    }
    set_style(cm.dom(), "height", "300px").unwrap();
    set_style(cm.scroll_dom(), "overflow", "auto").unwrap();
    cm.scroll_dom().scroll_to(0.0, 2000.0).unwrap();
    let before = viewports.lock().unwrap().len();
    cm.measure().unwrap();
    {
        let viewports = viewports.lock().unwrap();
        assert!(viewports.len() > before);
        let last = viewports[viewports.len() - 1];
        assert!(last.from > 0);
        assert!(last.to > viewports[0].from);
    }
    cm.scroll_dom().scroll_to(0.0, 4000.0).unwrap();
    let cur_len = viewports.lock().unwrap().len();
    cm.measure().unwrap();
    assert!(viewports.lock().unwrap().len() > cur_len);
}

struct UpdateCounter {
    updates: Arc<Mutex<usize>>,
    prev_doc: Text,
}

impl PluginValue for UpdateCounter {
    fn update(&mut self, update: &ViewUpdate, view: &EditorView) -> Result<(), div::Error> {
        assert!(*update.start_state.doc() == self.prev_doc);
        assert!(*update.state.doc() == *view.state().doc());
        self.prev_doc = view.state().doc().clone();
        *self.updates.lock().unwrap() += 1;
        Ok(())
    }
}

#[test]
fn calls_update_on_plugins() {
    let updates = Arc::new(Mutex::new(0));
    let count = updates.clone();
    let plugin = ViewPlugin::define(
        move |view: &EditorView| {
            Ok(UpdateCounter {
                updates: count.clone(),
                prev_doc: view.state().doc().clone(),
            })
        },
        PluginSpec::default(),
    );
    let (mut cm, _document) = temp_view("xyz", plugin.extension());
    // Measuring the new view may update the plugins (div-gap 13), as
    // upstream's first animation frame would; count from here.
    let start = *updates.lock().unwrap();
    dispatch(
        &mut cm,
        TransactionSpec {
            changes: Some(ChangeSpec::replace(1, 2, "u")),
            ..Default::default()
        },
    );
    assert_eq!(*updates.lock().unwrap(), start + 1);
    dispatch(
        &mut cm,
        TransactionSpec {
            selection: Some(EditorSelection::single(3, 3)),
            ..Default::default()
        },
    );
    assert_eq!(*updates.lock().unwrap(), start + 2);
}

#[test]
fn allows_content_attributes_to_be_changed_through_effects() {
    let mut attrs = Attrs::new();
    attrs.set("spellcheck", "true");
    let (cm, _document) = temp_view("", EditorView::content_attributes(attrs));
    assert_eq!(
        cm.content_dom()
            .get_attribute("spellcheck")
            .unwrap()
            .as_deref(),
        Some("true")
    );
}

#[test]
fn allows_editor_attributes_to_be_changed_through_effects() {
    let mut attrs = Attrs::new();
    attrs.set("class", "something");
    let (cm, _document) = temp_view("", EditorView::editor_attributes(attrs));
    assert!(has_class(cm.dom(), "something"));
    assert!(has_class(cm.dom(), "cm-editor"));
}

struct Greeting(Element);

impl PluginValue for Greeting {
    fn destroy(&mut self) -> Result<(), div::Error> {
        self.0.remove()
    }
}

#[test]
fn redraws_the_view_when_phrases_change() {
    let plugin = ViewPlugin::define(
        |view: &EditorView| {
            let elt = Element::create(view.document(), "div")?;
            view.dom().append_child(&elt)?;
            elt.set_text(&view.state().phrase("Hello", &[]))?;
            set_style(&elt, "position", "absolute")?;
            elt.set_attribute("class", "greeting")?;
            Ok(Greeting(elt))
        },
        PluginSpec::default(),
    );
    let lang = Compartment::new();
    let (mut cm, _document) = temp_view(
        "one",
        Extension::from(vec![plugin.extension(), lang.of(Extension::from(vec![]))]),
    );
    let greeting = |cm: &EditorView| {
        let found: Vec<Element> = children(cm.dom())
            .into_iter()
            .filter(|e| has_class(e, "greeting"))
            .collect();
        assert_eq!(found.len(), 1);
        found[0].text_content().unwrap()
    };
    assert_eq!(greeting(&cm), "Hello");
    let phrases = Arc::new(HashMap::from([(
        "Hello".to_string(),
        "Bonjour".to_string(),
    )]));
    dispatch(
        &mut cm,
        TransactionSpec {
            effects: vec![lang.reconfigure(facets::PHRASES.of(phrases))],
            ..Default::default()
        },
    );
    assert_eq!(greeting(&cm), "Bonjour");
}

// Upstream's "supports plugins with arguments" waits for `ViewPlugin.of`,
// which comes with milestone M7.

// The gutters' elements: `.cm-gutters`, then each `.cm-gutter` with its
// gutter elements.
fn gutters(cm: &EditorView) -> Vec<Element> {
    children(cm.scroll_dom())
        .into_iter()
        .filter(|e| has_class(e, "cm-gutters"))
        .collect()
}

// The line number gutter's elements' texts and classes, after the
// spacer.
fn line_number_elements(cm: &EditorView) -> Vec<(String, String)> {
    let gutters = gutters(cm);
    let gutter = &children(&gutters[0])[0];
    assert!(has_class(gutter, "cm-lineNumbers"));
    children(gutter)
        .into_iter()
        .skip(1)
        .map(|e| {
            (
                e.text_content().unwrap(),
                e.get_attribute("class").unwrap().unwrap_or_default(),
            )
        })
        .collect()
}

fn spacer_text(cm: &EditorView) -> String {
    let gutters = gutters(cm);
    let gutter = &children(&gutters[0])[0];
    children(gutter)[0].text_content().unwrap()
}

#[test]
fn draws_a_line_number_for_each_line() {
    let (cm, _document) = temp_view("a\nb\nc", line_numbers(LineNumberConfig::default()));
    assert_eq!(gutters(&cm).len(), 1);
    let numbers: Vec<String> = line_number_elements(&cm)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(numbers, ["1", "2", "3"]);
    assert_eq!(spacer_text(&cm), "9");
    // The gutters come before the content.
    let scroller = children(cm.scroll_dom());
    assert!(has_class(&scroller[0], "cm-gutters"));
    assert!(scroller[1].is_same_node(cm.content_dom()).unwrap());
}

#[test]
fn updates_line_numbers_and_the_spacer_on_changes() {
    let (mut cm, _document) = temp_view("a\nb\nc", line_numbers(LineNumberConfig::default()));
    dispatch(
        &mut cm,
        TransactionSpec {
            changes: Some(ChangeSpec::insert(0, "x\n".repeat(8))),
            ..Default::default()
        },
    );
    let numbers: Vec<String> = line_number_elements(&cm)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    let expected: Vec<String> = (1..=11).map(|n| n.to_string()).collect();
    assert_eq!(numbers, expected);
    assert_eq!(spacer_text(&cm), "99");
    dispatch(
        &mut cm,
        TransactionSpec {
            changes: Some(ChangeSpec::delete(0, 16)),
            ..Default::default()
        },
    );
    let numbers: Vec<String> = line_number_elements(&cm)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(numbers, ["1", "2", "3"]);
}

#[test]
fn formats_line_numbers() {
    let config = LineNumberConfig {
        format_number: Some(editor_state::Callback(Arc::new(
            |n: usize, _state: &EditorState| format!("<{n}>"),
        ))),
    };
    let (cm, _document) = temp_view("a\nb", line_numbers(config));
    let numbers: Vec<String> = line_number_elements(&cm)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(numbers, ["<1>", "<2>"]);
    assert_eq!(spacer_text(&cm), "<9>");
}

#[test]
fn marks_the_active_line_in_the_gutter() {
    let (mut cm, _document) = temp_view(
        "a\nb\nc",
        Extension::from(vec![
            line_numbers(LineNumberConfig::default()),
            highlight_active_line_gutter(),
        ]),
    );
    let active = |cm: &EditorView| -> Vec<bool> {
        line_number_elements(cm)
            .into_iter()
            .map(|(_, class)| class.split_whitespace().any(|c| c == "cm-activeLineGutter"))
            .collect()
    };
    assert_eq!(active(&cm), [true, false, false]);
    dispatch(
        &mut cm,
        TransactionSpec {
            selection: Some(EditorSelection::single(4, 4)),
            ..Default::default()
        },
    );
    assert_eq!(active(&cm), [false, false, true]);
}

#[test]
fn removes_the_gutters_when_reconfigured_away() {
    let gutter = Compartment::new();
    let (mut cm, _document) =
        temp_view("a\nb", gutter.of(line_numbers(LineNumberConfig::default())));
    assert_eq!(gutters(&cm).len(), 1);
    dispatch(
        &mut cm,
        TransactionSpec {
            effects: vec![gutter.reconfigure(Extension::from(vec![]))],
            ..Default::default()
        },
    );
    assert_eq!(gutters(&cm).len(), 0);
    dispatch(
        &mut cm,
        TransactionSpec {
            effects: vec![gutter.reconfigure(line_numbers(LineNumberConfig::default()))],
            ..Default::default()
        },
    );
    assert_eq!(gutters(&cm).len(), 1);
    let numbers: Vec<String> = line_number_elements(&cm)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(numbers, ["1", "2"]);
}
