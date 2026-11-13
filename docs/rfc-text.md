# RFC draft: text geometry, editing input and text measurement for editors

Status: **draft from the sample-editor investigation**, for the Div tracks
to accept, change or reject. Nothing here is implemented in Div. This
sample does not patch or fork Div.

Pinned Div: `565db46a`. Evidence: [design.md](design.md) and the probes in
[`probes/tests`](../probes/tests).

## Summary

A code editor (VS Code, Zed) or a rich-text editor needs six things from
the UI framework that Div does not expose today:

1. **Text geometry**: the caret rectangle for a text offset, the
   rectangles of a text range, and the text offset under a point.
2. **Editing input on any element**: committed text, IME composition
   with its caret and its clauses, and a way to place the IME candidate
   window, delivered to an editor surface that is not a form control.
3. **Range decoration without elements**: paint a selection, find matches
   and syntax colours over existing text without splitting it into spans.
4. **Layout that scales with what changed**, so a few hundred visible
   lines re-lay out in a frame when one line changes.
5. **Accessible text**: a screen reader can read the editor's text by
   character, word and line, and follow the caret and selection.
6. **Text measurement without a document**, for a canvas-drawn view such
   as a minimap.

Div's rule for gaps is that the fix is the web platform feature that
removes them, matched to Chromium. Each of 1–5 has a shipped web feature
that Chromium implements, so this RFC proposes those, in Rust form:

