// Builds the Chromium reference page for the CodeMirror 6 fidelity cases.
//
//     cd fidelity/reference && npm ci && node build.mjs
//
// It bundles src/main.mjs with the pinned packages into pages/editor.js,
// copies Geist Mono (the same file Div's div-components bundles) next to
// it, and writes pages/editor.html and pages/versions.json. The pages are
// committed; fidelity/capture.py loads them in pinned Chromium.

import { build } from "esbuild";
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const pages = join(here, "pages");
const modules = join(here, "node_modules");
const PACKAGES = [
  "@codemirror/state",
  "@codemirror/view",
  "@codemirror/commands",
  "style-mod",
  "w3c-keyname",
  "crelt",
  "geist",
  "esbuild",
];
const versions = Object.fromEntries(
  PACKAGES.map((name) => [name, JSON.parse(readFileSync(join(modules, name, "package.json"), "utf8")).version]),
);

mkdirSync(join(pages, "fonts"), { recursive: true });
await build({
  entryPoints: [join(here, "src/main.mjs")],
  bundle: true,
  format: "iife",
  target: "chrome120",
  outfile: join(pages, "editor.js"),
  legalComments: "eof",
  logLevel: "warning",
});
copyFileSync(
  join(modules, "geist/dist/fonts/geist-mono/GeistMono-Variable.woff2"),
  join(pages, "fonts/GeistMono-Variable.woff2"),
);
copyFileSync(join(modules, "geist/LICENSE.txt"), join(pages, "fonts/OFL.txt"));

const html = `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>CodeMirror 6 reference</title>
<style>
@font-face {
  font-family: "Geist Mono";
  src: url("fonts/GeistMono-Variable.woff2") format("woff2");
  font-weight: 100 900;
}
html, body { margin: 0; background: #ffffff; }
body { width: 800px; overflow: hidden; }
</style>
</head>
<body>
<script src="editor.js"></script>
</body>
</html>
`;
writeFileSync(join(pages, "editor.html"), html);
writeFileSync(join(pages, "versions.json"), JSON.stringify(versions, null, 2) + "\n");
console.log(versions);
