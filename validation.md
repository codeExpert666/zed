# Reference navigation validation

Patch: `83d5e6da867f58793595b9d649fafe0f09dff834`, based on upstream `ne96ffb0fd3cc43ec56dcff421146f5e823b13841`.

Environment: macOS 15.8 arm64; Zed Dev 1.27.0 debug; Java extension 6.8.29; JDT LS 1.63.0; Rust 1.98.1. The development keymap was empty, so `[r` / `]r` used the upstream Vim bindings. The About dialog is [captured here](about-build.png).

## Automated checks

- Final reference regressions with the fix reverted: 9 passed, 2 failed. The Java failure reproduced the wrong zero-based `(10, 15)` destination instead of `(2, 19)`.
- `cargo test -p editor -p gpui_platform --features gpui_platform/runtime_shaders --lib`: 1221 passed, 1 ignored, no failures.
- `./script/clippy -p editor -p gpui_platform`: passed (release, all targets/features, deny warnings).
- `cargo fmt -p editor -- --check` and `git diff --check`: passed.
- `cargo build -p zed --bin zed --features gpui_platform/runtime_shaders`: passed. Runtime shader compilation was used because full Xcode/Metal tooling was absent. Apple ld reported the large `__eh_frame` warning; Cargo reported the dependency `block` as future-incompatible.

## Manual checks

All positions below are one-based. The displayed source line, cursor/identifier, and status-bar row and column were checked together. Window titles were not used as position evidence.

On the exact three-file Maven reproduction from #66037, consecutive native `]r` visited the five original identifier ranges in order: `ClientWrapper.java:1:13`, `Factory.java:3:20`, `Usage.java:3:20`, `Usage.java:7:20`, `Usage.java:11:20`. The cursor stopped on the initial `C` of `ClientWrapper` at every step. Forward/reverse wraparound, `3]r`, `2[r`, and the generic Editor action also passed.

- [Previously skipped Usage line 3](after-usage-first.png)
- [Previously skipped Usage line 7](after-usage-second.png)
- [Excerpt navigation to Usage:1:1](excerpt-start.png), followed by [native next action to Usage:3:20](after-excerpt-move.png)

A separate Maven copy adds `SameLine.java`:

```java
final class SameLine {
    static Object[] make() {
        return new Object[] { new ClientWrapper(), new ClientWrapper() }; // ClientWrapper
    }
}
```

- The [first call at 3:35](after-same-line-first.png) and [second call at 3:56](after-same-line-second.png) are visited separately.
- The comment is skipped. From a [cursor explicitly on the comment at 3:90](cursor-on-comment.png), `]r` [resumes at Usage.java:3:20](after-comment.png).
- Inserting a line before the two calls moves the stops to [4:35](after-edit-first.png) and [4:56](after-edit-second.png). The edit was undone afterward.

Some captures during automated UI interaction initially contained an old rendered frame. Those were excluded; resizing the development window forced a fresh paint for the accepted captures.

## JDT LS evidence

[Extracted reference responses](jdtls-reference-results.json) were obtained through a separate stdio client using `includeDeclaration: true` against a private copy of the exact reproduction. Only URI prefixes were replaced with `/repro`; all ranges are unchanged and use LSP zero-based coordinates.

- Class declaration: 5 type-reference ranges, beginning on identifiers.
- Factory constructor call identifier: 3 constructor-reference ranges, including calls starting at `new` (one-based Factory:3:16 and Usage:11:16).
- Usage nested-type qualifier: the original 5 type references.
- Usage:11:16 (`new`): only the enclosing method name `third` at Usage:10:19, which does not contain the cursor. This accounts for the `Total references: 1` failure.

## Boundaries

Ranges retain the opened query's membership and track edits through anchors. They do not perform new semantic analysis during navigation; rerun the query after semantic membership changes. Query metadata is not newly persisted across app restarts. Regression tests use a fake language server with the captured range shapes; actual JDT LS behavior was verified separately.

This technical record and the patch were prepared with OpenAI Codex. Human review is required before upstream submission.
