# Fidelity tooling

The oracle for this sample is the original CodeMirror 6 in pinned
Chromium. `docs/design.md` section 5 describes the plan. This directory
holds step 1, the reference capture.

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

## Still to come

- The line-rendering benchmark in Chromium (design section 4).
- The structural and pixel comparisons against the port (M6).
- Behavioural fidelity (M7).
