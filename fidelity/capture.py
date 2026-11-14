#!/usr/bin/env python3
"""Capture the CodeMirror 6 reference cases in pinned Chromium.

Design section 5, step 1. For each case in reference/src/main.mjs this
loads reference/pages/editor.html#<case> in headless Chrome for Testing
147.0.7727.50 at 800x600, scale 1, and writes to
fixtures/reference-v1/<case>/:

- chromium.png: the viewport, once two consecutive frames are identical;
- editor.html: the editor's outerHTML;
- styles.css: the text of every stylesheet the page uses, including the
  ones style-mod adopts;
- ax.json: the accessibility tree, without Chromium's node ids;
- case.json: the document, selection and scroll line, for the Div side.

fixtures/reference-v1/manifest.json records the Chromium version, the
package versions, the Div tooling commit and each file's SHA-256.

Run it in Linux (WSL) with a Div checkout for the CDP helpers
(tools/accountability/run_all_pixel_comparisons.py) and the real-font
fontconfig profile:

    python3 fidelity/capture.py --div /path/to/div
    python3 fidelity/capture.py --div /path/to/div --check

--check captures into a temporary directory and fails unless every file
matches the committed fixtures byte for byte.
"""

from __future__ import annotations

import argparse
import base64
from contextlib import ExitStack, contextmanager
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import urllib.request

HERE = Path(__file__).resolve().parent
PAGE = HERE / "reference/pages/editor.html"
VERSIONS = HERE / "reference/pages/versions.json"
FIXTURES = HERE / "fixtures/reference-v1"
DEFAULT_CHROME = Path.home() / "openui-oracle/chrome/linux-147.0.7727.50/chrome-linux64/chrome"
EXPECTED_CHROMIUM = "147.0.7727.50"
WIDTH, HEIGHT = 800, 600
FILES = ["chromium.png", "editor.html", "styles.css", "ax.json", "case.json"]

pixel_runner = None


def load_pixel_runner(div_root: Path) -> None:
    global pixel_runner
    sys.path.insert(0, str(div_root / "tools/accountability"))
    import run_all_pixel_comparisons  # noqa: E402

    pixel_runner = run_all_pixel_comparisons


def chromium_version(chrome: Path) -> str:
    output = subprocess.run([str(chrome), "--version"], capture_output=True, text=True).stdout
    return output.strip().split()[-1] if output.strip() else ""


def git_head(path: Path) -> str:
    return subprocess.run(
        ["git", "-C", str(path), "rev-parse", "HEAD"], capture_output=True, text=True, check=True
    ).stdout.strip()


@contextmanager
def chromium_page(chrome: Path):
    """A headless pinned Chromium with one page, driven over CDP."""
    env = pixel_runner.chrome_environment(str(chrome.parent), use_real_font=True)
    with tempfile.TemporaryDirectory(prefix="sample-editor-") as profile_dir, ExitStack() as stack:
        process = subprocess.Popen(
            [
                str(chrome), "--headless", "--disable-gpu", "--no-sandbox", "--no-first-run",
                "--no-default-browser-check", "--hide-scrollbars",
                "--enable-features=FontDataServiceLinux:typeface/Freetype",
                "--remote-debugging-port=0", f"--user-data-dir={profile_dir}",
                f"--window-size={WIDTH},{HEIGHT + 87}", "about:blank",
            ],
            env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True,
        )
        stack.callback(pixel_runner._stop_chromium_process, process)
        port = pixel_runner._wait_for_devtools_endpoint(profile_dir, process, timeout=90)
        client = pixel_runner._CdpWebSocket(pixel_runner._page_websocket_url(port))
        stack.callback(pixel_runner._close_chromium_client, client)
        client.command("Page.enable")
        client.command("Emulation.setDeviceMetricsOverride", {
            "width": WIDTH, "height": HEIGHT, "deviceScaleFactor": 1, "mobile": False,
            "screenWidth": WIDTH, "screenHeight": HEIGHT,
        })
        yield client


def evaluate(client, expression: str):
    result = client.command("Runtime.evaluate", {
        "expression": expression, "awaitPromise": True, "returnByValue": True,
    })
    if "exceptionDetails" in result:
        raise RuntimeError(f"page script failed: {result['exceptionDetails']}")
    return result.get("result", {}).get("value")


# Fonts loaded, then a few frames so CodeMirror's measure cycle (which runs
# in requestAnimationFrame) has settled the viewport and gutters.
SETTLE_SCRIPT = """
new Promise(resolve => {
  const frames = n => n ? requestAnimationFrame(() => frames(n - 1)) : resolve();
  const done = () => document.fonts.ready.then(() => frames(4));
  if (document.readyState === 'complete') done();
  else addEventListener('load', done, {once: true});
}).then(() => [innerWidth, innerHeight, devicePixelRatio,
               document.fonts.check("13px 'Geist Mono'")])
"""

STYLES_SCRIPT = """
(() => {
  const text = sheet => Array.from(sheet.cssRules, rule => rule.cssText).join('\\n');
  const parts = [];
  Array.from(document.styleSheets).forEach((sheet, i) =>
    parts.push(`/* document.styleSheets[${i}] */\\n` + text(sheet)));
  Array.from(document.adoptedStyleSheets).forEach((sheet, i) =>
    parts.push(`/* document.adoptedStyleSheets[${i}] */\\n` + text(sheet)));
  return parts.join('\\n\\n') + '\\n';
})()
"""


