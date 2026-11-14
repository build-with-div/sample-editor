#!/usr/bin/env python3
"""Run the line-rendering benchmark in pinned Chromium.

Design section 4. lines.js ports probes/tests/line_elements.rs and
probes/tests/inline_cost.rs; this loads lines.html in headless Chrome for
Testing 147.0.7727.50 at 800x600, scale 1, and times each step the probes
time in Div. Each run of a case is a fresh page.

A step is the DOM mutation, timed in the page with performance.now(), and
the frame that follows it, timed from Chromium's own trace
(devtools.timeline). The phases are:

- style: UpdateLayoutTree;
- layout: Layout;
- paint: PrePaint, Paint, Layerize and the main thread's Commit;
- raster: RasterTask, summed over the raster threads (CPU time).

Nested events of one phase on one thread are counted once. Style and
layout that the mutation forces (scrollTo does) fall inside the mutation's
time; console.timeStamp marks bracket the mutation in the trace, so
"script" is the mutation without them and "total" counts them once.
"Total" is script plus the four phases: what the Div probe's "edit one
line + layout + paint" measures there. Before the timed runs, one pass
over every case warms the JIT and is discarded. Tracing has a cost of its
own; the numbers include it.

The hit test is the mean of 100 document.elementFromPoint calls at
distinct points along the row y = 300 (Chromium caches a repeated point),
timed in the page.

Run it in Linux (WSL) with a Div checkout for the CDP helpers, as
capture.py:

    python3 fidelity/bench/run.py --div /path/to/div
    python3 fidelity/bench/run.py --div /path/to/div --runs 5 --out fidelity/bench/results/chromium-147.json

It prints Markdown tables of the medians (and ranges over the runs) and,
with --out, writes every run's numbers with the conditions.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
from pathlib import Path
import platform
import statistics
import sys
import urllib.request

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import capture  # noqa: E402

PAGE = HERE / "lines.html"
PHASES = {
    "style": {"UpdateLayoutTree"},
    "layout": {"Layout"},
    "paint": {"PrePaint", "Paint", "Layerize", "Commit"},
    "raster": {"RasterTask"},
}
CATEGORIES = "devtools.timeline,disabled-by-default-devtools.timeline"
HIT_TEST_CALLS = 100  # at distinct points: Chromium caches a repeated one


class Session:
    """CDP commands that keep the events Div's helper skips."""

    def __init__(self, client):
        self.client = client
        self.events = []

    def command(self, method, params=None):
        client = self.client
        command_id = client.next_id
        client.next_id += 1
        request = {"id": command_id, "method": method}
        if params is not None:
            request["params"] = params
        client._send_frame(json.dumps(request, separators=(",", ":")).encode("utf-8"))
        while True:
            message = json.loads(client._read_message())
            if message.get("id") == command_id:
                if "error" in message:
                    raise RuntimeError(f"CDP {method} failed: {message['error']}")
                return message.get("result", {})
            if "method" in message:
                self.events.append(message)

    def wait_event(self, method):
        while True:
            for index, event in enumerate(self.events):
                if event["method"] == method:
                    return self.events.pop(index)
            message = json.loads(self.client._read_message())
            if "method" in message:
                self.events.append(message)

    def evaluate(self, expression):
        result = self.command("Runtime.evaluate", {
            "expression": expression, "awaitPromise": True, "returnByValue": True,
        })
        if "exceptionDetails" in result:
            raise RuntimeError(f"page script failed: {result['exceptionDetails']}")
        return result.get("result", {}).get("value")

    def trace(self, expressions):
        """Evaluate the expressions under tracing; their values and the events."""
        self.command("Tracing.start", {
            "traceConfig": {"includedCategories": CATEGORIES.split(","), "recordMode": "recordAsMuchAsPossible"},
            "transferMode": "ReturnAsStream",
        })
        values = [self.evaluate(expression) for expression in expressions]
        self.command("Tracing.end")
        stream = self.wait_event("Tracing.tracingComplete")["params"]["stream"]
        chunks = []
        while True:
            chunk = self.command("IO.read", {"handle": stream, "size": 1 << 20})
            data = chunk.get("data", "")
            chunks.append(base64.b64decode(data).decode("utf-8") if chunk.get("base64Encoded") else data)
            if chunk.get("eof"):
                break
        self.command("IO.close", {"handle": stream})
        trace = json.loads("".join(chunks))
        events = trace["traceEvents"] if isinstance(trace, dict) else trace
        return values, events


def renderer_pid(events) -> int:
    """The process that ran the page's script: the renderer."""
    for event in events:
        if event.get("name") == "FunctionCall" or event.get("name") == "EvaluateScript":
            return event["pid"]
    raise RuntimeError("no script event in the trace")


def step_window(events, pid):
    """The trace timestamps of the step's start and end marks."""
    marks = {}
    for event in events:
        if event.get("pid") == pid and event.get("name") == "TimeStamp":
            message = event.get("args", {}).get("data", {}).get("message")
            if message in ("bench-step-start", "bench-step-end"):
                marks[message] = event["ts"]
    if len(marks) != 2:
        raise RuntimeError(f"step marks missing from the trace: {marks}")
    return marks["bench-step-start"], marks["bench-step-end"]


