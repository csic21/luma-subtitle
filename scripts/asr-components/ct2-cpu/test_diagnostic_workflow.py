"""Download-free guards for the narrowly approved diagnostic artifact upload."""
from pathlib import Path
import unittest

WORKFLOW = Path(__file__).resolve().parents[3] / '.github/workflows/asr-ct2-cpu.yml'


class DiagnosticWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.text = WORKFLOW.read_text(encoding='utf-8')

    def test_export_requires_the_validated_direct_request_only(self):
        self.assertIn('diagnostic_artifact: ${{ steps.request.outputs.diagnostic_artifact }}', self.text)
        line = next(line for line in self.text.splitlines() if 'CPU_DIAGNOSTIC_EXPORT:' in line)
        for gate in ("needs.request.outputs.diagnostic_artifact == 'cpu-wheel-source-notices-1-day'",
                     "github.repository == 'csic21/luma-subtitle'", "github.event_name == 'push'",
                     "github.ref == 'refs/heads/feat/optional-asr-engines'", '!inputs.source_sha', '!inputs.publication'):
            self.assertIn(gate, line)
        header = self.text.split('permissions:', 1)[0]
        self.assertNotIn('diagnostic_artifact:', header, 'No manual/reusable diagnostic input')
        self.assertIn("$diagnosticArgs = @('--diagnostic-output', (Join-Path $env:GITHUB_WORKSPACE 'dist/ct2-cpu-diagnostic'))", self.text)

    def test_failed_capture_is_short_lived_and_has_only_four_literal_files(self):
        block = self.text.split('- name: Retain explicit failed-verifier diagnostic (never a release candidate)', 1)[1].split('- name:', 1)[0]
        condition = next(line for line in block.splitlines() if line.strip().startswith('if:'))
        for gate in ('failure()', '!cancelled()', "env.CPU_DIAGNOSTIC_EXPORT == 'true'",
                     "hashFiles('dist/ct2-cpu-diagnostic/diagnostic-manifest.json') != ''"):
            self.assertIn(gate, condition)
        for expected in ('retention-days: 1', 'overwrite: false', 'if-no-files-found: error',
                         'ct2-cpu-DIAGNOSTIC-NOT-FOR-RELEASE-${{ env.SOURCE_SHA }}-${{ github.run_id }}-${{ github.run_attempt }}'):
            self.assertIn(expected, block)
        paths = [line.strip() for line in block.split('path: |', 1)[1].splitlines() if line.strip()]
        self.assertEqual(paths, [
            'dist/ct2-cpu-diagnostic/ctranslate2-4.8.2-1lumacpu-cp312-cp312-win_amd64.whl',
            'dist/ct2-cpu-diagnostic/luma-ct2-cpu-4.8.2-1-sources.zip',
            'dist/ct2-cpu-diagnostic/luma-ct2-cpu-4.8.2-1-notices.zip',
            'dist/ct2-cpu-diagnostic/diagnostic-manifest.json',
        ])
        self.assertNotIn('publication-proof.json', block)
        self.assertNotIn('continue-on-error:', self.text)
        self.assertNotIn('actions: write', self.text)
        self.assertNotIn('contents: write', self.text)


if __name__ == '__main__':
    unittest.main()
