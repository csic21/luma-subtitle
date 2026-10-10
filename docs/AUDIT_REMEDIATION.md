# 1.2.1 audit remediation

## Credential destination boundary

The backend derives a credential scope from the saved task's normalized provider and URL origin at execution. HTTPS is required except loopback HTTP. Scheme/host/effective port changes never borrow another scope's key; URL userinfo and fragments are rejected, and API redirects are disabled. A path or model change within one origin keeps the same scope. Credential values are never returned in Settings; only available scopes are exposed. The key-entry field clears after saving.

Legacy unscoped keys are not guessed from today's settings. Users may explicitly tick a destination-labelled binding choice and save, or enter a key for that destination. Binding moves the legacy row to one scoped key without overwriting an existing destination. Saved task destinations remain unchanged. SQLite storage remains plaintext by explicit product decision; same-user malware, sufficiently privileged processes and database backups remain exposure boundaries. No operating-system credential migration runs.

## OpenCode boundary

The reviewed release is [OpenCode 1.18.35](https://github.com/anomalyco/opencode/releases/tag/v1.18.35), source [53d1eabb61e21162157817bf677da0a4ad3332e3](https://github.com/anomalyco/opencode/tree/53d1eabb61e21162157817bf677da0a4ad3332e3). New versions require source review and rerunning the native adversarial proof before extending the allowlist.

Each shard uses a private temporary home/config/data/cache/work directory, a cleared environment, only the selected provider's existing api/oauth authentication, an explicit primary text-only agent, deny-all permissions, no MCP/custom plugins/external skills, and disabled default title/summary/compaction/subagent agents. The CLI's exact version, resolved config and all resolved tool decisions are checked before subtitle submission. Machine-managed configuration and unsupported auth/profile forms fail closed. Existing profiles are never modified; only the disposable session's state is removed after its process has ended. This is capability restriction within the reviewed executable, not an operating-system sandbox against a malicious replacement executable. Explicit custom commands remain trusted user-selected programs with their normal account privileges.

The production-path native proof in `scripts/test_opencode_isolation.py` downloads the exact official native executable with a pinned size/hash. A loopback dummy provider forces read/write/bash/task/MCP tool calls even though the real CLI exposes no tools. Hostile inherited config/plugins/MCP and a private-file canary must not execute or leak. A clean translation must work with exactly one model request. No live keys, external model calls or paid inference are involved. See the source [configuration loader](https://github.com/anomalyco/opencode/blob/53d1eabb61e21162157817bf677da0a4ad3332e3/packages/opencode/src/config/config.ts), [permission-filtered tools](https://github.com/anomalyco/opencode/blob/53d1eabb61e21162157817bf677da0a4ad3332e3/packages/opencode/src/cli/cmd/debug/agent.handler.ts), and [authentication loader](https://github.com/anomalyco/opencode/blob/53d1eabb61e21162157817bf677da0a4ad3332e3/packages/opencode/src/auth/index.ts).

CLI probes have 10-second/64-KiB budgets; model listing has 15-second/256-KiB budgets; translation has a 240-second/8-MiB-per-stream budget. POSIX process groups and Windows Jobs own descendants. Windows children start suspended, join the Job before execution and are resumed only afterward. Timeout, overflow, cancellation and abandoned futures terminate/reap owned children and retain private files until cleanup completes.

## Queue and cache

Queue validation requires a legacy model path only for whisper.cpp. Native task-request/snapshot/queue tests and frontend fresh-settings/create/queue tests cover both optional engines without a legacy model or executable. These are production UI-flow regression tests with controlled transports, not a physical desktop inference demonstration.

Completed subtitles have a zero-byte global cache budget: the unused cache was removed, rather than evicting a result while transcription still needed it. The operation returns its owned result directly, persists it under existing generation/cancellation rules, and drops it on completion/failure. Preview and export already read persisted artifacts on demand; edits and deletions cannot leave stale global cache copies.

Legacy runtime trust details are in [LEGACY_DEPENDENCY_TRUST.md](LEGACY_DEPENDENCY_TRUST.md). Optional runtime input pins, manifests, signing key, updater endpoint and release publication guards are preserved. Qwen/MLX/CUDA inference and performance limitations from 1.2.0 still apply.
