// The line-rendering benchmark cases, in plain JavaScript for Chromium.
//
// A port of probes/tests/line_elements.rs and probes/tests/inline_cost.rs:
// the same source lines, tokens, colours, element structure and steps, so
// Chromium's timings are the reference for Div's (docs/design.md section 4).
// fidelity/bench/run.py drives it over CDP. Each step mutates the DOM and
// returns how long the mutation took; the runner takes style, layout, paint
// and raster from Chromium's trace of the frame that follows.

"use strict";

const LINE_HEIGHT = 18;
const VIEW_HEIGHT = 600;

// probes/src/lib.rs source_line: deterministic in n, about 5-55 bytes.
function sourceLine(n) {
  switch (n % 5) {
    case 0: return `fn item_${n}(value: u32) -> Option<u32> {`;
    case 1: return `    let total = value.wrapping_mul(${n}) + 7; // step`;
    case 2: return `    if total > ${n} { return Some(total); }`;
    case 3: return "    None";
    default: return "}";
  }
}

// probes/src/lib.rs tokens: identifiers, numbers, runs of spaces, comments
// and single punctuation characters. The lines are ASCII, so these ASCII
// classes agree with Rust's char::is_alphabetic and is_alphanumeric.
function tokens(line) {
  const out = [];
  let i = 0;
  while (i < line.length) {
    const c = line[i];
    let j = i + 1;
    let cls;
    if (/[A-Za-z_]/.test(c)) {
      while (j < line.length && /[A-Za-z0-9_]/.test(line[j])) j++;
      const word = line.slice(i, j);
      cls = ["fn", "let", "if", "return"].includes(word) ? "kw"
        : ["u32", "Option", "Some", "None"].includes(word) ? "ty" : "id";
    } else if (/[0-9]/.test(c)) {
      while (j < line.length && /[0-9]/.test(line[j])) j++;
      cls = "num";
    } else if (c === " ") {
      while (j < line.length && line[j] === " ") j++;
      cls = "ws";
    } else if (c === "/" && line[j] === "/") {
      j = line.length;
      cls = "com";
    } else {
      cls = "punct";
    }
    out.push([line.slice(i, j), cls]);
    i = j;
  }
  return out;
}

function tokenColor(cls) {
  switch (cls) {
    case "kw": return "rgb(175, 0, 219)";
    case "ty": return "rgb(38, 127, 153)";
    case "num": return "rgb(9, 134, 88)";
    case "com": return "rgb(0, 128, 0)";
    default: return "rgb(30, 30, 30)";
  }
}

function el(tag, style) {
  const node = document.createElement(tag);
  if (style) node.style.cssText = style;
  return node;
}

// line_elements.rs line_element: a gutter span and a span per token.
function lineElement(n) {
  const line = el("div", "height: 18px; white-space: pre");
  const gutter = el("span", "display: inline-block; width: 48px; color: #888888");
  gutter.textContent = String(n + 1);
  line.appendChild(gutter);
  for (const [token, cls] of tokens(sourceLine(n))) {
    const span = el("span");
    span.style.color = tokenColor(cls);
    span.textContent = token;
    line.appendChild(span);
  }
  return line;
}

// line_elements.rs shell: a flex column, `depth` nested flex columns, then
// the scroller.
function shell(depth, scrollerStyle) {
  document.body.replaceChildren();
  const outer = el("div",
    "display: flex; flex-direction: column; height: 600px; " +
    "font-family: monospace; font-size: 13px; line-height: 18px");
  let parent = outer;
  for (let i = 0; i < depth; i++) {
    const level = el("div", "display: flex; flex-direction: column; flex-grow: 1; min-height: 0");
    parent.appendChild(level);
    parent = level;
  }
  const scroller = el("div", scrollerStyle);
  parent.appendChild(scroller);
  document.body.appendChild(outer);
  return scroller;
}

const SCROLLER_STYLE = "overflow: auto; flex-grow: 1; min-height: 0; position: relative";

// line_elements.rs all_lines: every line is an element.
function allLines(lines, depth) {
  const scroller = shell(depth, SCROLLER_STYLE);
  const middle = Math.floor(lines / 2);
  return {
    build() {
      for (let n = 0; n < lines; n++) scroller.appendChild(lineElement(n));
    },
    edit() {
      const target = scroller.children[middle];
      scroller.insertBefore(lineElement(middle + 1), target);
      target.remove();
    },
    insertTop() {
      scroller.insertBefore(lineElement(0), scroller.firstChild);
    },
    scroll() {
      scroller.scrollTo(0, (lines * LINE_HEIGHT) / 2);
    },
  };
}

// line_elements.rs windowed: the visible lines, positioned absolutely in a
// tall spacer, rebuilt on scroll.
function windowed(lines) {
  const scroller = shell(0, SCROLLER_STYLE);
  const spacer = el("div");
  spacer.style.height = `${lines * LINE_HEIGHT}px`;
  scroller.appendChild(spacer);
  const visible = Math.floor(VIEW_HEIGHT / LINE_HEIGHT) + 10;
  const renderWindow = (first) => {
    spacer.replaceChildren();
    for (let n = first; n < Math.min(first + visible, lines); n++) {
      const line = lineElement(n);
      line.style.position = "absolute";
      line.style.left = "0px";
      line.style.right = "0px";
      line.style.top = `${n * LINE_HEIGHT}px`;
      spacer.appendChild(line);
    }
  };
  const middle = Math.floor(lines / 2);
  return {
    build() { renderWindow(0); },
    edit() {
      const line = spacer.firstChild.nextSibling;
      const replacement = lineElement(7);
      replacement.style.position = "absolute";
      replacement.style.left = "0px";
      replacement.style.right = "0px";
      replacement.style.top = "18px";
      spacer.insertBefore(replacement, line);
      line.remove();
    },
    insertTop() { renderWindow(0); },
    scroll() {
      scroller.scrollTo(0, middle * LINE_HEIGHT);
      renderWindow(middle);
    },
  };
}