| Need | Web feature (Chromium) | Proposal | Track | Size |
| --- | --- | --- | --- | --- |
| 1 | `Range.getClientRects()`, `document.caretPositionFromPoint()` | [A](#a-text-ranges-and-caret-from-point) | engine (layout), framework (divui) | M |
| 2 | `EditContext` (Chromium 121) | [B](#b-editcontext) | framework (platform, divui); engine (focus) | L |
| 3 | CSS Custom Highlight API, `::highlight()` (Chromium 105) | [C](#c-custom-highlights) | engine (style, paint); framework | M |
| 4 | LayoutNG's cached, incremental layout; `content-visibility: auto` | [D](#d-incremental-layout) | engine (layout) | L |
| 5 | Accessibility of editable text (UIA TextPattern, AT-SPI Text) | [E](#e-accessible-text) | engine (accessibility) | M |
| 6 | Canvas `measureText()` (`TextMetrics`) | [F](#f-text-measurement) | engine (div-text); framework | M |

[G](#g-a-native-text-view-not-recommended-now) describes a Div-only
text-view element, the approach Zed's GPUI takes, and why this RFC does
not recommend it first.

Offsets in every Rust API below are **UTF-8 byte offsets** into the text
node's data, as `Element::selection` already uses. The web APIs count
UTF-16 code units; Div's FFI facade converts at its boundary. div-text's
`ShapeResult` counts Unicode scalar values today
(`div-text/src/shaping/shaper.rs:150`), so the engine converts once, inside.

---

## A. Text ranges and caret from point

**Today.** `Document::hit_test(x, y)` returns an `Element`
(`divui/src/document.rs:861`). `Element::client_rects` and `bounding_rect`
return element boxes (`divui/src/element.rs:807,815`). div-text has
`ShapeResult::x_position_for_offset`, `offset_for_x_position` and
`width_for_range` (`div-text/src/shaping/shape_result.rs:298,312,354`),
but layout does not keep a text-offset map on its line fragments that
app code can query, and divui re-exports none of it.

**Proposal** (`divui`, mirroring DOM `Range` and `CaretPosition`):

```rust
/// A DOM Range over text nodes and elements.
pub struct Range { /* start/end boundary points */ }

impl Document {
    pub fn create_range(&self) -> Range;
    /// `document.caretPositionFromPoint(x, y)`: the text node and offset
    /// nearest the viewport point, or `None` outside any text.
    pub fn caret_position_from_point(&self, x: f32, y: f32)
        -> Result<Option<CaretPosition>, Error>;
}

pub struct CaretPosition {
    pub node: TextNode,
    pub offset: usize, // UTF-8 bytes
}

impl Range {
    pub fn set_start(&self, node: &TextNode, offset: usize) -> Result<(), Error>;
    pub fn set_end(&self, node: &TextNode, offset: usize) -> Result<(), Error>;
    /// `Range.getClientRects()`: one rect per line box fragment, viewport
    /// coordinates. A collapsed range gives the caret rect (zero width,
    /// line-box height), as Chromium does.
    pub fn client_rects(&self) -> Result<Vec<Rect>, Error>;
    pub fn bounding_rect(&self) -> Result<Option<Rect>, Error>;
}
```

**Engine work.** Inline layout keeps, per text fragment, the source text
node, the byte range it covers and its `ShapeResult` (or enough of it to
answer x-for-offset). `caretPositionFromPoint` walks the hit-test result
to the line box and maps x with the existing `offset_for_x_position`,
which needs a binary search instead of today's linear scan
(`shape_result.rs:322-334`). Tab expansion rewrites tabs as spaces
(`div-layout/src/inline/items_builder.rs:1232-1246`, `expand_tabs` at
`:2154`); the offset map must count the source tab, not its spaces.

**Chromium parity tests.** `getClientRects` and `caretPositionFromPoint`
over LTR, RTL, mixed bidi, ligatures (a caret inside `fi`), tabs in
`white-space: pre`, wrapped lines, and an empty line.

## B. EditContext

**Today.** `beforeinput`, `input` and the composition events reach only
`<input>` and `<textarea>`: `Document::dispatch_text_input` and
`dispatch_composition_start` return early unless the focused element is an
editable control (`divui/src/document.rs:1588,1629`). The probe
`input_events::custom_surface_ime` confirms a focusable `div` receives
none of them. The platform drops the preedit caret range
(`Ime::Preedit(text, _)`, `div-platform/src/desktop.rs:687`), never calls
winit's `Window::set_ime_cursor_area`, so the candidate window cannot
follow the caret, and the commit `input` event has no `inputType`
(`document.rs:1673,1692`).

**Proposal** (`divui`, mirroring the W3C EditContext API):

```rust
pub struct EditContext { /* shared, Rc */ }

impl EditContext {
    pub fn new(document: &Document, init: EditContextInit) -> Self;
    // The editor's view of its text near the caret, kept in sync by the app.
    pub fn update_text(&self, range: std::ops::Range<usize>, text: &str);
    pub fn update_selection(&self, start: usize, end: usize);
    // Geometry the platform needs for the IME and accessibility.
    pub fn update_control_bounds(&self, bounds: Rect);
    pub fn update_selection_bounds(&self, bounds: Rect);
    pub fn update_character_bounds(&self, first: usize, bounds: &[Rect]);
    pub fn text(&self) -> String;
    pub fn selection(&self) -> (usize, usize);
}

impl Element {
    /// `element.editContext = ctx`. A focused element with an edit
    /// context receives keyboard input as text and IME edits.
    pub fn set_edit_context(&self, context: Option<&EditContext>) -> Result<(), Error>;
}
```

Events on the `EditContext` (via `on`, as `Element::on`):

- `textupdate`: `update_range`, `text`, `selection` (and composition range).
- `textformatupdate`: per-clause underline style and thickness, so the
  app draws IME clauses as the platform does.
- `characterboundsupdate`: the range whose character rects the IME
  wants; the app answers with `update_character_bounds`.
- `compositionstart`, `compositionend`.

`beforeinput` still fires on the element for non-text intents
(`deleteContentBackward`, `insertParagraph`, `historyUndo`), with
`inputType`, so Backspace, Enter and Ctrl+Z map to editor commands
through one path on every platform.

**Platform work.** Keep the preedit cursor; translate selection bounds to
`set_ime_cursor_area` on each update; send `characterboundsupdate`
requests. On Windows, winit 0.30 drives IMM32; reconversion and
TSF-based input (and the text services that read the document) need TSF,
which is a later step (L by itself).

**Why not `contenteditable`.** It makes the engine own the document model,
which a code editor cannot accept for a 100k-line file, and its
behaviour is the hardest part of the web platform to match. EditContext
is the API Chromium added so that editors (VS Code's Monaco has an
EditContext mode) keep their own model and view. A rich-text editor built
on EditContext needs the same primitives as a code editor.

## C. Custom highlights

**Today.** Selection and syntax colours need one element per token. The
built-in textarea paints no caret or selection at all
(`div-paint/src/painter.rs:3816-3852`).

**Proposal** (`divui`, mirroring the CSS Custom Highlight API):

```rust
pub struct Highlight { /* a set of Ranges and a priority */ }

impl Document {
    /// `CSS.highlights.set(name, highlight)`.
    pub fn set_highlight(&self, name: &str, highlight: Option<Highlight>) -> Result<(), Error>;
}
impl Element {
    /// Style for `::highlight(name)` within this element's subtree:
    /// color, background-color, text-decoration (the CSS-allowed subset).
    pub fn set_highlight_style(&self, name: &str, style: &Style) -> Result<(), Error>;
}
```

Paint-only: a highlight change must mark paint dirty, not layout. With
highlights, a line is one text node; tokens, selection, find matches and
bracket pairs are ranges. That cuts the element count per visible line
from ~20 to 1–2 (the probe lines average 9–17 spans).

## D. Incremental layout

**Today.** Any layout invalidation re-runs `block_layout` from the root
and re-records paint for the whole tree (`div-engine/src/lib.rs:2605-2618,2659-2665`);
`DirtyState` is a set of booleans (`lib.rs:310`). There is no shaping
cache: each text item is reshaped on every layout
(`div-layout/src/inline/items_builder.rs:290-312`). `content-visibility`
is parsed but not used (`div-style/src/computed.rs:800`). Nested flex
columns cost about 2.2x per level (sample-git gap 26, engine fix in
progress).

**Proposal.** This is engine-internal and has no new API: per-subtree
dirty bits and cached layout results (LayoutNG's model), a shaping cache
keyed by (text, font, features, direction), and `content-visibility: auto`
with `contain-intrinsic-size`. The editor's acceptance target, measured
with the probes here: editing one line of a windowed 10,000-line
document, then layout and paint, within one 60 Hz frame (16.7 ms) on the
reference machine, in a release build.

## E. Accessible text

**Today** (AccessKit 0.25, `div-engine/src/accessibility.rs`):
- a textarea is a `MultilineTextInput` with one synthetic `TextRun` child
  holding the whole value (`:745-775`);
- the run has `character_lengths` but no `character_positions`,
  `character_widths` or `word_starts`, so per-character bounds and word
  navigation are unavailable to AccessKit's UIA TextPattern and AT-SPI
  Text adapters;
- the text selection is set on the run, not on the control node AccessKit
  reads it from (`:761` vs `:819-831`), so caret-moved and
  selection-changed events do not fire (the probe
  `input_events::textarea_accessibility_text` checks the control node has
  no selection);
- every layout or paint change rebuilds and diffs the whole semantic tree
  (`div-engine/src/lib.rs:2908-2911`).

**Proposal.**
1. Set `text_selection` on the text-input node (S, engine).
2. Emit one `TextRun` per line box with `character_positions`,
   `character_widths` and `word_starts` from the shaping results (M,
   engine). Ordinary paragraphs then become readable by character too.
3. An element with an `EditContext` is exposed as an editable text input
   whose runs are its text nodes and whose selection is the context's
   selection, so a custom editor gets TextPattern/AT-SPI Text without
   app-side work (M, engine). Chromium's EditContext accessibility is
   itself incomplete; Div should match what Chromium exposes and record
   the difference.
4. Rebuild only changed subtrees (part of D).

## F. Text measurement

For views that draw text themselves (a minimap, a canvas-drawn editor,
charts), and as the canvas 2D context's `measureText` (the
`sample-canvas` investigation is designing the canvas API; this section
is its text half):

```rust
pub struct TextLayout { /* shaped runs of one line */ }

impl TextLayout {
    /// Shape `text` with the document's fonts and a computed-style-like
    /// font description (family list, size, weight, features, tab size).
    pub fn new(document: &Document, text: &str, font: &FontSpec) -> Result<Self, Error>;
    pub fn width(&self) -> f32;
    pub fn metrics(&self) -> LineMetrics;            // ascent, descent, line gap
    pub fn x_for_offset(&self, offset: usize) -> f32; // caret x
    pub fn offset_for_x(&self, x: f32) -> usize;      // hit test, grapheme-snapped
    pub fn range_rects(&self, range: std::ops::Range<usize>) -> Vec<(f32, f32)>; // bidi-split
}
```

It wraps `TextShaper`, `ShapeResult` and the document's `FontCollection`;
offsets snap to grapheme cluster boundaries. Canvas `fillText` with a
`TextLayout` draws the same glyphs. Re-exporting div-text's internals
directly is not proposed: its offsets are scalar indices and its types
change with the engine.

## G. A native text view (not recommended now)

The alternative is a Div-only element that takes a document model and
draws only the visible lines itself, as Zed's GPUI editor element does:

```rust
pub trait TextSource {
    fn line_count(&self) -> usize;
    fn line(&self, index: usize) -> StyledLine; // text + style runs + decorations
}
pub struct TextView { /* element */ }
impl TextView {
    pub fn new(document: &Document, source: Rc<dyn TextSource>) -> Self;
    pub fn invalidate_lines(&self, range: std::ops::Range<usize>);
    pub fn caret_rect(&self, line: usize, offset: usize) -> Rect;
    pub fn position_at(&self, x: f32, y: f32) -> (usize, usize);
}
```

It is the fastest design and its scope is narrow (L in engine paint,
layout and accessibility). It is not recommended first because:
- it is not a web feature, so code ported from CodeMirror or Monaco does
  not run on it and Chromium cannot be its oracle;
- A–E are needed by other apps anyway (selection of ordinary text, find
  in page, IME in any custom widget, accessible paragraphs), and with
  them the DOM path can be fast enough once D lands;
- it would need its own IME and accessibility plumbing, which B and E
  provide for every element.

Revisit G if, after D, the windowed-DOM probe cannot meet the frame
target in section D.
