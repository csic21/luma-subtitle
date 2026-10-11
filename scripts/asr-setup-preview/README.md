# ASR setup developer preview

This is an isolated development/QA entry point. It is not part of the shipping app,
its package scripts, or production Vite entry. It renders the actual
`ManagedAsrSetup`, setup dialog, component hook, setup queue, locales, and styles.
Only the Tauri command/event boundary is replaced with in-memory fixture state.

Run from the repository root using already installed dependencies:

```sh
./node_modules/.bin/vite --config scripts/asr-setup-preview/vite.config.ts
```

Open `http://localhost:1427/?scenario=runtime-installed&width=400` or change width
to `1100`. Each iframe has a true CSS viewport of that width and a height of 800px.
The production component receives no styling overrides. The surrounding label and
controls identify this as a developer fixture. Add `locale=en-US` for English.

Scenarios: `runtime-installed` uses a verified runtime and two missing models;
`fresh` has no components; `ready` has all three. Catalog data and download sizes
come from the checked-in native catalog. Plan hashes are intentionally synthetic,
all-zero fixture values; they are not backend-generated receipts. Runtime install
is blocked, consent is never recorded, and the fresh scenario is for testing the
initial unchecked terms dialog only.

For progress, error, cancellation, and sequential-completion states: use
`runtime-installed`, click the production one-click button and confirm its model
list, then operate the clearly marked developer controls. Each model command
remains pending until Complete, Fail, or the production Cancel is clicked. The
command log verifies sequence and retry inventory skipping. These controls change
only the in-memory model. No downloads, native installers, real consent acceptance,
filesystem components, user settings, or external service writes occur.
