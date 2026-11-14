# RFC draft: the editing platform for Div

Status: **draft from the sample-editor investigation**, for the Div tracks
to accept, change or reject. Nothing here is implemented in Div, and this
sample does not patch or fork Div.

Pinned Div: `565db46a`. Evidence: [design.md](design.md) (sections 2 and 4)
and the probes in [`probes/tests`](../probes/tests).

**Accepted decisions (13 Nov 2026).** The user accepted the platform plan's
decisions (`platform-plan/decisions-2026-11-13.md`, Q1–Q16). For this RFC:

- **B is adopted** (decision 6): web-shaped `Range`, `Selection` and
  `StaticRange`, UTF-8 byte offsets in Rust and UTF-16 at the FFI. Owner:
  engine.
- **`contenteditable` before `EditContext`** (decision 11). Owner: engine.
- **One `DataTransfer`** for clipboard and drag; files are web-style `File`
  objects wrapping paths (decision 7). Owner: framework, with the engine.
- **Runtime CSS** (decision 2): a browser-compatible ingestion mode for
  third-party sheets (drops what Chromium drops and reports it) and runtime
  custom-property writes now; runtime sheet insertion later, alongside the
  editor work. Section H says what CodeMirror needs from it.
- **Frame and observer shapes** (decision 4): `requestAnimationFrame` per
  document with a typed handle, in every window; one `ResizeObserver` with
  three boxes, computed by the engine after layout and delivered by the
  framework before paint; Pointer Events as typed enums and bitflags with
  web values, mouse `pointerId` 1. These are Tier 0 work.

## Principle

CodeMirror 6, Monaco, ProseMirror/TipTap and Lexical are built on a known
set of web platform features. Div should implement those features, matched
to Chromium's behaviour and pixels, and verified with Div's comparison
harness and WPT. Then a Rust port of any of these packages works the way
the package does in the browser. Div should not add an editor-specific
primitive.

The Rust surface uses typed handles and Rust types (`Element`, `TextNode`,
`Range`, enums for `inputType`). Each call keeps the web method's
semantics: the same algorithm, the same event order, and the same
exceptions mapped to `Error` variants. Where the doc comment of a Rust
method names its web counterpart, the Chromium behaviour of that
counterpart is the specification.

**Offsets** (adopted, decision 6). The web counts DOM text offsets in UTF-16 code units. Div's
existing Rust selection API counts UTF-8 bytes (`Element::selection`). This
RFC keeps UTF-8 byte offsets in the Rust API, and the engine keeps the web
model internally. Div's FFI facade converts to UTF-16 at its boundary. An
offset that is not on a char boundary is an `Error`, as the web's
`IndexSizeError` is for an offset past the end.

