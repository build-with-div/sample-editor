# Design: a code editor on Div

Phase 1 of the sample-editor dogfood: what a professional text and code
editor (in the line of VS Code or Zed) needs from Div, what Div has at the
pinned commit, measurements, the recommended architecture, and the Div work
it depends on. The proposed Div APIs are drafted in [rfc-text.md](rfc-text.md).

Pinned Div: `565db46a` (build-with-div/div). References like
`divui/src/document.rs:1588` are paths under Div's `crates/` at that commit.

## 1. What the editor needs

| Area | Requirement |
| --- | --- |
| Model | A rope for large documents; transactions; undo/redo with selection restore; multiple selections |
| View | Only visible lines exist (100k-line file stays responsive); gutter with line numbers; folding; indentation guides; soft wrap; minimap (optional) |
| Text | Monospace and proportional fonts, ligatures, tabs, bidi, wide (CJK) and emoji clusters |
| Caret and selection | Painted caret (blinking), selection, multiple cursors; caret geometry for any offset; hit testing from a point to an offset |
| Mouse | Click, drag, double click (word), triple click (line), Alt/Ctrl click (add cursor), right click menu |
| Keyboard | Named keys with modifiers and repeat; Tab kept by the editor; command keymap |
| Text input and IME | Committed text and composition (preedit) on the editor surface; candidate window placed at the caret |
| Clipboard | Text; ideally a private format for multi-cursor paste and HTML for rich copy |
| Language | tree-sitter highlighting (incremental), bracket matching, find/replace |
| Scrolling | Smooth and programmatic scrolling, reveal a position |
| Accessibility | UIA TextPattern (Windows) and AT-SPI Text (Linux): read by character/word/line, follow the caret and selection |

## 2. What Div has today

From reading Div's source at `565db46a` and running the probes in
[`probes/tests`](../probes/tests).

### Text and shaping (engine: div-text, div-layout)

- Shaping is HarfBuzz through Skia's SkShaper, with per-run font fallback
  (`div-text/src/shaping/shaper.rs:506,612-615`), UAX#9 bidi
  (`div-text/src/bidi/paragraph.rs`), OpenType features from
  `font-variant-ligatures`, `font-kerning` and `font-feature-settings`
  (`div-text/src/font/features.rs:78-146`), `tab-size` and every
  `white-space` value (`div-style/src/enums.rs:532,1389`). Line breaking is
  in div-layout (`div-layout/src/inline/line_breaker.rs`).
- `ShapeResult` has caret and hit-test primitives:
  `x_position_for_offset`, `offset_for_x_position`, `width_for_range`
  (`div-text/src/shaping/shape_result.rs:298,312,354`). Offsets are
  Unicode scalar indices. `offset_for_x_position` scans linearly.
- **None of it is public.** divui re-exports only font registration types
  (`divui/src/lib.rs:153-164`). There is no text measurement API, no caret
  rectangle, no range rectangles, and no point-to-offset hit test:
  `Document::hit_test` returns an element (`divui/src/document.rs:861`).
- In `pre` text, layout rewrites tabs as spaces
  (`div-layout/src/inline/items_builder.rs:1232-1246`), so a future offset
  map must count source tabs.
- There is no shaping cache. Each text item is shaped on every layout
  (`items_builder.rs:290-312`), and intrinsic sizing shapes it again
  (`:1860-1930`).

### Editing (engine: interaction; framework: divui)

- `<input>`/`<textarea>` keep a `String` value, UTF-8 byte selection, and
  full-copy undo snapshots (`div-engine/src/interaction.rs:142-158,240-251`);
  each insertion copies the value into the `value` attribute and marks
  layout dirty (`:793-811`).
- Keys map to `EditCommand` move/delete by character or word, Home/End
  by logical line, select all, undo/redo, cut/copy/paste
  (`divui/src/document.rs:1328-1440`). Up/Down do nothing in text controls.
- The textarea paints its text by shaping every line of the value on
  every paint (`div-paint/src/painter.rs:3801-3852`). It paints **no caret,
  no selection and no composition** (the single-line input paints all
  three: `painter.rs:4142-4209`), does not wrap, and does not scroll.
- Mouse: a press focuses the target only (`document.rs:1111-1113`); there
  is no click-to-caret, drag selection or double/triple click.

### Keyboard, text input and IME (framework: divui, div-platform)

