#!/usr/bin/env python3
"""Record Chromium's visual order of the bidi test lines.

Upstream's webtest-bidi.ts checks `computeOrder` against the browser it
runs in: it draws each line with one span per character, in a left-to-right
and then a right-to-left block, and sorts the characters by `offsetLeft`.
This runs that same query (`queryBrowserOrder`) in headless Chrome for
Testing 147.0.7727.50 and writes the result to
fixtures/bidi-order-v1/order.json, which view/tests/bidi.rs compares the
port's order with. Indices count code points, as the query's `for...of`
does.

Run it in Linux (WSL) with a Div checkout for the CDP helpers, as
capture.py does:

    python3 fidelity/bidi_order.py --div /path/to/div
    python3 fidelity/bidi_order.py --div /path/to/div --check
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys

import capture

HERE = Path(__file__).resolve().parent
OUT = HERE / "fixtures/bidi-order-v1/order.json"

# The cases of webtest-bidi.ts, in order.
CASES = [
    "codemirror",
    "كودالمرآة",
    "codeمرآة",
    "الشفرةmirror",
    "codeمرآةabc",
    "كود1234المرآة",
    "كودabcالمرآة",
    "كو,",
    "code123مرآة157abc",
    "  foo  ",
    "  مرآة  ",
    "ab12-34%م",
    "م1234%bc",
    "ر12:34ر",
    "xyאהxyאהxyאהxyאהxyאהxyאהxyאה",
    "ab مرآة10 cde 20مرآة!",
    "(ء)و)",
    "(([^ء-ي]|^)و)",
    "ء(و)",
    "[foo(barء)]",
]

# `queryBrowserOrder` from webtest-bidi.ts, without its macOS font.
QUERY = """
(strings => {
  let scratch = document.body.appendChild(document.createElement("div"))
  for (let str of strings) {
    let wrap = scratch.appendChild(document.createElement("div"))
    wrap.style.whiteSpace = "pre"
    for (let ch of str) {
      let span = document.createElement("span")
      span.textContent = ch
      wrap.appendChild(span)
    }
  }
  let ltr = [], rtl = []
  for (let i = 0; i < 2; i++) {
    let dest = i ? rtl : ltr
    scratch.style.direction = i ? "rtl" : "ltr"
    for (let cur = scratch.firstChild; cur; cur = cur.nextSibling) {
      let positions = []
      for (let sp = cur.firstChild, i = 0; sp; sp = sp.nextSibling) positions.push([i++, sp.offsetLeft])
      dest.push(positions.sort((a, b) => a[1] - b[1]).map(x => x[0]))
    }
  }
  scratch.remove()
  return {ltr, rtl}
})(%s)
"""


def query(chrome: Path) -> dict:
    with capture.chromium_page(chrome) as client:
        client.command("Page.navigate", {"url": "about:blank"})
        order = capture.evaluate(client, QUERY % json.dumps(CASES))
    return {
        "chromium": capture.chromium_version(chrome),
        "source": "@codemirror/view 6.43.15 test/webtest-bidi.ts queryBrowserOrder",
        "cases": CASES,
        "ltr": order["ltr"],
        "rtl": order["rtl"],
    }


def render(result: dict) -> str:
    return json.dumps(result, indent=1, ensure_ascii=False) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--div", type=Path, required=True, help="Div checkout with the CDP helpers")
    parser.add_argument("--chrome", type=Path, default=capture.DEFAULT_CHROME)
    parser.add_argument("--check", action="store_true", help="compare with the committed fixture")
    args = parser.parse_args()

    version = capture.chromium_version(args.chrome)
    if version != capture.EXPECTED_CHROMIUM:
        raise SystemExit(f"Chromium {version!r}, expected {capture.EXPECTED_CHROMIUM}")
    capture.load_pixel_runner(args.div.resolve())

    text = render(query(args.chrome))
    if args.check:
        if OUT.read_text(encoding="utf-8") != text:
            print(f"differs from {OUT.relative_to(HERE)}")
            return 1
        print(f"matches {OUT.relative_to(HERE)}")
        return 0
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(text, encoding="utf-8", newline="\n")
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