def phase_times(events):
    """Milliseconds per phase in the renderer, nesting counted once per
    thread, and the part of each that fell inside the step's mutation."""
    pid = renderer_pid(events)
    window_start, window_end = step_window(events, pid)
    intervals = {phase: {} for phase in PHASES}
    open_events = {}
    for event in events:
        if event.get("pid") != pid:
            continue
        name = event.get("name")
        phase = next((p for p, names in PHASES.items() if name in names), None)
        if phase is None:
            continue
        key = (event["tid"], name)
        kind = event.get("ph")
        if kind == "X":
            span = (event["ts"], event["ts"] + event.get("dur", 0))
        elif kind == "B":
            open_events.setdefault(key, []).append(event["ts"])
            continue
        elif kind == "E" and open_events.get(key):
            span = (open_events[key].pop(), event["ts"])
        else:
            continue
        intervals[phase].setdefault(event["tid"], []).append(span)
    times, inside = {}, {}
    for phase, by_thread in intervals.items():
        total = forced = 0.0
        for spans in by_thread.values():
            spans.sort()
            merged = []
            for start, stop in spans:
                if merged and start < merged[-1][1]:
                    merged[-1][1] = max(merged[-1][1], stop)
                else:
                    merged.append([start, stop])
            for start, stop in merged:
                total += stop - start
                forced += max(0, min(stop, window_end) - max(start, window_start))
        times[phase] = total / 1000.0
        inside[phase] = forced / 1000.0
    return times, inside


def run_step(session, name) -> dict:
    (mutation, _), events = session.trace([f"bench.step({json.dumps(name)})", "bench.frames()"])
    phases, forced = phase_times(events)
    times = {"mutation": mutation, "script": max(0.0, mutation - sum(forced.values())), **phases}
    times["total"] = times["script"] + sum(phases.values())
    return times


def fresh_page(session, kind, args) -> dict:
    session.command("Page.navigate", {"url": "about:blank"})
    session.command("Page.navigate", {"url": "file:" + urllib.request.pathname2url(str(PAGE))})
    metrics = session.evaluate(capture.SETTLE_SCRIPT.replace(
        "document.fonts.check(\"13px 'Geist Mono'\")", "typeof bench === 'object'"))
    if metrics != [capture.WIDTH, capture.HEIGHT, 1, True]:
        raise RuntimeError(f"viewport or page not as expected: {metrics}")
    return session.evaluate(f"bench.setup({json.dumps(kind)}, {json.dumps(args)})")


# The probes' cases (PROBE_SCALE=full sizes) and the steps each times.
CASES = [
    *[
        {"id": f"flow-{content.lower()}-{margin}", "group": "window_flow", "kind": "windowFlow",
         "args": [100_000, content, margin], "steps": ["build", "edit", "scroll"],
         "label": label}
        for content, margin, label in [
            ("Plain", 0, "Visible only (43), plain text"),
            ("Grouped", 0, "Visible only, same-colour tokens merged"),
            ("Tokens", 0, "Visible only, a span per token"),
            ("Tokens", 1000, "Visible + 1000px margin (CM6's default), a span per token"),
        ]
    ],
    *[
        {"id": f"windowed-{lines}", "group": "windowed", "kind": "windowed", "args": [lines],
         "steps": ["build", "edit", "insertTop", "scroll"], "hit_test": True, "label": f"{lines:,}"}
        for lines in (10_000, 100_000)
    ],
    *[
        {"id": f"all-{lines}-{depth}", "group": "all_lines", "kind": "allLines", "args": [lines, depth],
         "steps": ["build", "edit", "insertTop", "scroll"], "hit_test": True,
         "label": f"{lines:,}" if depth == 0 else f"{lines:,} inside 3 more nested flex columns"}
        for lines, depth in [(200, 0), (500, 0), (2_000, 0), (200, 3)]
    ],
    *[
        {"id": f"inline-{lines}x{spans}-{'flex' if flex else 'block'}", "group": "inline_cost",
         "kind": "inlineCost", "args": [lines, spans, flex, True, False], "steps": ["build"],
         "label": f"{lines} lines x {spans} spans, {'flex column' if flex else 'block'}"}
        for lines, spans in [(40, 1), (40, 2), (40, 4), (40, 8), (40, 16), (40, 32),
                             (10, 8), (160, 8), (640, 8)]
        for flex in (False, True)
    ],
]


def platform_fonts(session) -> list:
    """The fonts Chromium used for a token line's text (`monospace`)."""
    fresh_page(session, "windowFlow", [100, "Plain", 0])
    session.evaluate("bench.step('build')")
    session.evaluate("bench.frames()")
    session.command("DOM.enable")
    session.command("CSS.enable")
    root = session.command("DOM.getDocument", {"depth": 0})["root"]["nodeId"]
    line = session.command("DOM.querySelector", {
        "nodeId": root, "selector": "body > div > div:nth-child(2)"})["nodeId"]
    fonts = session.command("CSS.getPlatformFontsForNode", {"nodeId": line})["fonts"]
    session.command("CSS.disable")
    session.command("DOM.disable")
    return fonts


