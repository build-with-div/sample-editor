# Fidelity tooling

The oracle for this sample is the original CodeMirror 6 in pinned
Chromium. `docs/design.md` section 5 describes the plan. This directory
holds step 1, the reference capture, and the line-rendering benchmark of section 4.

## Pinned packages

`reference/package.json` and `reference/package-lock.json` pin:
- `@codemirror/state` 6.7.6, `@codemirror/view` 6.43.15 and `@codemirror/commands` 6.11.1, with their dependencies (`style-mod`, `w3c-keyname`, `crelt`);
- `geist` 1.9.2 for Geist Mono, whose `GeistMono-Variable.woff2` is byte-identical to the copy Div's `div-components` bundles;
- `esbuild` 0.25.12, which bundles the page.

## The reference page

`reference/src/main.mjs` defines the cases. Each fixes the theme (Geist Mono 13px with an 18px line height), the 800x600 editor, the document text, the selection and the scroll position:

| Case | What it adds |
| --- | --- |
| `plain` | The editor and the document alone |
| `gutter` | Line numbers |
| `decorations` | Line numbers, plus a line, a mark, a widget and a replace decoration |
| `scrolled` | Line numbers, scrolled so line 150 is at the top |

Build the page on any machine with Node:

```bash
cd fidelity/reference && npm ci && node build.mjs
```

The output, `reference/pages/` (the bundle, the font with its OFL licence, `editor.html` and `versions.json`), is committed.

## Capturing in Chromium

`capture.py` runs in Linux (WSL) with Chrome for Testing 147.0.7727.50. It uses a Div checkout for the CDP helpers and the real-font fontconfig profile in `tools/accountability`. The committed fixtures were captured with Div at `92ea25a0`, the commit the workspace pins.

```bash
python3 fidelity/capture.py --div /path/to/div
```

```bash
python3 fidelity/capture.py --div /path/to/div --check
```

For each case, it writes these files to `fixtures/reference-v1/<case>/`:
- `chromium.png`: the viewport, once two consecutive frames are identical;
- `editor.html`: the editor's `outerHTML`;
- `styles.css`: the text of every stylesheet, including the ones `style-mod` adopts;
- `ax.json`: the accessibility tree, without Chromium's node ids;
- `case.json`: the document, selection and scroll position, which the Div side rebuilds.

`fixtures/reference-v1/manifest.json` records the Chromium version, the package versions, the Div tooling commit and each file's SHA-256.

`--check` captures again into a temporary directory and fails unless every file matches the manifest.

## The line-rendering benchmark

`bench/` is the Chromium side of the benchmark in design section 4. `bench/lines.js` ports the Div probes `probes/tests/line_elements.rs` and `probes/tests/inline_cost.rs` to plain JavaScript: the same source lines, tokens, colours, element structure and steps. The window cases count the same inline boxes as the probe: 43, 249, 555 and 1,921. `bench/lines.html` loads it; no build step is needed.

`bench/run.py` runs every case in Chrome for Testing 147.0.7727.50 (WSL), at 800x600 and scale 1, with a fresh page for each run of a case. It uses the same Div CDP helpers as `capture.py`.

Each step is timed in two parts:
- the DOM mutation, timed in the page with `performance.now()`;
- the frame that follows it, from Chromium's own trace (`devtools.timeline`), split into style (`UpdateLayoutTree`), layout (`Layout`), paint (`PrePaint`, `Paint`, `Layerize`, `Commit`) and raster (`RasterTask`, summed over the raster threads).

Style and layout that the mutation forces, as `scrollTo` does, are counted once. `console.timeStamp` marks bracket the mutation in the trace, so the runner can subtract them.

A warm-up pass over every case runs first and is discarded. Then each case runs five times, and the tables give the median and the range.

```bash
python3 fidelity/bench/run.py --div /path/to/div --out fidelity/bench/results/chromium-147.json
```

`bench/results/chromium-147.json` holds every run's numbers, together with the conditions: the Chromium version, the fonts `monospace` resolved to, the machine and the Div tooling commit.

## The structural comparison

`view/tests/fidelity.rs` draws each case's document and decorations with the port on Div and compares the `.cm-line` elements with those in the case's `editor.html`, line for line:

```bash
cargo test -p editor-view --test fidelity
```

Chromium drew only its viewport's lines; the port has no viewport yet and draws them all, so the test compares the lines Chromium drew with the same lines of the port. Div's DOM cannot yet walk text nodes, list attributes or name tags (design section 2, gaps 23–25), so the port's lines are read in the tile tree's order, with the attributes CodeMirror writes read by name, and tags named by Div's element kind.

## Still to come

- The structural comparison of the editor's outer elements and gap spacers, and the pixel comparison (M6).
- Behavioural fidelity (M7).
