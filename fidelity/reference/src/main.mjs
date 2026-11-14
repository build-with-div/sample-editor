// The CodeMirror 6 reference cases for the fidelity captures.
//
// Each case mounts the pinned packages with a fixed theme, font, document,
// selection and scroll position. `fidelity/capture.py` loads
// `pages/editor.html#<case>` in pinned Chromium and records what the port
// must reproduce. The Div side rebuilds the same state from the case's
// meta.json, so everything here is deterministic.

import { EditorState, EditorSelection, RangeSetBuilder } from "@codemirror/state";
import { Decoration, EditorView, WidgetType, lineNumbers } from "@codemirror/view";

const WIDTH = 800;
const HEIGHT = 600;

// A deterministic Rust-like document: no randomness, so every build and
// every capture sees the same text.
function documentText(lines) {
  const words = ["let", "value", "self", "match", "Some", "None", "fn", "pub", "impl", "return"];
  const out = [];
  for (let i = 0; i < lines; i++) {
    const indent = "    ".repeat(i % 4);
    const a = words[i % words.length];
    const b = words[(i * 7 + 3) % words.length];
    switch (i % 5) {
      case 0: out.push(`${indent}fn ${a}_${i}(${b}: usize) -> usize {`); break;
      case 1: out.push(`${indent}let ${b} = ${a}.get(${i}).unwrap_or(0);`); break;
      case 2: out.push(`${indent}// ${a} ${b} line ${i}`); break;
      case 3: out.push(`${indent}${a}::${b}(&mut self, "${i}")`); break;
      default: out.push(`${indent}}`);
    }
  }
  return out.join("\n");
}

const DOC = documentText(300);

const theme = EditorView.theme({
  "&": { height: `${HEIGHT}px` },
  ".cm-scroller": {
    fontFamily: "'Geist Mono', monospace",
    fontSize: "13px",
    lineHeight: "18px",
  },
  ".cm-ref-mark": { color: "#b00020" },
  ".cm-ref-line": { backgroundColor: "#eef3ff" },
  ".cm-ref-widget": { color: "#555555" },
});

class TextWidget extends WidgetType {
  constructor(text) {
    super();
    this.text = text;
  }
  eq(other) {
    return other.text === this.text;
  }
  toDOM() {
    const span = document.createElement("span");
    span.className = "cm-ref-widget";
    span.textContent = this.text;
    return span;
  }
}

function lineStart(n) {
  return EditorState.create({ doc: DOC }).doc.line(n).from;
}

function decorations() {
  const state = EditorState.create({ doc: DOC });
  const line = (n) => state.doc.line(n);
  const builder = new RangeSetBuilder();
  // Ranges are added in document order, as RangeSetBuilder requires.
  builder.add(line(2).from, line(2).from, Decoration.line({ class: "cm-ref-line" }));
  builder.add(line(2).from + 4, line(2).from + 7, Decoration.mark({ class: "cm-ref-mark" }));
  builder.add(line(3).from + 2, line(3).from + 2, Decoration.widget({ widget: new TextWidget("[w]"), side: 1 }));
  builder.add(line(4).from + 8, line(4).to - 2, Decoration.replace({ widget: new TextWidget("…") }));
  return EditorView.decorations.of(builder.finish());
}

// Each case: extensions, the selection, and where it scrolls to.
const CASES = {
  plain: { extensions: [], anchor: lineStart(3) + 4, scrollLine: null },
  gutter: { extensions: [lineNumbers()], anchor: lineStart(3) + 4, scrollLine: null },
  decorations: { extensions: [lineNumbers(), decorations()], anchor: lineStart(6), scrollLine: null },
  scrolled: { extensions: [lineNumbers()], anchor: lineStart(150), scrollLine: 150 },
};

function mount(name) {
  const spec = CASES[name];
  if (!spec) throw new Error(`unknown case ${name}`);
  const state = EditorState.create({
    doc: DOC,
    selection: EditorSelection.cursor(spec.anchor),
    extensions: [theme, spec.extensions],
  });
  const view = new EditorView({
    state,
    parent: document.body,
    scrollTo:
      spec.scrollLine === null
        ? undefined
        : EditorView.scrollIntoView(state.doc.line(spec.scrollLine).from, { y: "start" }),
  });
  window.__case = {
    name,
    width: WIDTH,
    height: HEIGHT,
    doc: state.doc.toString(),
    selection: state.selection.toJSON(),
    scrollLine: spec.scrollLine,
  };
  window.__view = view;
}

window.__cases = Object.keys(CASES);
mount(location.hash.slice(1) || "plain");