- `Event::key()` gives a `Key` (`divui/src/keyboard.rs:11-33`) and
  `Event::repeat` (Div #27), with `Modifiers` SHIFT/CTRL/ALT/META. No
  `code`, no Insert key, no modifier-only keys.
- `keydown`/`keyup` reach any focused element (any `tabindex` element can
  take focus); `preventDefault` on keydown keeps Tab
  (`document.rs:1294`, before the focus move at `:1319`).
- `beforeinput`/`input` (with `inputType`) and
  `compositionstart/update/end` are dispatched **only to `<input>` and
  `<textarea>`**: `dispatch_text_input` and `dispatch_composition_start`
  return early for anything else (`document.rs:1588,1629`).
- The platform uses winit 0.30.13's IME events (`div-platform/src/desktop.rs:657-708`,
  `set_ime_allowed` at `:747`). It drops the preedit caret range
  (`Ime::Preedit(text, _)`, `:687`) and never calls `set_ime_cursor_area`,
  so no app can position the candidate window. Raw `PlatformEvent`s are
  visible to `App::on_platform_event` (`divui/src/app.rs:198`), after the
  document has handled them, with no element target.
- The `beforeinput`/`input` pair after a composition commit carries no
  `inputType` (`document.rs:1673,1692`), and no `input` fires during
  composition updates (Chromium fires `insertCompositionText`).

### Clipboard (framework)

Text only: `CF_UNICODETEXT` on Windows, `UTF8_STRING` on X11,
smithay-clipboard on Wayland (`div-platform/src/windows_clipboard.rs`,
`desktop.rs:650`); public `div::clipboard::read_text`/`write_text`
(`divui/src/clipboard.rs:35-59`).

### Accessibility (engine: div-engine accessibility; AccessKit 0.25)

- AccessKit (`accesskit_windows` 0.35, `accesskit_unix` 0.23) provides
  UIA TextPattern and AT-SPI Text for text-input nodes whose children are
  `TextRun`s.
- A textarea is `MultilineTextInput` with one synthetic `TextRun` holding
  the whole value (`div-engine/src/accessibility.rs:745-775`). The run has
  `character_lengths` but no `character_positions`, `character_widths` or
  `word_starts`, and the text selection is set on the run, not on the
  control node AccessKit reads it from (`:761` vs `:819-831`). So
  per-character bounds, word navigation and caret/selection change events
  are not available (probe: `textarea_accessibility_text`).
- App code can set role, label, description, value, live, hidden and
  relations on any element (`divui/src/element.rs:750-801`), but cannot
  expose a custom element's text as a text pattern.
- Every invalidation rebuilds and diffs the whole semantic tree
  (`div-engine/src/lib.rs:2908-2911`).

### Layout, paint, scrolling (engine)

- Any layout change re-runs `block_layout` from the root and re-records
  paint for the whole tree (`div-engine/src/lib.rs:2605-2618,2659-2665`).
  `content-visibility` is parsed but unused.
- Scrolling: `scroll_to`, `scroll_by`, `scroll_into_view`, smooth variants,
  `scroll_metrics`, and a `scroll` event (`divui/src/element.rs:828-930`).
  `bounding_rect`/`client_rects` give element boxes.
- No custom drawing: `<canvas>` is a placeholder
  (`div-paint/src/painter.rs:6331-6333`); the `sample-canvas` session is
  designing a 2D context.
- Timers (`div::task::time::interval`) are enough for a caret blink.

## 3. Measurements

All on Windows 11, 24 cores, 64 GB RAM; headless `TestApp` (CPU Skia
raster), 800x600 CSS px at scale 1; `cargo test` dev profile, where Div
and all dependencies build at `opt-level = 2` and the probes unoptimized;
one run each, so treat them as indicative. Command:

```bash
PROBE_SCALE=full cargo test -p editor-probes -- --nocapture --test-threads=1
```

Lines are Rust-like source, 5–55 bytes, monospace 13px, 18px line height.
"Layout" is `Document::update_all`; "paint" is `render_at` (record and
raster one frame).

### A `<textarea>` with a large document ([textarea.rs](../probes/tests/textarea.rs))

| Lines (bytes) | Mount + layout | First paint | One keystroke (event, edit, layout) | Paint after it |
| --- | --- | --- | --- | --- |
| 1,000 (29.5 KB) | 39 ms | 7.5 ms | 26 ms | 6.1 ms |
| 10,000 (301 KB) | 291 ms | 20 ms | 300 ms | 22 ms |
| 100,000 (3.07 MB) | 2.55 s | 171 ms | 2.79 s | 182 ms |