| Section | Features (design.md numbering) | Owner | Size |
| --- | --- | --- | --- |
| [A](#a-editing-host-contenteditable-and-input-events) | 2 `contenteditable`, 3 `beforeinput`/`input` | engine (editing, layout, paint); framework (events) | L + M |
| [B](#b-selection-and-range) | 4 Selection API and painting, 5 Range geometry, 6 caret from point (adopted) | engine | L + M + M |
| [C](#c-composition-and-ime) | 7 composition and IME; 20 `EditContext` after `contenteditable` | framework (platform, events) | M (L for EditContext) |
| [D](#d-clipboard-and-drag-and-drop) | 8 clipboard events and 9 drag and drop, sharing one `DataTransfer` | framework, with the engine | M + M |
| [E](#e-observers-and-frame-callbacks) | 10 `MutationObserver`, 11 `ResizeObserver`, 12 `IntersectionObserver`, 13 `requestAnimationFrame`, 19 `document.fonts` | engine (DOM, layout, text); framework | M + S–M + M + S + S |
| [F](#f-fast-dom-line-rendering) | 16 layout that scales with what changed | engine (layout, text, paint) | L |
| [G](#g-accessible-editable-text) | 17 UIA TextPattern / AT-SPI Text for editable content | engine (accessibility) | M |
| [H](#h-runtime-stylesheets-for-style-mod) | 14b runtime `<style>` text, 14c `insertRule` and adopted sheets | engine (style); framework (API) | S–M + M |

Not drafted here:
- 14a, stylesheets and selectors: Tier 0 on the engine track.
- 15, UI Events completeness: a list of small fixes in design.md.
- 18, canvas 2D and `measureText`: designed by `sample-canvas`.
- 21, `<textarea>` parity: Chromium's textarea is the specification.

---

## A. Editing host: `contenteditable` and input events

**Today.** The `contenteditable` attribute only gives an empty block its
caret line (`div-layout/src/block.rs:5107`). An element carrying it is not
focusable (`focus()` returns `NotFocusable`), and typing into it changes
nothing. `beforeinput`/`input` fire only for `<input>`/`<textarea>`
(`divui/src/document.rs:1588`), with ten input types
(`divui/src/events.rs:10-21`).

**Proposal.**

```rust
impl Element {
    /// `element.contentEditable = "true" | "plaintext-only" | "false"`.
    /// An editing host is focusable, shows a caret, and edits its DOM
    /// through the input event flow below.
    pub fn set_content_editable(&self, mode: ContentEditable) -> Result<(), Error>;
    pub fn is_content_editable(&self) -> Result<bool, Error>;
}

/// The full Input Events Level 2 `inputType` list (insertText,
/// insertReplacementText, insertLineBreak, insertParagraph,
/// insertCompositionText, insertFromPaste, insertFromDrop,
/// deleteContentBackward/Forward, deleteWordBackward/Forward,
/// deleteSoftLineBackward/Forward, deleteHardLineBackward/Forward,
/// deleteByCut, deleteByDrag, historyUndo, historyRedo, format*, …).
#[non_exhaustive]
pub enum InputType { /* … */ }

impl InputEventInfo {
    pub fn input_type(&self) -> InputType;
    pub fn data(&self) -> Option<&str>;
    pub fn data_transfer(&self) -> Option<&DataTransfer>;   // paste and drop
    /// `InputEvent.getTargetRanges()`: the static ranges the edit replaces.
    pub fn target_ranges(&self) -> &[StaticRange];
    pub fn is_composing(&self) -> bool;
}
```

**Engine work.**
- Editing commands over the DOM (insert text, split and merge blocks,
  delete by unit), driven by the default action of `beforeinput`.
  Lexical and CodeMirror cancel most of these and apply their own edits.
  ProseMirror lets many through and reads the result with a
  `MutationObserver` (E).
- Caret painting in editing hosts (blink rate from the platform), and
  `caret-color`.
- Undo grouping for native edits (`historyUndo`/`historyRedo` as input
  types).
- Line and word movement by visual lines. This needs B's geometry.

**Verify.** WPT `editing/` (contenteditable, `run/` command tests),
`input-events/`. Harness screenshots of the caret and of edits in
`white-space: pre` and `pre-wrap` hosts.

## B. Selection and Range

**Today.** Selection exists only inside form controls. Geometry exists
only for element boxes (`divui/src/element.rs:807,815`).
`Document::hit_test` is `elementFromPoint` (`document.rs:861`). div-text has
`x_position_for_offset` and `offset_for_x_position`
(`div-text/src/shaping/shape_result.rs:298,312`), but layout keeps no text
offset map that can be queried.

**Proposal.**

```rust
/// A live DOM Range.
pub struct Range { /* … */ }
/// `StaticRange`: an immutable range, as `getTargetRanges()` returns.
pub struct StaticRange { /* … */ }

impl Document {
    pub fn create_range(&self) -> Range;
    /// `document.getSelection()`: the document's single Selection.
    pub fn selection(&self) -> Selection;
    /// `document.caretPositionFromPoint(x, y)`.
    pub fn caret_position_from_point(&self, x: f32, y: f32)
        -> Result<Option<CaretPosition>, Error>;
}

impl Range {
    pub fn set_start(&self, node: &Node, offset: usize) -> Result<(), Error>;
    pub fn set_end(&self, node: &Node, offset: usize) -> Result<(), Error>;
    pub fn collapsed(&self) -> bool;
    /// `Range.getClientRects()`: per line-box fragment, viewport coordinates;
    /// a collapsed range in text gives the caret rect.
    pub fn client_rects(&self) -> Result<Vec<Rect>, Error>;
    pub fn bounding_rect(&self) -> Result<Option<Rect>, Error>;
}

impl Selection {
    pub fn anchor(&self) -> Option<(Node, usize)>;
    pub fn focus(&self) -> Option<(Node, usize)>;
    pub fn range_count(&self) -> usize;
    pub fn get_range_at(&self, index: usize) -> Result<Range, Error>;
    pub fn collapse(&self, node: &Node, offset: usize) -> Result<(), Error>;
    pub fn extend(&self, node: &Node, offset: usize) -> Result<(), Error>;
    pub fn set_base_and_extent(&self, anchor: (&Node, usize), focus: (&Node, usize))
        -> Result<(), Error>;
    pub fn remove_all_ranges(&self);
}
// `selectionchange` fires on the Document, coalesced as in Chromium.
```

`Node` is the common handle over `Element` and `TextNode` (which divui
already has separately).

**Engine work.**
- Inline layout keeps, for each text fragment, its text node, its source
  byte range and enough shaping data for x-for-offset. Offsets count
  source characters, including tabs that layout rewrites as spaces
  (`div-layout/src/inline/items_builder.rs:1232-1246`).
- `offset_for_x_position` uses a binary search.
- Selection painting: native highlight, `::selection` style, and
  `user-select`.
- Pointer selection: click, drag, double and triple click. These need
  `MouseEvent.detail` (feature 15).

**Verify.** WPT `selection/`, `dom/ranges/`, `css/cssom-view/` (including
`caretPositionFromPoint` and `getClientRects`). Harness screenshots of
selections over LTR, RTL, mixed bidi, ligatures, tabs, wrapped lines and
empty lines.

## C. Composition and IME

**Today.** Composition events reach only form controls
(`document.rs:1629`). The platform drops the preedit cursor
(`Ime::Preedit(text, _)`, `div-platform/src/desktop.rs:687`) and never
calls winit's `set_ime_cursor_area`. The `input` after a commit has no
`inputType` (`document.rs:1673,1692`). No `input` fires during composition.

**Proposal** (no new API for apps; it is the web's event flow):
- Composition events and `beforeinput`/`input` with
  `insertCompositionText` reach any focused editing host (A), in
  Chromium's order: `compositionstart`, then
  `beforeinput`/`compositionupdate`/`input` for each update, then
  `compositionend`.
- `CompositionEvent.data`, and the preedit selection kept from the
  platform.
- The platform places the candidate window at the caret's rect: B's
  geometry for the focused selection, passed to `set_ime_cursor_area` after
  each layout that moves it.
- Later, after `contenteditable` (decision 11), **`EditContext`**
  (feature 20), which Monaco's newer input path uses. It is the same flow with the app holding the text. It also gives
  `characterboundsupdate` for the IME. It is drafted only once A–C exist,
  since it shares their plumbing.

**Verify.**
- WPT `uievents/` composition tests.
- Behavioural comparison with Chromium driven by CDP
  `Input.imeSetComposition`.
- A manual native check with the Windows IME and IBus/Fcitx on Linux,
  recorded with a screenshot of the candidate window position.

## D. Clipboard and drag and drop

**Today.** Plain text only, through a private document clipboard synced
around Ctrl+C/X/V (`div-platform/src/desktop.rs:924-949`). There are no
`copy`/`cut`/`paste` events and no drag and drop.

**Proposal.**

One `DataTransfer` type serves clipboard events, drag events and the
`dataTransfer` of `beforeinput` (decision 7).

```rust
/// `DataTransfer`: typed items. `text/plain`, `text/html` and custom
/// web types (`web application/x-…`), as Chromium maps them to native formats.
pub struct DataTransfer { /* … */ }
impl DataTransfer {
    pub fn get_data(&self, format: &str) -> Option<String>;
    pub fn set_data(&self, format: &str, data: &str);
    pub fn types(&self) -> Vec<String>;
    /// Web-style `File` objects; on desktop they wrap paths.
    pub fn files(&self) -> Vec<File>;
    pub fn set_drag_image(&self, element: &Element, x: f32, y: f32);
}
impl Event {
    /// `ClipboardEvent.clipboardData` and `DragEvent.dataTransfer`.
    pub fn data_transfer(&self) -> Option<&DataTransfer>;
}
```

The events:
- `copy`, `cut` and `paste` fire on the focused editing host or the
  selection's container, and are cancelable, as in Chromium.
- `dragstart`, `drag`, `dragenter`, `dragover`, `dragleave`, `drop` and
  `dragend` fire with OS drag sources and targets: OLE on Windows, XDND
  and Wayland data-device on Linux.
- Native formats map like Chromium's: `CF_UNICODETEXT` and `CF_HTML` on
  Windows; `text/plain;charset=utf-8` and `text/html` on Linux. Custom
  types map like Chromium's web custom formats.

**Verify.** WPT `clipboard-apis/`, `html/editing/dnd/`. A round trip with a
native app (pasting HTML from a browser) checked manually.

## E. Observers and frame callbacks

**Today.** None of these exist. Div has timers (`divui/src/task.rs:157-178`)
and the platform redraw loop.

**Proposal.** Each follows its spec's timing in the event loop, as
Chromium runs it. The Rust callbacks receive typed records.

```rust
pub struct MutationObserver;      // observe(target, MutationObserverInit), take_records, disconnect
pub struct ResizeObserver;        // observe(element, box): content-box, border-box, device-pixel-content-box
pub struct IntersectionObserver;  // root, root_margin, thresholds; entries with ratios and rects
impl Document {
    /// `requestAnimationFrame`: runs before style, layout and paint of the
    /// next frame of this document's window; works in every window.
    pub fn request_animation_frame(&self, f: impl FnOnce(f64) + 'static) -> FrameRequestId;
    pub fn cancel_animation_frame(&self, id: FrameRequestId);
    /// `document.fonts`: `ready`, and `loading`/`loadingdone` events.
    pub fn fonts(&self) -> FontFaceSet;
}
```

`MutationObserver` records come from the engine's DOM mutation points.
Those include edits that A's editing host makes; ProseMirror and CM6 read
exactly these. `ResizeObserver` and `IntersectionObserver` run after
layout in the update-the-rendering steps: the engine computes the entries
after layout and the framework delivers them before paint (decision 4).
`requestAnimationFrame` and `ResizeObserver` are Tier 0 work.

**Verify.** WPT `dom/nodes/MutationObserver-*`, `resize-observer/`,
`intersection-observer/`, `html/webappapis/update-rendering/`,
`css/css-font-loading/`.

## F. Fast DOM line rendering

**Today, measured** (design.md section 4):
- **Cost per box.** Layout costs about 0.2 ms per inline box in a block
  container and about 2.7x that in a flex column.
- **CM6-sized viewport.** About 1,900 boxes cost 272 ms per edit with
  layout and paint, and 454 ms per scroll.
- **Every change is whole-document.** Each change re-lays out from the
  root and re-records paint for the whole tree
  (`div-engine/src/lib.rs:2605-2618,2659-2665`).
- **No caching.** Each text item is shaped again on every layout, and
  again for intrinsic sizing (`div-layout/src/inline/items_builder.rs:290-312`).
  Shaping itself is cheap (23 µs per 45-character line).

**Proposal.** This is engine-internal and adds no API.
1. Profile the per-box cost. The hypotheses are: font resolution per item
   (`items_builder.rs:291-300`), double shaping, and repeated
   `block_layout` passes (`lib.rs:2602-2648`).
2. Add a shaping cache keyed by text, font, features and direction.
3. Add per-subtree dirty bits, and cache layout results for clean
   subtrees, as LayoutNG does. Then editing one `.cm-line` re-lays out
   that line and its ancestors' block offsets.
4. Re-record paint only for damaged subtrees, and limit damage to the
   changed rect.
5. Implement `content-visibility: auto` with `contain-intrinsic-size`
   (parsed today, `div-style/src/computed.rs:800`, but unused).

**Target.** The probe `window_flow_cost` with the 1000px margin must fit
one edit plus layout plus paint within 16.7 ms on the reference machine.
Chromium's time for the same page on the same machine, measured in phase
2, is the reference. The nested-flex fix already in progress (sample-git
gap 26) is part of this.

## G. Accessible editable text

**Today.**
- **Selection on the wrong node.** AccessKit provides UIA TextPattern and
  AT-SPI Text for text inputs, and it reads the selection from the input
  node. Div sets the selection on the child run instead
  (`div-engine/src/accessibility.rs:761` vs `:819-831`).
- **Runs without positions.** Runs have no `character_positions`,
  `character_widths` or `word_starts` (`:745-775`).
- **Whole-tree rebuilds.** Every invalidation rebuilds the whole tree
  (`div-engine/src/lib.rs:2908-2911`).

**Proposal.**
1. Put `text_selection` on the text-input node (S).
2. Emit a `TextRun` per line box with positions, widths and word starts
   from layout (M). This uses the offset map from B.
3. Expose an editing host (A) the way Chromium does: role, editable state,
   its text runs, and the document Selection as the text selection (M).
4. Rebuild only changed subtrees (part of F).

**Verify.** Compare with Chromium's accessibility tree for the same page,
from CDP `Accessibility.getFullAXTree`. Manual checks with NVDA (Windows)
and Orca (Linux) reading by character, word and line, and following the
caret.

## H. Runtime stylesheets for style-mod

**What CodeMirror does.** CM6 styles everything through `style-mod`
4.1.4. Its `StyleModule.mount` takes one of two paths
(`style-mod/src/style-mod.js:94-139`):

- **A document root** (it has a `head`), which is how an editor is
  normally mounted: a `<style>` element in `<head>`, whose `textContent` is
  rewritten with every mounted module's rules whenever the set changes.
- **A shadow root**: one constructed `CSSStyleSheet` per document, filled
  with `insertRule` and prepended to the root's `adoptedStyleSheets`.

So the common case needs no CSSOM, only a `<style>` element whose text
the engine parses at runtime and re-parses when it changes (14b). It fits
decision 2's browser-compatible ingestion mode: the sheet comes from a
third-party package, so Div should drop what Chromium drops and report
it. `insertRule` and adopted sheets (14c) are needed only for editors in
shadow roots and belong to decision 2's later CSSOM step.

**Proposal.**
- 14b: `<style>` elements in the document are stylesheets, parsed in the
  ingestion mode when connected and whenever their text changes, and
  ordered by document position, as in Chromium. Size S–M once the
  ingestion mode exists. Owner: engine (style), framework (the element's
  text API).
- 14c: `CSSStyleSheet` construction, `insertRule`, `deleteRule`, and
  `adoptedStyleSheets` on documents and shadow roots. Size M. Owner:
  framework (API surface), engine (rule storage and invalidation).

**Verify.** WPT `css/cssom/` (`insertRule`, constructable stylesheets) and
`html/semantics/document-metadata/the-style-element/`. The harness renders
the CM6 reference captures (design.md section 5) with their `<style>` text
unchanged.

## Rejected alternatives

- **A Div-only text-view element**, as in Zed's GPUI. **Rejected:** it
  does not port the packages, and Chromium cannot serve as its oracle. The
  measured performance gap belongs in F.
- **Exposing div-text's shaping types to apps.** **Rejected:** that is
  not a web API; canvas `measureText` (feature 18) is the web API.
- **A Div-specific EditContext before `contenteditable`.** **Rejected:**
  three of the four packages need `contenteditable`, and the fourth uses a
  `<textarea>` today.
