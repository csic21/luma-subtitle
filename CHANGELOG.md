# Changelog

## 1.1.15 — 2026-10-09

### Subtitle editing
- Search subtitle text, jump to cue IDs, compare source and translation, and edit translated cues without changing IDs or timing.
- Save pending task settings before starting an operation, prevent duplicate submissions, and refresh previews when result content changes.

### Reliable jobs and exports
- Version results and guard publication by the current task attempt so cancelled or obsolete work cannot replace newer results.
- Invalidate translations after retranscription or translation-setting changes. Preserve previous exports and roll back new export files when publication fails.
- Make API, CLI, and local-model translation cancellation responsive, and clean up owned processes and output readers.

### Translation and transcription
- Refill bounded translation slots as each shard finishes, checkpoint completed shards immediately, and retry transient API failures with a limited budget.
- Accept valid translated dialogue containing phrases such as “I cannot” without unnecessary retry; reject duplicate IDs and blank translations.
- Report transcription progress, elapsed time, timing breakdowns, and the observed inference backend. Avoid oversubscribing low-core machines.
- Fix the macOS Whisper source-release URL and response contract.

### Validation and compatibility
- Add Windows/macOS pull-request checks and focused lifecycle, parser, editing, and cancellation regressions.
- API-key storage, updater keys, and model-quality defaults are unchanged.
- Actual inference-speed gains have not been benchmarked on user hardware. Native desktop UI and visual checks were not performed in this validation environment.