def run_case(session, case) -> dict:
    setup = fresh_page(session, case["kind"], case["args"])
    result = {"inlineBoxes": setup.get("inlineBoxes")}
    for step in case["steps"]:
        result[step] = run_step(session, step)
    if case.get("hit_test"):
        result["hitTest"] = session.evaluate(f"bench.hitTest({HIT_TEST_CALLS})")["ms"]
    return result


def fmt(values) -> str:
    """The median in ms, then the range over the runs."""
    def one(ms):
        return f"{ms:.3f}" if ms < 0.1 else f"{ms:.2f}" if ms < 10 else f"{ms:.1f}"
    median = statistics.median(values)
    if len(values) == 1:
        return f"{one(median)} ms"
    return f"{one(median)} ms ({one(min(values))}–{one(max(values))})"


def report(results) -> str:
    def col(case_id, step, phase="total"):
        return fmt([run[step][phase] for run in results[case_id]])

    def layout(case_id):
        return fmt([run["build"]["style"] + run["build"]["layout"] for run in results[case_id]])

    def paint(case_id):
        return fmt([run["build"]["paint"] + run["build"]["raster"] for run in results[case_id]])

    out = ["CM6-shaped viewport (window_flow_cost), 100,000 lines:", "",
           "| Lines in the DOM | Inline boxes | Style + layout | Paint + raster | "
           "Edit one line + layout + paint | Scroll to middle + paint |",
           "| --- | --- | --- | --- | --- | --- |"]
    for case in CASES:
        if case["group"] == "window_flow":
            boxes = results[case["id"]][0]["inlineBoxes"]
            out.append(f"| {case['label']} | {boxes:,} | {layout(case['id'])} | {paint(case['id'])} | "
                       f"{col(case['id'], 'edit')} | {col(case['id'], 'scroll')} |")
    for group, title in [("windowed", "Absolutely positioned window in a flex shell (windowed):"),
                         ("all_lines", "Every line in the DOM, flex shell (all_lines):")]:
        out += ["", title, "",
                "| Lines | Style + layout | Paint + raster | Edit one line + layout + paint | "
                "Insert at top + layout + paint | Scroll to middle + paint | Hit test |",
                "| --- | --- | --- | --- | --- | --- | --- |"]
        for case in CASES:
            if case["group"] == group:
                hit = fmt([run["hitTest"] for run in results[case["id"]]])
                out.append(f"| {case['label']} | {layout(case['id'])} | {paint(case['id'])} | "
                           f"{col(case['id'], 'edit')} | {col(case['id'], 'insertTop')} | "
                           f"{col(case['id'], 'scroll')} | {hit} |")
    out += ["", "Style + layout of a fresh document (inline_cost):", "",
            "| Case | Style + layout |", "| --- | --- |"]
    for case in CASES:
        if case["group"] == "inline_cost":
            out.append(f"| {case['label']} | {layout(case['id'])} |")
    return "\n".join(out) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--div", type=Path, required=True, help="Div checkout with the CDP helpers")
    parser.add_argument("--chrome", type=Path, default=capture.DEFAULT_CHROME)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--warmup", type=int, default=1, help="discarded passes before the runs")
    parser.add_argument("--only", help="run only the cases whose id starts with this")
    parser.add_argument("--out", type=Path, help="write every run's numbers here as JSON")
    args = parser.parse_args()

    version = capture.chromium_version(args.chrome)
    if version != capture.EXPECTED_CHROMIUM:
        raise SystemExit(f"Chromium {version!r}, expected {capture.EXPECTED_CHROMIUM}")
    capture.load_pixel_runner(args.div.resolve())

    cases = [case for case in CASES if not args.only or case["id"].startswith(args.only)]
    results = {case["id"]: [] for case in cases}
    with capture.chromium_page(args.chrome) as client:
        session = Session(client)
        fonts = platform_fonts(session)
        print(f"monospace resolves to {fonts}", file=sys.stderr)
        for _ in range(args.warmup):
            for case in cases:
                run_case(session, case)
        for run in range(args.runs):
            for case in cases:
                results[case["id"]].append(run_case(session, case))
            print(f"run {run + 1}/{args.runs} done", file=sys.stderr)

    if not args.only:
        print(report(results))
    else:
        print(json.dumps(results, indent=1))
    if args.out:
        record = {
            "chromium": version,
            "flags": "headless, --disable-gpu (software compositing and raster)",
            "monospace": fonts,
            "viewport": {"width": capture.WIDTH, "height": capture.HEIGHT, "deviceScaleFactor": 1},
            "machine": {"system": platform.system(), "release": platform.release(),
                        "cpus": os.cpu_count()},
            "div_tooling": capture.git_head(args.div),
            "runs": args.runs,
            "warmup": args.warmup,
            "cases": {case["id"]: {"kind": case["kind"], "args": case["args"], "runs": results[case["id"]]}
                      for case in cases},
        }
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(record, indent=1) + "\n", encoding="utf-8", newline="\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