Enter, Backspace, Ctrl+Z and ArrowRight cost the same as a typed key:
the cost grows linearly with document size, so the textarea is unusable
as an editor above a few thousand lines even before its missing caret,
selection, scrolling and Up/Down (section 2).

### One element per line, a span per token ([line_elements.rs](../probes/tests/line_elements.rs))

Every line present ("all"), in a flex-column shell, 9–17 spans per line plus
an `inline-block` gutter:

| Lines | Build | Layout | Edit one line, then layout + paint | Scroll to middle + paint |
| --- | --- | --- | --- | --- |
| 200 | 240 ms | 2.10 s | 2.16 s | 83 ms |
| 500 | 873 ms | 5.70 s | 6.87 s | 314 ms |
| 2,000 | 2.43 s | 27.3 s | 29.1 s | 5.63 s |
| 200, under 3 more nested flex columns | 225 ms | 40.6 s | 42.1 s | 120 ms |

An earlier 2,000-line run reached 6.3 GB of working set, and its
10,000-line case was stopped after 10 minutes.

Only the visible lines exist (43 lines), positioned absolutely in a tall
spacer, in the same flex shell ("window"):

| Document lines | Layout | Edit one line + layout + paint | Scroll to middle (rebuild window) + paint |
| --- | --- | --- | --- |
| 10,000 | 867 ms | 883 ms | 1.09 s |
| 100,000 | 887 ms | 887 ms | 1.02 s |

Windowing makes the cost independent of document size, but not small.

The same window in normal flow between two spacers, in a block-only
shell (`window_flow_cost`, 100,000-line document):

| Line content | Inline boxes in view | Layout | Edit one line + layout + paint | Scroll to middle + paint |
| --- | --- | --- | --- | --- |
| Plain text (one text node per line) | 43 | 13.7 ms | 21.0 ms | 21.8 ms |
| Same-colour tokens merged into one span | 249 | 79.1 ms | 70.9 ms | 94.8 ms |
| A span per token | 555 | 161 ms | 151 ms | 210 ms |

### Where the layout time goes ([inline_cost.rs](../probes/tests/inline_cost.rs))

Fresh document, `white-space: pre` lines, "token" and " " spans, one layout:

| | 1 span | 4 | 8 | 16 | 32 |
| --- | --- | --- | --- | --- | --- |
| 40 lines, block container | 12.6 ms | 35.7 ms | 65.9 ms | 127 ms | 266 ms |
| 40 lines, flex column container | 28.3 ms | 101 ms | 182 ms | 363 ms | 789 ms |

| Lines x 8 spans | 10 | 40 | 160 | 640 |
| --- | --- | --- | --- | --- |
| Block | 13.4 ms | 72.5 ms | 280 ms | 1.27 s |
| Flex column | 44.5 ms | 196 ms | 710 ms | 3.28 s |

Layout is linear in inline boxes at about **0.2 ms per inline box** in a
block container and about 2.7x that in a flex column; the inline-block
gutter adds little; `white-space: normal` is about 20% cheaper. Each
nested flex column multiplies again (sample-git gap 26, engine fix in
progress); absolute positioning in a flex shell added roughly another
2.4x in the "window" case above.

### div-text primitives ([shaping.rs](../probes/tests/shaping.rs))

Calling engine-internal div-text directly (not public API), 2,000 calls:

| Operation | Per call |
| --- | --- |
| `TextShaper::new` | 0.3 µs |
| `Font::new` (monospace 13px, warm collection) | 33.2 µs |
| Shape a 5-character token | 13.0 µs |
| Shape a ~45-character source line | 23.3 µs |
| `x_position_for_offset`, `offset_for_x_position` (50 chars) | < 0.1 µs |

Shaping a screen of 45 lines costs about 1 ms, so the ~200 µs per inline
box in layout is mostly not shaping. A hypothesis for the engine track to
profile (not measured here): `Font::new_in_collection` per text item
(`items_builder.rs:291-300`), shaping twice (intrinsic and layout), and up
to three `block_layout` passes per update (`div-engine/src/lib.rs:2602-2648`).

### Events ([input_events.rs](../probes/tests/input_events.rs))

