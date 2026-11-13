# sample-editor

A text and code editor built on [Div](https://github.com/build-with-div/div),
to find what Div needs for professional desktop editors. Div is a native Rust
UI framework with no webview; this sample uses only its public API and marks
each workaround with a `// div-gap: <n>` comment.

Status: **phase 1, investigation.** There is no editor yet.

- [docs/design.md](docs/design.md): what Div offers an editor today, the
  measurements, the recommended architecture and the Div work it needs.
- [docs/rfc-text.md](docs/rfc-text.md): draft Div APIs for text geometry,
  editing input (EditContext), highlights, accessible text and measurement.
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
