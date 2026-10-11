/* Test-only WebView page. These are the real generated command IPC wrappers. */
(async () => {
  const { invoke, transformCallback } = window.__TAURI_INTERNALS__;
  const report = { results: [], snapshots: [], events: [], fatal: null };
  try {
    const plan = await invoke('native_test_plan');
    if (plan.metricsOnly) {
      await invoke('native_test_measure');
    } else {
      for (const event of ['model-download-event', 'dependency-install-event']) {
        await invoke('plugin:event|listen', {
          event,
          target: { kind: 'Any' },
          handler: transformCallback((message) => report.events.push({ event, ...message.payload })),
        });
      }
      const install = async () => {
        try {
          return { ok: true, value: await invoke(plan.command, plan.args) };
        } catch (error) {
          return { ok: false, error: String(error) };
        }
      };
      if (plan.concurrent) {
        report.results = await Promise.all([install(), install()]);
        report.snapshots.push(await invoke('download_status'));
      } else {
        for (let index = 0; index < plan.iterations; index += 1) {
          if (index) await invoke('native_test_followup');
          report.results.push(await install());
          report.snapshots.push(await invoke('download_status'));
        }
      }
      // Let any already queued WebView event callbacks run, then compare them to
      // the command result and the backend's authoritative terminal snapshot.
      await new Promise((resolve) => setTimeout(resolve, 100));
      report.finalSnapshot = await invoke('download_status');
    }
  } catch (error) {
    report.fatal = String(error && error.stack || error);
  }
  await invoke('native_test_finish', { report });
})();
