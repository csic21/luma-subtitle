# Changelog

## 1.2.0 — 2026-10-09

### Optional local transcription engines
- Add experimental Whisper acceleration with faster-whisper on CPU/CUDA and MLX Whisper on native Apple Silicon. Keep the existing whisper.cpp engine and GGML/Turbo models as the default path.
- Add experimental Qwen3-ASR 0.6B/1.7B with a separate mandatory Qwen3-ForcedAligner model for validated subtitle timestamps. The Qwen adapter supports CPU/CUDA; macOS Qwen uses CPU.
- Reuse an optional engine's loaded model between jobs, report the observed device and cold/warm timings, and release its process and memory on cancellation or shutdown.
- Add local capability/model checks, actionable setup errors, storage estimates, and forced-alignment/CJK subtitle validation.

### Setup and compatibility
- Optional engines require a separately prepared Python environment and complete local model directories. Luma does not install these packages or download model weights automatically.
- Existing settings, downloaded GGML models, results, subtitle import/export and updater configuration are preserved. Existing users do not need Python or receive optional-runtime requirements unless they select a new engine.
- Qwen CPU keeps both ASR and aligner weights resident as float32. The 0.6B pair needs roughly 7.5 GB for weights alone, plus runtime/audio/activation memory; do not assume an 8 GB device can run it.

### Validation boundary
- Windows/macOS regression coverage includes the optional worker protocol, cancellation, shutdown, stale-result handling, Unicode paths and legacy defaults.
- A real Linux CPU faster-whisper tiny-model smoke passed transcription, timed SRT/export, warm reuse, cancellation and recovery. Actual Qwen package imports/API compatibility were checked without loading Qwen weights.
- Qwen inference and memory fit, Metal/CUDA inference and performance, representative quality/speed comparisons, and a full native desktop UI/queue smoke remain unverified. New engines remain opt-in and experimental; no speed or quality improvement is promised.

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
