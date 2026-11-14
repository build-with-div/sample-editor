# Design: porting a code editor to Div

Phase 1 of the sample-editor dogfood. The guiding principle (from the user,
13 Nov 2026): samples port what popular npm packages do, and Div closes
each missing capability **by implementing the web platform feature the
package relies on, matched to Chromium's behaviour and pixels**, not with
app workarounds or Div-only primitives. Workarounds in this sample are
temporary scaffolding, labelled `// div-gap: <n>`.

So this design leads with the packages, then the platform features that
power them, then what Div has, what is missing, who owns it and how big it
is. Draft Div APIs for the missing features are in [rfc-text.md](rfc-text.md).

Phase 1 read Div at `565db46a` (build-with-div/div). References such as
`divui/src/document.rs:1588` are paths under Div's `crates/` at that commit.
Phase 2 pins Div `92ea25a0`, which adds only component `{expr}` children
over `565db46a`.

## 1. Packages and the platform features that power them

Editors professionals build on:

| Package | Kind | How it edits | How it renders |
| --- | --- | --- | --- |
| **CodeMirror 6** (`@codemirror/view`, `state`, `language`) | code | A `contenteditable` host (`.cm-content`); reads edits from `beforeinput` and from DOM mutations (its `DOMObserver`: `MutationObserver` + `selectionchange`) | Only the viewport's lines (plus a ~1000px margin) as `.cm-line` divs with token spans; gaps as tall spacers; selection and cursors drawn as absolutely positioned layers; styles injected as stylesheets |
| **Monaco** (VS Code's editor) | code | A hidden `<textarea>` placed at the caret (or `EditContext` in newer builds) | Absolutely positioned `.view-line` divs with a span per token; layers with `contain: strict` and transforms; minimap on a 2D canvas |
| **ProseMirror** / **TipTap** (on ProseMirror) | rich text | A `contenteditable` host; reads DOM mutations and selection, maps them to its document model | The document model rendered as ordinary DOM |
| **Lexical** | rich text | A `contenteditable` host driven mostly by `beforeinput` (`inputType`, `getTargetRanges()`); reverts unexpected mutations seen by `MutationObserver` | Ordinary DOM from its node tree |

The web platform features they depend on:

| # | Platform feature | CM6 | Monaco | ProseMirror/TipTap | Lexical |
| --- | --- | --- | --- | --- | --- |
| 2 | `contenteditable` editing host (focus, caret, native editing, `plaintext-only`) | ● | | ● | ● |
| 3 | `beforeinput`/`input` with the full `inputType` set, `getTargetRanges()` | ● | ○ | ● | ● |
| 4 | Selection API (`getSelection`, `setBaseAndExtent`, `selectionchange`), native selection painting, `::selection`, `caret-color`, `user-select` | ● | ○ | ● | ● |
| 5 | Range API geometry (`createRange`, `getClientRects`, `getBoundingClientRect`) | ● | ● | ● | ● |
| 6 | `caretPositionFromPoint` / `caretRangeFromPoint` (and `elementFromPoint`) | ● | ● | ● | ○ |
| 7 | Composition events and IME (preedit, candidate window at the caret) | ● | ● | ● | ● |
| 8 | Clipboard events with `clipboardData` (`DataTransfer`: `text/plain`, `text/html`, custom types) | ● | ● | ● | ● |
| 9 | Drag and drop (`dragstart`/`dragover`/`drop`, `DataTransfer`) | ● | ● | ● | ● |
| 10 | `MutationObserver` | ● | | ● | ● |
| 11 | `ResizeObserver` | ● | ● | | |
| 12 | `IntersectionObserver` | ● | | | |
| 13 | `requestAnimationFrame` | ● | ● | ● | ● |
| 14 | Stylesheets and selectors (`<style>`, classes, `:focus`, `::selection`, custom properties, `@keyframes`) | ● | ● | ● | ● |
| 15 | UI Events completeness: `MouseEvent.detail`, `dblclick`, `contextmenu`, `auxclick`, focus on press, Pointer Events, `KeyboardEvent.code` | ● | ● | ● | ● |
| 16 | Fast DOM line rendering: layout that scales with what changed | ● | ● | ● | ● |
| 17 | Accessible editable text (UIA TextPattern, AT-SPI Text) | ● | ● | ● | ● |
| 18 | Canvas 2D and `measureText` | | ● (minimap) | | |
| 19 | `document.fonts` (`FontFaceSet.ready`, `loadingdone`) | ● | ● | | |
| 20 | `EditContext` | | ● (newer) | | |
| 21 | `<textarea>` editing and rendering parity (Monaco's hidden input) | | ● | | |
| — | `white-space: pre`/`pre-wrap`, `tab-size`, font metrics, `scrollTop`, `scrollIntoView`, `getBoundingClientRect`, `focus()` | ● | ● | ● | ● |

● relies on it; ○ uses it in part. The columns come from these packages'
documented architecture. Phase 2 step 1 pins exact versions and records each
API call from their published source, so the matrix becomes a checked list
(section 8).

## 2. The Div work list

The owners below follow the platform decisions the user accepted on
13 Nov 2026 (`platform-plan/decisions-2026-11-13.md`): this sample's
`Range`/`Selection`/`StaticRange` model is adopted for the engine
(decision 6), `contenteditable` comes before `EditContext` (decision 11),
one `DataTransfer` serves clipboard and drag (decision 7), and runtime CSS
is ingestion of third-party sheets and runtime custom properties now, with
runtime sheet insertion later (decision 2).

This is the platform Div must implement faithfully for these packages. Each
row has a Chromium oracle and a way to verify it: Div's comparison harness
(pinned Chromium 147), WPT, or both. Owner tracks: **engine** (div-style,
div-layout, div-paint, div-text, div-engine), **framework** (divui,
div-platform, div-macros, div-cli), **components** (div-components). Size:
**S** about a week or less, **M** a few weeks, **L** more, for one owner.

| # | Platform feature | Div at `565db46a` | Owner | Size | Verify with |
| --- | --- | --- | --- | --- | --- |
| 1 | (packaging) a fresh app lockfile builds on Linux | **Missing**: zbus pins incomplete (sample-git gap 24) | framework | S | CI of a fresh `div new` app |
| 2 | `contenteditable` editing host | **Missing**: the attribute only affects an empty block's caret line (`div-layout/src/block.rs:5107`); probe `contenteditable_host`: not focusable without `tabindex`, and typing fires no `beforeinput` and changes nothing | engine (editing, layout, paint); framework | L | WPT `editing/`, `contenteditable`; harness screenshots of caret and selection |
| 3 | `beforeinput`/`input`, full `inputType`, `getTargetRanges()` | **Partial**: only on `<input>`/`<textarea>` (`divui/src/document.rs:1588`); 10 input types (`divui/src/events.rs:10-21`); the commit after a composition has none (`document.rs:1673,1692`) | framework (events); engine | M | WPT `input-events/` |
| 4 | Selection API, selection painting, `::selection`, `caret-color`, `user-select` | **Missing** for documents; only form-control selection with `select`/`selectionchange` on the control (`document.rs:464`) | engine (decision 6) | L | WPT `selection/`; harness screenshots |
| 5 | Range API geometry | **Missing**: element boxes only (`divui/src/element.rs:807,815`); div-text has the primitives but they are internal (`div-text/src/shaping/shape_result.rs:298,312,354`) | engine (decision 6) | M | WPT `dom/ranges/`, `css/cssom-view/` |
| 6 | `caretPositionFromPoint` / `caretRangeFromPoint` | **Missing**; `Document::hit_test` (`elementFromPoint`) exists (`document.rs:861`) | engine (decision 6) | M | WPT `css/cssom-view/caretPositionFromPoint*` |
| 7 | Composition events and IME | **Partial**: composition events only on form controls (`document.rs:1629`); the preedit cursor is dropped (`div-platform/src/desktop.rs:687`); `set_ime_cursor_area` is never called, so the candidate window cannot follow the caret; no `input` during composition | framework (platform, events) | M | WPT `uievents/`, scripted IME in Chromium via CDP `Input.imeSetComposition`; native manual check |
| 8 | Clipboard events and `DataTransfer` (one type with 9, decision 7) | **Missing**: no `copy`/`cut`/`paste` events; the clipboard is plain text only (`divui/src/clipboard.rs:35-59`, `div-platform/src/windows_clipboard.rs`) | framework, with the engine | M | WPT `clipboard-apis/` |
| 9 | Drag and drop (the same `DataTransfer`; files as `File` wrapping paths) | **Missing** | framework, with the engine | M | WPT `html/editing/dnd/` |
| 10 | `MutationObserver` | **Missing** | engine (DOM); framework | M | WPT `dom/nodes/MutationObserver-*` |
| 11 | `ResizeObserver` (one API, three boxes; Tier 0) | **Missing** | engine (layout); framework | S–M | WPT `resize-observer/` |
| 12 | `IntersectionObserver` | **Missing** | engine (layout); framework | M | WPT `intersection-observer/` |
| 13 | `requestAnimationFrame` (per document, typed handle; Tier 0) | **Missing**; only timers (`divui/src/task.rs:157-178`) and the platform redraw loop | framework | S | WPT `html/webappapis/update-rendering/` |
| 14 | 14a stylesheets, selectors, pseudo-classes and pseudo-elements, custom properties, `@keyframes` (Tier 0), with the browser-compatible ingestion mode for third-party sheets and runtime custom properties (decision 2); 14b runtime `<style>` element text, which style-mod uses for a document root; 14c `insertRule` and adopted sheets, for shadow roots (decision 2's later step) | **Missing** (inline styles only); 14a in progress on the engine track | engine (style); framework (14b/14c API) | L + S–M + M | harness, WPT `css/`, `css/cssom/` |
| 15 | UI Events completeness | **Partial**: no `detail`/`dblclick`; a right press fires `click` and no `contextmenu`/`auxclick`; a press on a child does not focus the focusable ancestor; no `pointer*` from headless input; `Key` has no `Insert`, `code` or modifier keys (`divui/src/keyboard.rs:11-33`) | framework (events) | S–M | WPT `uievents/`, `pointerevents/` |
| 16 | Fast DOM line rendering | **Too slow, measured** (section 4): ~0.2 ms per inline box per layout; whole-document relayout and repaint per change (`div-engine/src/lib.rs:2605-2618,2659-2665`); no shaping cache (`div-layout/src/inline/items_builder.rs:290-312`); nested flex ~2.7x per level. Chromium 147 on the same machine: ~1.4 µs per box, flex at block cost, and the CM6 edit in 1.79 ms against Div's 272 ms (section 4) | engine (layout, text) | L | the probes here, against the Chromium timings of `fidelity/bench` |
| 17 | Accessible editable text | **Partial**: AccessKit gives UIA TextPattern / AT-SPI Text to text inputs, but Div sets the selection on the run, not the control (`div-engine/src/accessibility.rs:761` vs `:819-831`); runs have no character positions or word starts (`:745-775`); `contenteditable` content is not exposed as editable text | engine (accessibility) | M | Chromium's accessibility tree for the same page (CDP `Accessibility.getFullAXTree`); manual NVDA/Orca |
| 18 | Canvas 2D, `measureText` | **Missing** (`<canvas>` is a placeholder, `div-paint/src/painter.rs:6331`); `sample-canvas` is designing it | engine (paint, text); framework | L | harness |
| 19 | `document.fonts` | **Missing**: `Document::register_font_face` exists (`document.rs:712`), with no load events | engine (text); framework | S | WPT `css/css-font-loading/` |
| 20 | `EditContext` (after 2, decision 11) | **Missing** | framework (platform, events); engine | L | WPT `editing/edit-context/` |
| 21 | `<textarea>` parity | **Partial**: no caret, selection or composition paint (`div-paint/src/painter.rs:3816-3852`), content does not scroll, Up/Down do nothing, undo puts the caret at the end, O(document) work per keystroke (section 4) | engine (forms, paint) | M | harness, WPT `html/semantics/forms/the-textarea-element/` |

Priority for the code editor, CM6 first: **16** (without it the DOM
approach cannot be interactive), **2, 3, 4, 5, 6** (the editing host and
selection geometry), **14** (CM6 is styled entirely by injected
stylesheets), **7, 8, 10, 11, 13**, then **15, 17**. Monaco additionally
needs **21** (or **20**) and **18**. Rich text (ProseMirror, Lexical)
needs the same plus **9** and `text/html` in **8**.

## 3. What Div has today, in more detail

### Text (engine)

Shaping is HarfBuzz through Skia's SkShaper with per-run font fallback
(`div-text/src/shaping/shaper.rs:506,612-615`). It also does UAX#9 bidi,
OpenType features from `font-variant-ligatures`/`font-feature-settings`
(`div-text/src/font/features.rs:78-146`), `tab-size` and every `white-space`
value. In `pre` text, layout rewrites tabs as spaces
(`div-layout/src/inline/items_builder.rs:1232-1246`). A Range offset map
(5, 6) must count source tabs. `ShapeResult` offsets are Unicode scalar
indices, and `offset_for_x_position` scans linearly (`shape_result.rs:322-334`).

### Form-control editing (engine, framework)

`<input>`/`<textarea>` keep a `String`, a UTF-8 byte selection and
full-copy undo snapshots (`div-engine/src/interaction.rs:142-158,240-251`).
Each insertion copies the value into the `value` attribute and marks
layout dirty (`:793-811`). Keys map to character and word motions,
logical-line Home/End, select all, undo/redo and cut/copy/paste
(`divui/src/document.rs:1328-1440`). A press only focuses its target
(`document.rs:1111-1113`). There is no click-to-caret, drag selection or
double/triple click.

### Input (framework)

`Event::key()` reports a `Key`, and `Event::repeat` comes from Div #27.
`keydown` reaches any focused element, and `preventDefault` keeps Tab
(`document.rs:1294`). IME comes from winit 0.30.13
(`div-platform/src/desktop.rs:657-708`); raw `PlatformEvent`s reach
`App::on_platform_event` after the document (`divui/src/app.rs:198`).

### Accessibility (engine)

AccessKit 0.25 (`accesskit_windows` 0.35, `accesskit_unix` 0.23). A
textarea is a `MultilineTextInput` with one synthetic `TextRun`
(`div-engine/src/accessibility.rs:745-775`). App code can set the role,
label, description, value, live region, hidden flag and relations
(`divui/src/element.rs:750-801`). Every invalidation rebuilds the whole
semantic tree (`div-engine/src/lib.rs:2908-2911`).

## 4. Measurements

**Conditions.** Windows 11, 24 cores, 64 GB RAM. Headless `TestApp` with
CPU Skia raster, 800x600 CSS px at scale 1. `cargo test` dev profile: Div
and its dependencies at `opt-level = 2`, the probes unoptimized. Other
sessions' builds shared the machine, so where a case ran twice both runs
are shown. Treat the numbers as indicative.

**Command.**

```bash
PROBE_SCALE=full cargo test -p editor-probes -- --nocapture --test-threads=1
```

**Inputs.** Lines are Rust-like source of 5–55 bytes, monospace 13px with
an 18px line height. "Layout" is `Document::update_all`. "Paint" is
`render_at`, which records and rasters one frame.

### DOM line rendering (feature 16)

**CM6-shaped viewport.** Only the window's lines are in the document, in
normal flow between two spacers, in a block-only container. This is the
structure CM6 uses ([line_elements.rs](../probes/tests/line_elements.rs),
`window_flow_cost`). The document has 100,000 lines.

| Lines in the DOM | Inline boxes | Layout | Edit one line + layout + paint | Scroll to middle + paint |
| --- | --- | --- | --- | --- |
| Visible only (43), plain text | 43 | 10.6–13.7 ms | 12.0–21.0 ms | 17.4–21.8 ms |
| Visible only, same-colour tokens merged | 249 | 49.9–79.1 ms | 54.4–70.9 ms | 80.1–94.8 ms |
| Visible only, a span per token | 555 | 98.1–161 ms | 109–151 ms | 152–210 ms |
| Visible + 1000px margin (CM6's default), a span per token | 1,921 | 340 ms | 272 ms | 454 ms |

The same 43-line window, but with lines positioned absolutely in a flex
shell (Monaco's structure):

| Document lines | Layout | Edit one line + layout + paint | Scroll to middle + paint |
| --- | --- | --- | --- |
| 10,000 | 867 ms | 883 ms | 1.09 s |
| 100,000 | 887 ms | 887 ms | 1.02 s |

Every line in the DOM (no viewport), flex shell, 9–17 spans per line:

| Lines | Layout | Edit one line + layout + paint |
| --- | --- | --- |
| 200 | 2.10 s | 2.16 s |
| 500 | 5.70 s | 6.87 s |
| 2,000 | 27.3 s | 29.1 s |
| 200 inside 3 more nested flex columns | 40.6 s | 42.1 s |

An earlier 2,000-line run reached 6.3 GB of working set. Its 10,000-line
case was stopped after 10 minutes.

**Where the time goes** ([inline_cost.rs](../probes/tests/inline_cost.rs)).
Each case is a fresh document laid out once, with `white-space: pre` lines.

| 40 lines × spans per line | 1 | 4 | 8 | 16 | 32 |
| --- | --- | --- | --- | --- | --- |
| Block container | 12.6 ms | 35.7 ms | 65.9 ms | 127 ms | 266 ms |
| Flex column container | 28.3 ms | 101 ms | 182 ms | 363 ms | 789 ms |

| Lines × 8 spans | 10 | 40 | 160 | 640 |
| --- | --- | --- | --- | --- |
| Block | 13.4 ms | 72.5 ms | 280 ms | 1.27 s |
| Flex column | 44.5 ms | 196 ms | 710 ms | 3.28 s |

Layout is linear at about **0.2 ms per inline box** in a block container,
and about 2.7x that in a flex column. The inline-block gutter adds little.

**div-text, called directly** ([shaping.rs](../probes/tests/shaping.rs);
engine-internal, not public API). Each figure is the mean of 2,000 calls:

| Call | Per call |
| --- | --- |
| Shape a ~45-character line | 23.3 µs |
| Shape a 5-character token | 13.0 µs |
| `Font::new` (monospace 13px) | 33.2 µs |
| `TextShaper::new` | 0.3 µs |
| Caret x for an offset, offset for an x | < 0.1 µs |

So shaping a CM6 viewport's ~150 lines costs a few milliseconds. Most of
the 0.2 ms per box is elsewhere in layout. This is a hypothesis for the
engine track to profile, not measured here:
- a font is resolved per text item (`items_builder.rs:291-300`);
- each item is shaped twice, once for intrinsic sizing and once for layout;
- an update can run `block_layout` up to three times
  (`div-engine/src/lib.rs:2602-2648`).

**What feature 16 must reach.** A CM6 viewport of about 2,000 inline boxes
must take one edit plus layout plus paint within a 16.7 ms frame. Today
that is 272 ms. Getting there needs:
- per-box cost about 20x lower;
- relayout limited to the edited line, as LayoutNG caches it.

The probe is the benchmark. The Chromium reference below runs the same
cases in pinned Chromium on the same machine. There, the same edit takes
1.79 ms.

### Chromium reference for the line-rendering probes (M5)

**Conditions.**
- Browser: Chrome for Testing 147.0.7727.50, headless with
  `--disable-gpu`, so compositing and raster run in software.
- Machine and OS: the same machine as above, under WSL2 (Linux 6.18, 24
  CPUs).
- Viewport: 800x600 CSS px at scale 1.
- Font: `monospace` resolves to DejaVu Sans Mono through Div's real-font
  fontconfig profile. Div resolves `monospace` to Consolas on Windows.

So the OS and the font differ from the Div runs, and the ratios below are
indicative.

**Runs and timing.** Each case runs on a fresh page. One warm-up pass over
every case is discarded, then each case runs five times. The tables give
the median, with the range over the runs in brackets.

The DOM mutation is timed in the page. The frame after it comes from
Chromium's trace:
- style + layout: `UpdateLayoutTree` and `Layout`;
- paint + raster: `PrePaint`, `Paint`, `Layerize` and `Commit`, plus
  `RasterTask` CPU time summed over the raster threads.

The tracing overhead is included.

**Sources.**
- Page: [lines.js](../fidelity/bench/lines.js), a port of `line_elements.rs`
  and `inline_cost.rs` with the same lines, tokens, colours, structure and
  steps. The window cases count the same inline boxes as the probe.
- Runner: [run.py](../fidelity/bench/run.py).
- Every run's numbers:
  [chromium-147.json](../fidelity/bench/results/chromium-147.json).

```bash
python3 fidelity/bench/run.py --div /path/to/div --out fidelity/bench/results/chromium-147.json
```

**CM6-shaped viewport** (`window_flow_cost`, 100,000 lines). The Div
column repeats the table above.

| Lines in the DOM | Inline boxes | Style + layout | Edit one line + layout + paint | Scroll to middle + paint | Div: edit one line + layout + paint |
| --- | --- | --- | --- | --- | --- |
| Visible only (43), plain text | 43 | 0.30 ms (0.27–0.57) | 0.74 ms (0.69–0.86) | 2.66 ms (2.40–2.94) | 12.0–21.0 ms |
| Visible only, same-colour tokens merged | 249 | 0.58 ms (0.51–0.68) | 1.07 ms (0.90–1.24) | 3.75 ms (3.66–4.41) | 54.4–70.9 ms |
| Visible only, a span per token | 555 | 1.21 ms (0.92–1.68) | 1.15 ms (1.01–1.45) | 5.99 ms (4.97–6.14) | 109–151 ms |
| Visible + 1000px margin (CM6's default), a span per token | 1,921 | 2.78 ms (2.66–3.29) | 1.79 ms (1.67–2.29) | 10.7 ms (10.5–11.8) | 272 ms |

Paint + raster of the first frame was 1.47–3.69 ms (medians) across these
four cases.

**Monaco-shaped window** (`windowed`): the 43 lines are absolutely
positioned in a flex shell.

| Document lines | Style + layout | Edit one line + layout + paint | Scroll to middle + paint | Div: edit one line + layout + paint |
| --- | --- | --- | --- | --- |
| 10,000 | 1.09 ms (1.01–1.28) | 1.33 ms (1.25–2.20) | 6.16 ms (6.04–8.60) | 883 ms |
| 100,000 | 1.10 ms (1.00–1.37) | 1.32 ms (1.17–1.93) | 6.56 ms (5.89–7.75) | 887 ms |

**Every line in the DOM** (`all_lines`, flex shell):

| Lines | Style + layout | Edit one line + layout + paint | Insert at top + layout + paint | Div: layout |
| --- | --- | --- | --- | --- |
| 200 | 4.07 ms (3.98–5.32) | 2.14 ms (1.68–2.75) | 5.59 ms (5.08–7.25) | 2.10 s |
| 500 | 11.1 ms (9.67–16.3) | 2.47 ms (2.10–2.93) | 7.74 ms (7.33–8.32) | 5.70 s |
| 2,000 | 47.2 ms (39.6–61.4) | 5.63 ms (5.43–5.84) | 17.0 ms (15.6–17.9) | 27.3 s |
| 200 inside 3 more nested flex columns | 4.09 ms (3.88–8.83) | 1.91 ms (1.76–1.99) | 5.85 ms (5.44–6.74) | 40.6 s |

**Hit testing.** `elementFromPoint` took 3–17 µs per call across all runs
of the `windowed` and `all_lines` cases. Each figure is the mean of 100
calls at distinct points along y = 300, because Chromium caches a repeated
point.

**Where the time goes** (`inline_cost`, a fresh document, style + layout):

| 40 lines × spans per line | 1 | 4 | 8 | 16 | 32 |
| --- | --- | --- | --- | --- | --- |
| Block container | 0.26 ms | 0.41 ms | 0.59 ms | 0.96 ms | 1.82 ms |
| Flex column container | 0.27 ms | 0.40 ms | 0.57 ms | 0.94 ms | 1.62 ms |

| Lines × 8 spans | 10 | 40 | 160 | 640 |
| --- | --- | --- | --- | --- |
| Block | 0.23 ms | 0.59 ms | 1.93 ms | 7.14 ms |
| Flex column | 0.24 ms | 0.57 ms | 1.98 ms | 7.26 ms |

**What this tells the engine track.** Three measured gaps:
- **Per-box cost.** Between 160 and 640 lines of 8 spans (3,840 more
  inline boxes), Chromium adds about 1.4 µs per box
  ((7.14 − 1.93) ms / 3,840). Div, measured the same way, adds about
  260 µs per box ((1.27 s − 280 ms) / 3,840), roughly 190x more.
- **Flex.** A flex column costs Chromium the same as a block container.
  It costs Div about 2.7x.
- **Incremental edits.** Chromium's edit step is cheaper than laying out
  the window from scratch: 1.79 ms against 2.78 ms at 1,921 boxes. With
  every line in the DOM, the edit costs 5.63 ms against 47.2 ms for the
  full layout at 2,000 lines. From 200 lines to 2,000, the edit grows from
  2.14 ms to 5.63 ms while the full layout grows from 4.07 ms to 47.2 ms.
  Div re-lays out and repaints everything on each edit, so its edit costs
  about as much as a full layout (272 ms against 340 ms; 2.16 s against
  2.10 s).

At CM6's default viewport, Div's edit (272 ms) is about 150x Chromium's
(1.79 ms).

### `<textarea>` with a large document (feature 21) ([textarea.rs](../probes/tests/textarea.rs))

| Lines (bytes) | Mount + layout | First paint | One keystroke | Paint after it |
| --- | --- | --- | --- | --- |
| 1,000 (29.5 KB) | 39 ms | 7.5 ms | 26 ms | 6.1 ms |
| 10,000 (301 KB) | 291 ms | 20 ms | 300 ms | 22 ms |
| 100,000 (3.07 MB) | 2.55 s | 171 ms | 2.79 s | 182 ms |

Enter, Backspace, Ctrl+Z and ArrowRight cost the same as a typed key.
The cost grows linearly with document size.

### Events ([input_events.rs](../probes/tests/input_events.rs))

| Signal | `contenteditable` / focusable `div` | `<textarea>` |
| --- | --- | --- |
| Focus | `contenteditable` without `tabindex`: `focus()` → `NotFocusable` (Chromium: focusable) | yes |
| `keydown`/`keyup` with `Key`, modifiers, `repeat` | yes | yes |
| Typing edits the content | **no** (no editing host) | yes |
| `beforeinput`/`input` | **no** | yes, `inputType=insertText` |
| Composition events | **no** | yes; preedit written into the value |
| `input` after a composition commit | — | yes, **no `inputType`** |
| A press on a child focuses the focusable ancestor | **no** (Chromium: yes) | — |
| Second click at the same point | `click` again; **no `dblclick`**, no `detail` | — |
| Right-button press | **fires `click`**, no `contextmenu` (Chromium: `auxclick`, `contextmenu`) | — |
| Accessibility | — | selection not on the control node; text run without positions |

## 5. Fidelity plan: the package in Chromium against the port in Div

The sample is a Rust port of CodeMirror 6's view and state layers. Div runs
no JavaScript, so this port is what "porting the package" means here. The
port keeps CM6's DOM: the same elements, classes and stylesheet rules for
the same editor state. That makes the original package in Chromium the
oracle.

1. **Reference capture (Chromium 147, WSL, Div's harness tooling).** A small
   page loads the pinned CM6 with a fixed theme and font (Geist Mono,
   registered on both sides). It also fixes the document text, viewport
   size, scroll position and selection. Capture three things:
   - a screenshot;
   - the editor's `outerHTML` plus its stylesheet text;
   - the accessibility tree.
2. **Static fidelity (no editing).** The captured HTML and CSS becomes a
   test page in Div's comparison harness. Div renders it, and the result
   is diffed with Chromium's pixels. This works without the sample.
   It needs 14 (stylesheets) and nothing editor-specific. It turns "renders
   like CM6" into an exactness gate the engine track can own.
3. **Structural fidelity.** For the same state, the port's DOM is
   serialized and compared node by node with CM6's `outerHTML`, ignoring
   generated ids.
4. **Behavioural fidelity.** Scripted input sequences run on both sides:
   - keys, text, IME composition, and clicks and drags at coordinates;
   - Chromium is driven through CDP (`Input.dispatchKeyEvent`,
     `Input.insertText`, `Input.imeSetComposition`,
     `Input.dispatchMouseEvent`);
   - Div is driven through `TestApp` and `Document::dispatch_*`.

   The resulting document text, selection, DOM and frame are compared
   after each step.
5. **WPT.** Each feature in section 2 names its WPT directory. These join
   Div's coverage suites, as `css-grid` did, so progress is measured
   rather than claimed.

Monaco follows the same plan with its `.view-lines` DOM. ProseMirror and
Lexical follow it later in a rich-text sample.

## 6. The sample's architecture (port of CM6)

```
editor-state  (port of @codemirror/state; no Div dependency)
  Text (CM6's line tree) · ChangeSet · EditorSelection (multiple ranges)
  Facet/StateField · Transaction · RangeSet (decorations)
  History (undo with selection restore, from @codemirror/commands)
      │ ViewUpdate
      ▼
editor-view   (port of @codemirror/view, on Div's DOM)
  .cm-editor > .cm-scroller > .cm-gutters, .cm-content[contenteditable]
  viewport = visible + margin; .cm-line rows; gap spacers
  input: beforeinput + composition on the editing host; DOMObserver
         (MutationObserver + selectionchange) for native edits
  geometry: Range.getClientRects (coordsAtPos), caretPositionFromPoint (posAtCoords)
  layers: .cm-selectionLayer, .cm-cursorLayer (absolute), cursor blink by @keyframes
  measure cycle: requestAnimationFrame, ResizeObserver, IntersectionObserver
  styles: injected stylesheets (style-mod) and theme classes
```

The public Rust API keeps CM6's shape with Rust types. The web semantics
stay underneath:

```rust
use sample_editor::{EditorState, EditorView, extensions::*};

let state = EditorState::create(EditorStateConfig {
    doc: text.into(),
    extensions: vec![line_numbers(), history(), rust_language(), keymap(default_keymap())],
});
let view = EditorView::new(&document, state);   // owns the .cm-editor element
parent.append_child(view.dom())?;
view.dispatch(view.state().update(TransactionSpec::insert(0, "// header\n")));
let pos = view.pos_at_coords(x, y);             // caretPositionFromPoint underneath
let rect = view.coords_at_pos(pos);             // Range.getClientRects underneath
```

## 7. Temporary scaffolding in the sample

These workarounds are allowed only so the port can be built and measured
while Div implements the features. Each is labelled in code with
`// div-gap: <n>`. None of them is the solution.

| Workaround | Gap | Removed by |
| --- | --- | --- |
| Seed `Cargo.lock` from Div's lock | 1 | Div pinning its zbus crates |
| A visually hidden `<textarea>` receives text and IME in place of the `contenteditable` host | 2, 3, 7 | `contenteditable` with `beforeinput`/composition events |
| Selection and caret drawn by the port's layers, with no native selection | 4 | Selection API and selection painting |
| Caret and hit-test geometry from a measured monospace cell: x = column × cell. Wrong for wide characters, emoji, ligatures, proportional fonts and bidi | 5, 6 | `Range.getClientRects`, `caretPositionFromPoint` |
| Inline styles generated from CM6's theme rules | 14 | 14a stylesheets with the ingestion mode, then 14b: style-mod's `<style>` text used as is |
| Viewport recomputed from `scroll` events and timers | 11, 12, 13 | `ResizeObserver`, `IntersectionObserver`, `requestAnimationFrame` |
| The editor kept in a block-only subtree; tokens of the same colour merged into one span | 16 | Fast DOM line rendering |
| Double and triple click timed by the app | 15 | `MouseEvent.detail`, `dblclick` |

## 8. Phase 2 plan

The parent approved Phase 2 on 13 Nov 2026, alongside Div's Tier 0
foundations (stylesheets and selectors, the UA stylesheet, custom
properties, the per-box cost and shaping cache, damage tracking,
`requestAnimationFrame`, `ResizeObserver`, Pointer Events). Selection,
Range, `contenteditable` and IME (features 2–7) are not scheduled yet, so
the port starts with the layers that run on current Div. Each milestone is
one focused PR, merged before the next. Upstream: `@codemirror/state`
6.7.6, `@codemirror/view` 6.43.15, `@codemirror/commands` 6.11.1 (npm,
13 Nov 2026).

| # | Milestone | Upstream | Needs from Div |
| --- | --- | --- | --- |
| M1 | `Text`, `ChangeSet`, `EditorSelection` with upstream tests; pin Div `92ea25a0` | state `text.ts`, `change.ts`, `selection.ts` | nothing |
| M2 | Facets, state fields, compartments, `EditorState`, transactions, annotations and effects, change filters; character helpers | state `facet.ts`, `state.ts`, `transaction.ts`, `char.ts`, `column.ts`, `charcategory.ts` | nothing |
| M3 | `RangeSet` and `RangeValue`: the storage for decorations, gutter markers and folds | state `rangeset.ts` | nothing |
| M4 | History and the default commands and keymap, on state alone | commands | nothing |
| M5 | Fidelity tooling: the pinned packages, the reference-capture page and the line-rendering benchmark in Chromium 147 (section 5 step 1, section 4) | view, state | Div's harness tooling (Chromium in WSL) |
| M6 | View, static: `.cm-editor`, `.cm-scroller`, `.cm-content`, `.cm-line`, the viewport with gap spacers, mark/widget/line/replace decorations, gutters and line numbers, the theme; structural and pixel diffs against M5's captures | view `docview`, `viewport`, `decoration`, `gutter`, `theme` | 14a (Tier 0 stylesheets) and 14b (runtime `<style>` text), with inline styles as `div-gap: 14` until then; 16 (Tier 0 per-box cost) |
| M7 | View, interactive on labelled scaffolding: cursor and selection layers, `posAtCoords`/`coordsAtPos`, keymap through `w3c-keyname`, text through the input proxy, scrolling, the measure cycle | view `input`, `cursor`, `layer`, `keymap` | 2–7 (`div-gap` scaffolding), 11 and 13 (Tier 0) |
| M8 | Language and features: highlighting, search, bracket matching, multiple selections, folding, each with behavioural fidelity | language, search, autocomplete | as M7 |

Progress:
- M1 merged in sample-editor#2.
- M2 merged in sample-editor#4. It ports `facet.ts`, `state.ts`,
  `extension.ts`, `transaction.ts`, `char.ts`, `column.ts` and
  `charcategory.ts`, with `test-facet.ts`,
  `test-state.ts` and `test-charcategory.ts`, and `ChangeSet.filter`,
  left out of M1. `findClusterBreak` comes from `@marijn/find-cluster-break`
  1.0.4, ported as a module of `editor-state` with its test. Two parts are
  not ported yet: `EditorState.toJSON`/`fromJSON` with the field
  serializers (upstream's JSON test is skipped until a feature needs them;
  `Text`, `ChangeSet` and `EditorSelection` JSON are ported), and
  `combineConfig`, which merges untyped config objects; the packages that
  use it get typed config structs when they are ported.
- M3 merged in sample-editor#5. It ports `rangeset.ts` with `test-rangeset.ts`. `RangeValue` is a trait
  with upstream's defaults; values are shared as `Arc`s and compared by
  identity, then by `RangeValue::range_eq`. `RangeSet.compare` and
  `RangeSet.spans` take trait objects for the comparator and span
  iterator; a comparator opts into `boundChange` through
  `wants_bound_changes`. Upstream's "ignores identical sub-nodes" test also
  makes reading the first chunk's values throw, to prove the comparison
  skips the shared chunk; Rust has no property traps, so the port checks
  the reported ranges only.
- M4, first part (merged in sample-editor#6): the undo history (`history.ts` with `test-history.ts`)
  in a new `editor-commands` crate. `historyKeymap` and the `historyUndo`/
  `historyRedo` input handler wait for the view; the history JSON waits
  for `EditorState` JSON. The editing commands (`commands.ts`,
  `comment.ts`) follow in a separate PR.
- M4, second part (merged in sample-editor#7): the 20 commands of `commands.ts` that need only the
  state (logical character motion, document and line selection, line
  moving and copying, `transposeChars`, `splitLine`, `insertNewline`,
  `insertNewlineKeepIndent`, `deleteTrailingWhitespace`), with the 17
  upstream test cases that cover them and port-specific cases for the
  rest. The other commands and the keymaps need the view (M6, M7) or
  `@codemirror/language` (M8) and come with them; `deleteGroupForward`
  and `deleteGroupBackward` are typed as state commands upstream but
  attach the view's `announce` effect when deleting a selection, so they
  wait for the view too.
- M5, first part: the reference capture (section 5 step 1) in
  `fidelity/`. The pinned CM6 packages and Geist Mono build a page with
  four cases (`plain`, `gutter`, `decorations`, `scrolled`).
  `fidelity/capture.py` loads it in Chrome for Testing 147.0.7727.50
  (WSL) through Div's CDP helpers at `92ea25a0` and writes a screenshot,
  the editor's `outerHTML`, the stylesheet text, the accessibility tree
  and the case state to `fidelity/fixtures/reference-v1/`. Two further
  captures with `--check` reproduced all 20 files byte for byte.
- M5, second part: the line-rendering benchmark in Chromium,
  `fidelity/bench/`, with the results in section 4.
  - `lines.js` ports the `line_elements` and `inline_cost` probes.
  - `run.py` times each step in Chrome for Testing 147.0.7727.50 from
    Chromium's trace.
  - At CM6's default viewport (1,921 inline boxes), one edit plus layout
    plus paint takes 1.79 ms in Chromium against 272 ms in Div.
  - Per inline box, Chromium's layout costs about 1.4 µs against Div's
    260 µs, measured the same way.

Every milestone after M5 runs the fidelity checks in section 5 and reports
measured numbers. The highlighter (M8) follows CM6's design, a parse tree
mapped to `@lezer/highlight` tags; whether the tree comes from a port of
the Lezer runtime or from tree-sitter is decided at M8 and recorded here.
As each Div feature lands, its scaffolding is removed, the fidelity suite
re-runs and `docs/framework-gaps.md` records the change.

## 9. Alternatives considered

- **A Div-only text-view element** (Zed's GPUI model) or **a canvas-drawn
  editor**: both rejected. Neither is a web platform feature, so neither
  ports CM6, Monaco or ProseMirror. Neither has Chromium as an oracle.
  Each would need its own IME and accessibility plumbing. The measured
  performance gap belongs to feature 16, the engine's layout; it is not a
  reason for a separate primitive. div-text already shapes a CM6 viewport
  in a few milliseconds (section 4).
- **Fixing only `<textarea>`**: necessary for forms and Monaco's input
  path (21), but a textarea cannot render highlighted, decorated code.

## 10. Risks

- **Feature 16 is the critical path and is large.** Today a CM6-sized
  viewport costs 272 ms per edit. No amount of editing-API work makes the
  port interactive until layout gets about 20x cheaper per box and becomes
  incremental.
- **`contenteditable` and the Selection API are the largest and least
  regular parts of the web platform.** Matching Chromium needs WPT
  coverage from the start, plus the behavioural harness (section 5,
  step 4).
- **IME is untested natively.** Windows goes through winit's IMM32 path;
  TSF features such as reconversion are a further step.
- **Accessibility of `contenteditable` differs between browsers.** Div
  should match Chromium's tree, and record where screen readers still
  fall short.
- **The measurements are single or double runs on a shared machine**,
  headless, in the debug profile. Native frames add presentation time.
