# Desktop UI preview

An isolated developer/QA entry for the actual React router, task queue/detail,
subtitle editor, settings, and production CSS. Only Tauri API/event and updater
boundaries are replaced. No production entry or package script imports this fixture.

All paths, tasks, statuses and subtitle cues are synthetic. Subtitle/settings edits
are in memory only. File picking, import, deletion, processing/export, downloads,
installation, credentials, native path opening, and relaunch are blocked. ASR
catalog data permits inspection of the real consent UI but cannot install anything
or persist acceptance. A synthetic ready status does not demonstrate native readiness.

Using already-installed dependencies:

```sh
./node_modules/.bin/vite --config scripts/ui-preview/vite.config.ts
```

Open `http://127.0.0.1:1433/?width=1100&height=700&locale=zh-CN&scenario=completed&page=detail`.
The actual application lives inside `#audit-frame`, with the exact CSS viewport
specified by width/height. Harness resize buttons preserve iframe/React state.
`locale=en-US` checks longer labels; `page=queue` and `page=settings` select routes.
Scenarios: completed, running, failed, loading, empty and long.

The fixture sets its requested initial hash before dynamically loading `App`.
This lets the real router index the first route correctly, so browser Back from
the first opened task exercises the unsaved-edit guard. The parent harness does
not construct a second application router.

`present=1` replaces QA controls with a 28px `luma · 示例数据 / Sample data`
label for clean captures. It does not change the iframe's production CSS or dimensions.

Static preview:

```sh
./node_modules/.bin/vite build --config scripts/ui-preview/vite.config.ts
python3 -m http.server 1433 --bind 127.0.0.1 --directory scripts/ui-preview/.preview-dist
```

The ignored `.preview-dist` output is separate from production `dist`. Native
Windows/Wry, engine, media, credential, and installer behavior require native gates.
Browser fixture results make no claim about them.