def ax_tree(client) -> dict:
    """The full accessibility tree, nested, without Chromium's ids."""
    client.command("Accessibility.enable")
    nodes = client.command("Accessibility.getFullAXTree")["nodes"]
    by_id = {node["nodeId"]: node for node in nodes}

    def value(field):
        return field.get("value") if isinstance(field, dict) else None

    def convert(node):
        out = {"role": value(node.get("role", {}))}
        if node.get("ignored"):
            out["ignored"] = True
        name = value(node.get("name", {}))
        if name:
            out["name"] = name
        node_value = value(node.get("value", {}))
        if node_value not in (None, ""):
            out["value"] = node_value
        properties = {
            prop["name"]: value(prop.get("value", {}))
            for prop in node.get("properties", [])
        }
        # The document URL holds the checkout's absolute path; keep the
        # part relative to the pages directory.
        if isinstance(properties.get("url"), str):
            properties["url"] = properties["url"].rsplit("/pages/", 1)[-1]
        if properties:
            out["properties"] = dict(sorted(properties.items()))
        children = [convert(by_id[c]) for c in node.get("childIds", []) if c in by_id]
        if children:
            out["children"] = children
        return out

    roots = [node for node in nodes if "parentId" not in node]
    if len(roots) != 1:
        raise RuntimeError(f"expected one accessibility root, got {len(roots)}")
    return convert(roots[0])


def screenshot(client) -> bytes:
    options = {"format": "png", "fromSurface": True, "captureBeyondViewport": False}
    frames = [client.command("Page.captureScreenshot", options)["data"] for _ in range(2)]
    while frames[-1] != frames[-2]:
        if len(frames) > 8:
            raise RuntimeError("the page did not settle")
        frames.append(client.command("Page.captureScreenshot", options)["data"])
    return base64.b64decode(frames[-1], validate=True)


def capture_case(client, name: str, out: Path) -> None:
    client.command("Page.navigate", {"url": "about:blank"})
    url = "file:" + urllib.request.pathname2url(str(PAGE)) + "#" + name
    client.command("Page.navigate", {"url": url})
    metrics = evaluate(client, SETTLE_SCRIPT)
    if metrics != [WIDTH, HEIGHT, 1, True]:
        raise RuntimeError(f"{name}: viewport or font not as expected: {metrics}")
    case = evaluate(client, "window.__case")
    if not case or case.get("name") != name:
        raise RuntimeError(f"{name}: the page mounted {case and case.get('name')}")
    case["scrollTop"] = evaluate(client, "window.__view.scrollDOM.scrollTop")
    out.mkdir(parents=True, exist_ok=True)
    (out / "chromium.png").write_bytes(screenshot(client))
    (out / "editor.html").write_text(
        evaluate(client, "document.querySelector('.cm-editor').outerHTML") + "\n",
        encoding="utf-8", newline="\n",
    )
    (out / "styles.css").write_text(evaluate(client, STYLES_SCRIPT), encoding="utf-8", newline="\n")
    (out / "ax.json").write_text(
        json.dumps(ax_tree(client), indent=1, ensure_ascii=False) + "\n", encoding="utf-8", newline="\n"
    )
    (out / "case.json").write_text(
        json.dumps(case, indent=1, ensure_ascii=False) + "\n", encoding="utf-8", newline="\n"
    )


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def capture_all(chrome: Path, div_root: Path, out: Path) -> dict:
    with chromium_page(chrome) as client:
        client.command("Page.navigate", {"url": "file:" + urllib.request.pathname2url(str(PAGE))})
        evaluate(client, SETTLE_SCRIPT)
        cases = evaluate(client, "window.__cases")
        for name in cases:
            capture_case(client, name, out / name)
    manifest = {
        "chromium": chromium_version(chrome),
        "viewport": {"width": WIDTH, "height": HEIGHT, "deviceScaleFactor": 1},
        "packages": json.loads(VERSIONS.read_text(encoding="utf-8")),
        "div_tooling": git_head(div_root),
        "cases": {
            name: {file: sha256(out / name / file) for file in FILES} for name in cases
        },
    }
    (out / "manifest.json").write_text(json.dumps(manifest, indent=1) + "\n", encoding="utf-8", newline="\n")
    return manifest


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--div", type=Path, required=True, help="Div checkout with the CDP helpers")
    parser.add_argument("--chrome", type=Path, default=DEFAULT_CHROME)
    parser.add_argument("--check", action="store_true", help="compare with the committed fixtures")
    args = parser.parse_args()

    version = chromium_version(args.chrome)
    if version != EXPECTED_CHROMIUM:
        raise SystemExit(f"Chromium {version!r}, expected {EXPECTED_CHROMIUM}")
    load_pixel_runner(args.div.resolve())

    if not args.check:
        manifest = capture_all(args.chrome, args.div, FIXTURES)
        print(json.dumps(manifest, indent=1))
        return 0

    with tempfile.TemporaryDirectory(prefix="sample-editor-check-") as temp:
        fresh = capture_all(args.chrome, args.div, Path(temp))
    committed = json.loads((FIXTURES / "manifest.json").read_text(encoding="utf-8"))
    failures = [
        f"{name}/{file}"
        for name, files in committed["cases"].items()
        for file, digest in files.items()
        if fresh["cases"].get(name, {}).get(file) != digest
    ]
    failures += [f"{name} (new case)" for name in fresh["cases"] if name not in committed["cases"]]
    if failures:
        print("differs from the committed fixtures: " + ", ".join(failures))
        return 1
    print(f"{sum(len(f) for f in committed['cases'].values())} files match the committed fixtures")
    return 0


if __name__ == "__main__":
    sys.exit(main())
