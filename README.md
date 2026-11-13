# sample-editor

A port of a code editor (CodeMirror 6's architecture) to
[Div](https://github.com/build-with-div/div), to find the web platform
features Div must implement for the editors professionals use: CodeMirror 6,
Monaco, ProseMirror/TipTap and Lexical. Div is a native Rust UI framework
with no webview that renders like Chromium. This sample uses only Div's
public API; every temporary workaround carries a `// div-gap: <n>` comment
naming the platform feature that will replace it.

Status: **phase 1, investigation.** There is no editor yet.

- [docs/design.md](docs/design.md): packages → the platform features that
  power them → what Div has, what is missing, owner and size; measurements;
  the fidelity plan against the packages in Chromium.
- [docs/rfc-text.md](docs/rfc-text.md): draft Div APIs for those platform
  features (`contenteditable`, input events, Selection and Range, IME,
  clipboard and drag and drop, observers, fast line rendering, accessible text).
- [probes/tests](probes/tests): the headless probes behind the design.

## Running the probes

Div builds Skia from source; see Div's CONTRIBUTING for the toolchain. Then:

```bash
cargo test -p editor-probes -- --nocapture
```

CI runs small sizes. `PROBE_SCALE=full` runs the sizes the design reports:

```bash
PROBE_SCALE=full cargo test -p editor-probes -- --nocapture --test-threads=1
```

The dev profile builds Div and every dependency at `opt-level = 2` and the
probes unoptimized, as an app under development runs.

Div is pinned to one commit in `Cargo.toml`; `Cargo.lock` is seeded from
Div's own lock (see `div-gap: 1` there).