// line_elements.rs flow_line: Plain is one text node, Grouped merges
// adjacent tokens of one colour, Tokens is a span per token.
function flowLine(n, content) {
  const line = el("div", "height: 18px; white-space: pre");
  if (content === "Plain") {
    line.textContent = sourceLine(n);
    return line;
  }
  const groups = [];
  for (const [token, raw] of tokens(sourceLine(n))) {
    const cls = raw === "ws" || raw === "punct" ? "id" : raw;
    const last = groups[groups.length - 1];
    if (content === "Grouped" && last && last[1] === cls) last[0] += token;
    else groups.push([token, cls]);
  }
  for (const [text, cls] of groups) {
    const span = el("span");
    span.style.color = tokenColor(cls);
    span.textContent = text;
    line.appendChild(span);
  }
  return line;
}

// line_elements.rs window_flow: block containers only, the visible lines in
// normal flow between two spacers, rebuilt on scroll. CodeMirror 6's shape.
function windowFlow(lines, content, marginPx) {
  document.body.replaceChildren();
  const scroller = el("div",
    "height: 600px; overflow: auto; font-family: monospace; font-size: 13px; line-height: 18px");
  document.body.appendChild(scroller);
  const visible = Math.floor((VIEW_HEIGHT + 2 * marginPx) / LINE_HEIGHT) + 10;
  const renderWindow = (first) => {
    scroller.replaceChildren();
    const last = Math.min(first + visible, lines);
    const before = el("div");
    before.style.height = `${first * LINE_HEIGHT}px`;
    scroller.appendChild(before);
    for (let i = first; i < last; i++) scroller.appendChild(flowLine(i, content));
    const after = el("div");
    after.style.height = `${(lines - last) * LINE_HEIGHT}px`;
    scroller.appendChild(after);
  };
  const middle = Math.floor(lines / 2);
  // Counted as the probe counts them: the window's first `visible` lines.
  let inlineBoxes = 0;
  for (let n = 0; n < visible; n++) {
    inlineBoxes += content === "Plain" ? 1 : flowLine(n, content).childNodes.length;
  }
  return {
    inlineBoxes,
    build() { renderWindow(0); },
    edit() {
      const line = scroller.firstChild.nextSibling.nextSibling;
      scroller.insertBefore(flowLine(6, content), line);
      line.remove();
    },
    scroll() {
      renderWindow(middle);
      scroller.scrollTo(0, middle * LINE_HEIGHT);
    },
  };
}

// inline_cost.rs layout_time: `lines` lines, each with `spans` alternating
// "token" and " " spans. The tree is built off-document; the step attaches
// it, and its frame lays it out once.
function inlineCost(lines, spans, flex, pre, gutter) {
  document.body.replaceChildren();
  const outer = el("div",
    (flex ? "display: flex; flex-direction: column; height: 600px; " : "height: 600px; ") +
    "font-family: monospace; font-size: 13px; line-height: 18px");
  const scroller = el("div", "overflow: auto; height: 600px");
  outer.appendChild(scroller);
  for (let n = 0; n < lines; n++) {
    const line = el("div", pre ? "height: 18px; white-space: pre" : "height: 18px");
    if (gutter) {
      const g = el("span", "display: inline-block; width: 48px");
      g.textContent = String(n);
      line.appendChild(g);
    }
    for (let s = 0; s < spans; s++) {
      const span = el("span");
      span.textContent = s % 2 === 0 ? "token" : " ";
      line.appendChild(span);
    }
    scroller.appendChild(line);
  }
  return {
    build() { document.body.appendChild(outer); },
  };
}

// The runner calls bench.setup(kind, args) on a fresh page, then for each
// step bench.step(name) and bench.frames(), which waits for the frame that
// follows the step to be laid out, painted and rastered.
let current = null;
window.bench = {
  sourceLine,
  tokens,
  setup(kind, args) {
    const make = { allLines, windowed, windowFlow, inlineCost }[kind];
    current = make(...args);
    return { inlineBoxes: current.inlineBoxes ?? null };
  },
  // The marks bracket the mutation in the trace, so the runner can tell
  // style and layout the mutation forced (scrollTo does) from the frame's.
  step(name) {
    console.timeStamp("bench-step-start");
    const start = performance.now();
    current[name]();
    const ms = performance.now() - start;
    console.timeStamp("bench-step-end");
    return ms;
  },
  frames() {
    return new Promise((resolve) => {
      requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(resolve, 50)));
    });
  },
  // The probe hit-tests (200, 300) once. Chromium caches the last hit
  // test's point, so the mean is over distinct points along that row.
  hitTest(calls) {
    const start = performance.now();
    let hit = null;
    for (let i = 0; i < calls; i++) hit = document.elementFromPoint(200 + i * 0.5, 300);
    return { ms: (performance.now() - start) / calls, tag: hit && hit.tagName };
  },
};