| Signal | Focusable `div` (custom surface) | `<textarea>` |
| --- | --- | --- |
| `keydown`/`keyup` with `Key`, modifiers, `repeat` | yes | yes |
| Tab kept by `preventDefault` on keydown | yes | yes |
| `beforeinput`/`input` for typed text | **no** | yes, `inputType=insertText` |
| `compositionstart`/`update`/`end` | **no** | yes; preedit text written into the value |
| `input` after a composition commit | — | yes, **no `inputType`** |
| Click on a child focuses the focusable ancestor | **no** (Chromium: yes) | — |
| Second click at the same point | `click` again; **no `dblclick`**, no click count | — |
| Right-button press | **fires `click`**, no `contextmenu` (Chromium: `auxclick`, `contextmenu`) | — |
| `pointer*` events from headless mouse input | no (sample-git gap 7) | — |
| Clipboard | text round trip works | — |

A 50-byte monospace span measured 7.148 px per character from its box,
equal to div-text's advance (7.147 px), so the cell size can be measured
from layout.

Not measured: native Windows/Linux IME with a real input method, screen
readers, GPU raster, release builds.

## 4. Approaches

| | 1. Div elements (one per line, as CodeMirror and Monaco) | 2. A text-view primitive in Div (as Zed's GPUI) | 3. Canvas 2D context + text measurement |
| --- | --- | --- | --- |
| How it draws | Windowed line elements; tokens as spans, later as custom highlights | Engine element pulls visible lines from an app `TextSource` and draws them with div-text | App draws glyph runs into a canvas each frame |
| Div must expose | Range rects + caret from point (A), EditContext (B), highlights (C), faster layout (D), accessible text (E) | The element (G), plus B and E | Canvas (sample-canvas), `TextLayout`/`measureText` (F), B, and a custom accessibility tree |
| Speed today | Plain text 21 ms per edit; highlighted 71–151 ms (block shell) | Not built; shaping a screen ≈ 1 ms | Not built; no canvas |
| Text features (bidi, fallback, ligatures, wrap) | Engine's | Engine's, via div-text | App's own line breaking, bidi and fallback on top of F |
| Accessibility | From the DOM once E lands | Custom per element | Fully custom; hardest |
| IME | B | B | B |
| Chromium as the oracle | Yes: every API is a web API | No | Canvas and `measureText` only |
| Useful to other Div apps | High: selection, find, IME in any widget, accessible paragraphs | Low: editors only | Medium: charts, minimaps |

## 5. Recommendation

**Build on Div elements (approach 1), windowed, and ask Div for the web
features that make it fast and complete: A, B, C, D and E in
[rfc-text.md](rfc-text.md).** Keep approach 2 as the fallback if, after
the layout work (D), the windowed view cannot edit one line within a
16.7 ms frame. Use approach 3 only for the minimap, once the canvas
context exists.

Reasons:
- A 45-line window of plain text already edits in about 21 ms; the
  remaining cost is per inline box, which highlights (C) remove from the
  editor's side and the layout work (D) removes from Div's side.
- Every API it needs is a Chromium API, so the pixel and behaviour oracle
  Div already uses applies, and code that ports CodeMirror or Monaco
  patterns works.
- Each API is useful beyond editors. A native text view (2) would need its
  own IME and accessibility plumbing anyway.

### Architecture

```
editor-core (no Div dependency, unit-tested)
  Buffer (ropey::Rope) · Selections (multi-cursor, UTF-8 offsets)
  Transaction / History (undo groups, selection restore)
  Syntax (tree-sitter, incremental reparse on edits; highlight spans by line)
  Folding ranges · Bracket pairs · Search (regex over the rope)
  DisplayMap (folds, soft-wrap rows, tab expansion) → visible rows
      │ rows for [first, last), with style runs and decorations
      ▼
editor-view (Div)
  Scroller (overflow: auto, block shell) ── on scroll ──▶ recompute window
  Window: spacer · row elements (gutter + text) · spacer
  Overlay layer: carets, selections, matches, indent guides (absolute boxes)
  Input: focus target ─ keydown → Command; text + IME → Insert
  Geometry: cell metrics today; Range rects / caret-from-point later (A)
```

- **Model.** `ropey` for text; offsets are UTF-8 bytes, as Div's
  selection APIs use. Transactions carry the selection before and after,
  so undo restores it. tree-sitter parses incrementally and highlights only
  rows in view.
- **View.** Only the rows in view plus an overscan of about 10 exist.
  Unchanged rows are reused by row identity, so an edit touches one or two
  row elements. Tokens are merged by style until highlights exist.
- **Input.** Today: a focused, visually hidden `<textarea>` receives text and
  IME (`div-gap: 2`). The app reads each `input` then clears the control, and
  handles commands from `keydown` with `preventDefault`. After B: an
  `EditContext` on the editor surface.
- **Geometry.** Today: monospace only. The cell width is measured once from
  a span's box; x = column × cell, and a column is counted in grapheme
  clusters with tabs expanded. CJK wide characters, emoji, ligatures,
  proportional fonts and bidi are wrong until A or F lands, and the
  prototype says so in the UI.
- **Caret and selection.** Absolutely positioned boxes in an overlay above
  the rows. The caret blinks on a `div::task::time::interval`.
- **Accessibility.** Rows carry their text. The surface gets the
  `MultilineTextInput` role and a value of the visible text (a stopgap)
  until E exposes the real text pattern.

### Public component API (sketch)

```rust
use sample_editor::{CodeEditor, EditorHandle, EditorOptions, Language};

let editor = EditorHandle::new(text);            // owns editor-core state
editor.set_language(Language::Rust);
editor.on_change(move |change| save_state.set(Dirty));

view! {
    <CodeEditor
        editor={editor.clone()}
        options={EditorOptions {
            font_family: "Geist Mono".into(),
            font_size: 13.0,
            line_height: 18.0,
            tab_size: 4,
            soft_wrap: false,
            line_numbers: true,
            minimap: false,
            read_only: false,
            ..Default::default()
        }}
    />
}

// Commands, also bound to the default keymap:
editor.run(Command::AddCursorBelow);
editor.select_next_occurrence();
editor.find(FindQuery::regex(r"fn \w+"));
editor.reveal(Position { line: 1200, column: 0 }, Reveal::Center);
let text: String = editor.text();                 // or editor.slice(range)
```

`CodeEditor` is a `#[component]` that owns the scroller, window and overlay;
`EditorHandle` is `Clone` (shared state behind `Rc`) and exposes signals for
selections, scroll position and dirty state, so the app's toolbar and
status bar react with Div's signals.

## 6. Div work this depends on

Owner tracks: **engine** (div-style, div-layout, div-paint, div-text,
div-engine), **framework** (divui, div-platform, div-macros, div-cli),
**components** (div-components). Size: **S** about a week or less,
**M** a few weeks, **L** more, for one track owner. "Removed by" names the web
feature that closes the gap, as Div's gap rule asks. Numbers are this
sample's `div-gap: <n>` labels.

| # | Gap | Removed by | Owner | Size | Needed for |
| --- | --- | --- | --- | --- | --- |
| 1 | A fresh lockfile does not build on Linux (zbus pins) | Div pins `zbus_macros`/`zvariant` | framework (packaging) | S | building any app (sample-git gap 24) |
| 2 | Text input and IME reach only `<input>`/`<textarea>` | `EditContext` (RFC B) | framework (divui, platform); engine (focus, editability) | L | typing and IME on a custom surface; workaround: hidden textarea |
| 3 | IME details: preedit caret dropped, candidate window never positioned (`set_ime_cursor_area`), commit `input` lacks `inputType`, no `input` during composition | EditContext's `updateSelectionBounds`/`updateCharacterBounds`; UI Events composition order | framework (platform, divui) | M | CJK input placed at the caret, also in plain textareas |
| 4 | `<textarea>` is not an editor view: no caret/selection/composition paint, no content scrolling, Up/Down no-op, undo moves the caret to the end, O(document) work per keystroke | HTML `<textarea>` as Chromium renders and edits it | engine (forms, paint) | M | any app with a notes field; not on this editor's path |
| 5 | Mouse: no `dblclick` or click count (`detail`); right press fires `click`, no `auxclick`/`contextmenu`; pressing a child does not focus the focusable ancestor; headless input has no `pointer*` | UI Events `MouseEvent.detail`, `dblclick`, `auxclick`, `contextmenu`; HTML focus fixup; Pointer Events | framework (events) | S–M | word/line selection, context menu, focus on click, drag tests |
| 6 | Layout cost: ~0.2 ms per inline box, ~2.7x per flex column level, more under absolute positioning; whole-document relayout and paint per change; no shaping or layout cache | LayoutNG-style cached, incremental layout; shaping cache; `content-visibility: auto` (RFC D) | engine (layout, text) | L | highlighted rows within a frame; any app shell with nested flex |
| 7 | No text geometry: caret rect, range rects, offset from point | `Range.getClientRects()`, `document.caretPositionFromPoint()` (RFC A) | engine (layout); framework (divui) | M | proportional fonts, ligatures, CJK, bidi, wrap; mouse to offset |
| 8 | No decoration of text ranges without elements | CSS Custom Highlight API, `::highlight()` (RFC C) | engine (style, paint); framework | M | syntax colours, selection, find matches at one node per line |
| 9 | Accessible text: selection on the wrong node, no per-character positions or word starts, no text pattern for a custom surface | AccessKit text runs per line box; EditContext exposed as editable text (RFC E) | engine (accessibility) | M | screen readers in any text field and in the editor |
| 10 | No text measurement API | Canvas `measureText()` / `TextMetrics` (RFC F) | engine (div-text); framework | M | minimap, canvas views, column math without a document |
| 11 | No custom drawing | Canvas 2D context (being designed by `sample-canvas`) | engine (paint); framework | L | minimap (optional) |
| 12 | Clipboard is plain text only | Async Clipboard API with `text/html` and web custom formats | framework (platform) | S–M | multi-cursor paste fidelity, rich copy |
| 13 | Keys: no `Insert`, no physical `code`, no modifier-only keys; native F13–F24 unmapped | UI Events `KeyboardEvent.key`/`code` | framework (events) | S | full keymaps (Insert, Ctrl+Insert) |
| 14 | No stylesheets, `:hover`/`:focus`, custom properties, `prefers-color-scheme` | CSS cascade, selectors, `var()`, media queries (in progress) | engine (style) | L (in progress) | themed editor chrome; workaround: inline styles |

Order of value for this editor: 6 and 8 (speed of highlighted rows),
then 2 and 3 (input), 7 (geometry beyond monospace ASCII), 5, 9, 1.

## 7. Risks

- **Layout speed is the critical path.** Even windowed, highlighted rows
  cost 71–151 ms per edit in a block shell and about 0.9 s in a flex shell
  with absolute rows, measured here. An app shell of nested flex columns
  makes this worse (sample-git gap 26). Until gap 6 lands, the prototype
  keeps the editor in a block-only subtree and merges tokens by colour.
- **IME is untested natively.** The hidden-textarea workaround depends on
  winit's IMM32 (Windows) and text-input/XIM (Linux) paths. With no
  `set_ime_cursor_area`, the candidate window will not follow the caret.
  Reconversion and TSF services need TSF (L by itself).
- **Monospace-cell geometry is a stopgap.** It is wrong for wide
  characters, emoji, ligatures, proportional fonts, bidi and soft wrap of
  such text. Gap 7 or 10 is required for a real editor.
- **Accessibility of custom editors is hard everywhere.** Chromium's own
  EditContext accessibility is incomplete; matching it may still leave
  screen-reader gaps that need a Div-specific extension.
- **EditContext is a large surface** (events, bounds, formats, platform
  plumbing), and it is the API most likely to slip.
- **Measurements are single-run, debug-profile, headless numbers.** Native
  frame times also include presentation, and release builds will differ.
- **tree-sitter** is C, built with `cc` on both platforms. It adds compile
  time, but no Div risk.

## 8. What can be prototyped now

With Div `565db46a` and labelled workarounds, in small PRs:

1. `editor-core`: rope buffer, multi-selections, transactions, undo with
   selection restore, word/line motions, search. Pure Rust, unit-tested,
   no Div.
2. A windowed plain-text view in a block-only shell. It uses one text node
   per row, a gutter, a caret and selection overlay, a blinking caret,
   mouse click and drag through the monospace cell, and a scroll-driven
   window. The target is a 100k-line file, measured against the
   21 ms-per-edit baseline above.
3. Input: a hidden textarea for text and IME (`div-gap: 2`), and a
   keydown keymap for commands, Tab kept with `preventDefault`. Double and
   triple click are timed by the app (`div-gap: 5`).
4. tree-sitter highlighting with tokens merged by colour. Report the
   per-edit cost against the 71 ms measured here.
5. Find/replace, bracket matching, multiple cursors, folding, indent guides.
6. Soft wrap for monospace text, via the display map.

Not until Div work lands: proportional fonts and ligature-correct carets
(7 or 10), correct IME candidate placement (3), screen-reader text (9),
minimap drawing (11). A rich-text editor is a separate later sample. It
needs the same A, B, C and E (an `EditContext` over a rich model, not
`contenteditable`; see RFC B) plus `text/html` clipboard (12).
