# Changelog

## 1.2.0 — 2026-10-10

### Optional local transcription engines
- Add experimental Whisper acceleration with faster-whisper on CPU/CUDA and MLX Whisper on native Apple Silicon. Keep the existing whisper.cpp engine and GGML/Turbo models as the default path.
- Add experimental Qwen3-ASR 0.6B/1.7B with a separate mandatory Qwen3-ForcedAligner model for validated subtitle timestamps. The Qwen adapter supports CPU/CUDA; macOS Qwen uses CPU.
- Reuse an optional engine's loaded model between jobs, report the observed device and cold/warm timings, and release its process and memory on cancellation or shutdown.
- Add local capability/model checks, actionable setup errors, storage estimates, and forced-alignment/CJK subtitle validation.

### Setup and compatibility
- Add user-triggered installation of verified private engine components and separate model downloads, with size/storage information, progress, cancellation, repair, removal and atomic activation. No user-installed Python, terminal commands, system PATH changes or automatic first-launch downloads.
- Assemble exact pinned binary wheels with the app's private pinned pip, fully offline, after the user reviews sources and applicable terms. The published Windows CPU wheel was source-built; app setup downloads that pinned wheel and does not compile or resolve dependencies over the network.
- Enable all four opt-in experimental managed recipes with exact input pins and applicable terms. Windows Whisper uses the separately published CPU-only CTranslate2 wheel with its full native notices and one-thread cleanup policy; no vendor CUDA/MKL/OpenMP wheel is substituted.
- Managed components provide Windows Whisper CPU, Apple Silicon MLX Whisper/Metal, and Qwen CPU on both platforms. They do not include CUDA; advanced external runtimes remain optional. Failed setup retains the previous component and never deletes external models.
- Existing settings, downloaded GGML models, results, subtitle import/export and updater configuration are preserved. Existing users do not need Python or receive optional-runtime requirements unless they select a new engine.
- Qwen CPU keeps both ASR and aligner weights resident as float32. The 0.6B pair needs roughly 7.5 GB for weights alone, plus runtime/audio/activation memory; do not assume an 8 GB device can run it.

### Validation boundary
- Windows/macOS regression coverage includes the optional worker protocol, cancellation, shutdown, stale-result handling, Unicode paths and legacy defaults.
- A real Linux CPU faster-whisper tiny-model smoke passed transcription, timed SRT/export, warm reuse, cancellation and recovery. Actual Qwen package imports/API compatibility were checked without loading Qwen weights.
- All four native runtime jobs passed at source `401c344b6e9786ba52af280639b57b7c3ed27dbf` in [run 38026666599](https://github.com/csic21/luma-subtitle/actions/runs/38026666599), including private install/repair/cancel/remove. A real Metal tensor check passed; MLX speech inference remains unverified.
- The published Windows CPU wheel passed fresh fixed-path byte repeatability. Native reference checks passed Tiny/JFK cold/warm timed inference, model switch, unload and normal EOF under the one-thread policy. Actual Rust receipt-selected tests passed SRT/export, active cancellation/recovery and process/lease cleanup under a test-owned managed root/catalog. Global embedded-catalog path selection and full desktop UI/queue behavior remain untested.
- Exact-final-source app checks, all four native runtime jobs and independent review remain separate gates before merge and the 1.2.0 application release.
- A bounded Windows CPU Qwen3-ASR 0.6B + ForcedAligner-0.6B proof passed at source `401c344b6e9786ba52af280639b57b7c3ed27dbf` in [run 38028126893](https://github.com/csic21/luma-subtitle/actions/runs/38028126893). The exact pinned 3,720,689,099-byte model pair downloaded successfully, and the 11-second JFK fixture produced three valid timestamped segments. Cold-worker elapsed was 36.875 seconds, including startup/model loading, ASR/alignment and process completion, excluding setup/downloads. Resource gating and owned cache/private-output cleanup passed. This is a single-host functional check, not a real-time or performance claim.
- The earlier bounded Windows Qwen attempt at source `0875deb4430ac84de1fabc80a9f4454589e13fba` in [run 38024179671](https://github.com/csic21/luma-subtitle/actions/runs/38024179671) passed fresh native readiness and resource gates, downloaded five metadata/tokenizer files (1,736,805 bytes), then rejected the first model-weight redirect before inference. Owned child/cache/private-output cleanup was confirmed; the exact rejected host was not recorded. The two-host CDN compatibility correction was independently documented; the later successful run does not establish the earlier rejected host.
- Qwen 1.7B/macOS inference, warm reuse, Qwen managed-worker lifecycle/cancellation/export/GUI, general memory fit and peak memory, MLX speech and CUDA inference/GPU performance, representative quality/speed comparisons, and a full native desktop UI/queue smoke remain unverified. New engines remain opt-in and experimental; no speed or quality improvement is promised.

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
