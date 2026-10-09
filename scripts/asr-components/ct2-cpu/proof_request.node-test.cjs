'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const p = require('./proof_request.cjs');
const BASE = 'a'.repeat(40), PROOF = 'b'.repeat(40), OTHER = 'c'.repeat(40);
const BASE_GITHUB = '1'.repeat(40), PROOF_GITHUB = '2'.repeat(40), OLD_REQUESTS = '3'.repeat(40), NEW_REQUESTS = '4'.repeat(40);
const BLOB = '5'.repeat(40), PREVIOUS_BLOB = '6'.repeat(40), SAME = '7'.repeat(40);
const entry = (path, sha, type = 'blob', mode = type === 'tree' ? '040000' : '100644') => ({ path, sha, type, mode });
const clone = x => JSON.parse(JSON.stringify(x));
function fixture() {
  const request = { schema_version: 1, source_sha: BASE, purpose: p.PURPOSE };
  const context = { repo: { owner: 'csic21', repo: 'luma-subtitle' }, eventName: 'push', ref: `refs/heads/${p.BRANCH}`,
    sha: PROOF, payload: { deleted: false, before: BASE, after: PROOF, forced: false, repository: { full_name: p.REPOSITORY } } };
  const state = { request, commit: { sha: PROOF, parents: [{ sha: BASE }] }, head: PROOF, blob: BLOB, type: 'file',
    trees: {
      [BASE]: [entry('.github', BASE_GITHUB, 'tree'), entry('package.json', SAME)],
      [PROOF]: [entry('.github', PROOF_GITHUB, 'tree'), entry('package.json', SAME)],
      [BASE_GITHUB]: [entry('workflows', SAME, 'tree')],
      [PROOF_GITHUB]: [entry('workflows', SAME, 'tree'), entry('requests', NEW_REQUESTS, 'tree')],
      [NEW_REQUESTS]: [entry('ct2-cpu-proof.json', BLOB)],
    } };
  const data = d => ({ data: clone(d) });
  const github = { rest: { repos: {}, git: {} } };
  github.rest.repos.getCommit = async ({ ref }) => { assert.equal(ref, PROOF); return data(state.commit); };
  github.rest.repos.getContent = async ({ path, ref }) => { assert.equal(path, p.REQUEST_PATH); assert.equal(ref, PROOF);
    return data({ type: state.type, encoding: 'base64', sha: state.blob, content: Buffer.from(JSON.stringify(state.request)).toString('base64') }); };
  github.rest.git.getTree = async ({ tree_sha }) => { assert(state.trees[tree_sha]); return data({ tree: state.trees[tree_sha], truncated: state.truncated || false }); };
  github.rest.git.getRef = async ({ ref }) => { assert.equal(ref, `heads/${p.BRANCH}`); return data({ object: { sha: state.head, type: 'commit' } }); };
  return { state, github, context, prepare: () => p.prepareCpuProof({ github, context }),
    validate: () => p.validateProofCommit({ github, repo: context.repo, sha: PROOF }) };
}
test('new nested request directory tests the request commit itself with no mutation API', async () => {
  const f = fixture(); const outputs = {};
  const result = await p.prepareCpuProof({ ...f, core: { setOutput: (key, value) => { outputs[key] = value; } } });
  assert.equal(result.source_sha, PROOF); assert.equal(result.base_sha, BASE);
  assert.deepEqual(outputs, { source_sha: PROOF, base_sha: BASE });
});
test('existing request directory and prior inert request may be updated without touching siblings', async () => {
  const f = fixture(); f.state.trees[BASE_GITHUB].push(entry('requests', OLD_REQUESTS, 'tree'));
  f.state.trees[OLD_REQUESTS] = [entry('ct2-cpu-proof.json', PREVIOUS_BLOB), entry('other.json', SAME)];
  f.state.trees[NEW_REQUESTS].push(entry('other.json', SAME)); await f.prepare();
});
test('historical validation is independent of later branch advancement', async () => { const f = fixture(); f.state.head = OTHER; await f.validate(); await assert.rejects(f.prepare(), /stale/); });
for (const [label, mutate] of [
  ['extra field', f => f.state.request.publish = true], ['wrong purpose', f => f.state.request.purpose = 'publish'],
  ['wrong schema', f => f.state.request.schema_version = 2], ['non-SHA parent', f => f.state.request.source_sha = 'main'],
  ['wrong parent', f => f.state.commit.parents[0].sha = OTHER], ['multiple parents', f => f.state.commit.parents.push({ sha: OTHER })],
  ['resolved commit mismatch', f => f.state.commit.sha = OTHER], ['blob content mismatch', f => f.state.blob = OTHER],
  ['content symlink', f => f.state.type = 'symlink'], ['wrong repo', f => f.context.repo.owner = 'other'],
  ['payload fork', f => f.context.payload.repository.full_name = 'other/repo'], ['main ref', f => f.context.ref = 'refs/heads/main'],
  ['manual dispatch', f => f.context.eventName = 'workflow_dispatch'], ['deleted push', f => f.context.payload.deleted = true],
  ['bundled multi-commit push', f => f.context.payload.before = OTHER],
  ['mismatched push after', f => f.context.payload.after = OTHER],
  ['forced push', f => f.context.payload.forced = true],
  ['missing force evidence', f => delete f.context.payload.forced],
  ['stale branch', f => f.state.head = OTHER], ['truncated tree', f => f.state.truncated = true],
  ['root code change', f => f.state.trees[PROOF][1].sha = OTHER],
  ['workflow change', f => f.state.trees[PROOF_GITHUB][0].sha = OTHER],
  ['extra request sibling', f => f.state.trees[NEW_REQUESTS].push(entry('publish.json', OTHER))],
  ['executable request', f => f.state.trees[NEW_REQUESTS][0].mode = '100755'],
  ['symlink request', f => f.state.trees[NEW_REQUESTS][0].mode = '120000'],
  ['request tree', f => f.state.trees[NEW_REQUESTS][0] = entry('ct2-cpu-proof.json', BLOB, 'tree')],
  ['symlink directory', f => f.state.trees[PROOF_GITHUB][1] = entry('requests', NEW_REQUESTS, 'blob', '120000')],
  ['duplicate path', f => f.state.trees[NEW_REQUESTS].push(entry('ct2-cpu-proof.json', OTHER))],
  ['unsafe tree path', f => f.state.trees[NEW_REQUESTS].push(entry('../other', OTHER))],
]) test(`proof trigger rejects ${label}`, async () => { const f = fixture(); mutate(f); await assert.rejects(f.prepare()); });
test('unchanged request cannot trigger an arbitrary push', async () => {
  const f = fixture(); f.state.trees[BASE_GITHUB].push(entry('requests', OLD_REQUESTS, 'tree'));
  f.state.trees[OLD_REQUESTS] = [entry('ct2-cpu-proof.json', BLOB)]; await assert.rejects(f.prepare(), /changed regular/);
});
test('replacing an existing nonregular request is rejected', async () => {
  const f = fixture(); f.state.trees[BASE_GITHUB].push(entry('requests', OLD_REQUESTS, 'tree'));
  f.state.trees[OLD_REQUESTS] = [entry('ct2-cpu-proof.json', PREVIOUS_BLOB, 'blob', '120000')]; await assert.rejects(f.prepare());
});
test('oversized request is rejected before parsing', () => { assert.throws(() => p.parseProofRequest(' '.repeat(4096), {}), /bounds/); });

const fs = require('node:fs');
const path = require('node:path');
function workflowJobs() {
  const text = fs.readFileSync(path.join(__dirname, '../../../.github/workflows/asr-ct2-cpu.yml'), 'utf8');
  const section = text.slice(text.indexOf('\njobs:\n') + 7);
  const jobs = Object.fromEntries([...section.matchAll(/^  ([a-z][a-z0-9-]*):\n([\s\S]*?)(?=^  [a-z][a-z0-9-]*:\n|$(?![\s\S]))/gm)].map(m => [m[1], m[2]]));
  assert.deepEqual(Object.keys(jobs).sort(), ['request', 'source-tests', 'windows-cpu-proof', 'windows-direct-crt-signatures']);
  return { text, jobs };
}
test('PR revisions run cheap source guards and only inert request-path pushes can trigger the native gate', () => {
  const { text, jobs } = workflowJobs();
  assert(text.includes('  pull_request:'));
  assert(text.includes("  push:\n    branches: [feat/optional-asr-engines]\n    paths: ['.github/requests/ct2-cpu-proof.json']"));
  assert(jobs['source-tests'].includes('name: CPU proof source guards'));
  assert(jobs['source-tests'].includes('runs-on: ubuntu-latest'));
  assert(!/^    (if|needs):/m.test(jobs['source-tests']));
  assert(jobs['source-tests'].includes('proof_request.node-test.cjs'));
  assert(jobs.request.includes(`name: ${p.PROOF_JOB}`));
  assert(jobs.request.includes("if: github.event_name == 'push' && !inputs.source_sha"));
  assert(jobs.request.includes('needs: source-tests'));
  assert(jobs.request.includes('await prepareCpuProof({ github, context, core });'));
  assert(jobs.request.includes('source_sha: ${{ steps.request.outputs.source_sha }}'));
  assert(!/contents: write|actions: write|secrets: inherit|pull_request_target:/.test(text));
});
test('native jobs require successful guards and explicit request/manual/source route, and never start after cancellation', () => {
  const { jobs } = workflowJobs();
  const expected = "always() && !cancelled() && needs.source-tests.result == 'success' && (needs.request.result == 'success' || inputs.source_sha || github.event_name == 'workflow_dispatch')";
  for (const name of ['windows-cpu-proof', 'windows-direct-crt-signatures']) {
    const job = jobs[name], condition = job.match(/^    if: (.+)$/m)?.[1];
    assert.equal(condition, expected);
    assert(job.includes('needs: [source-tests, request]'));
    assert(job.includes('SOURCE_SHA: ${{ inputs.source_sha || needs.request.outputs.source_sha || github.sha }}'));
    assert(job.includes('ref: ${{ env.SOURCE_SHA }}'));
    // Evaluate only the exact asserted, bounded GitHub condition above.
    const check = new Function('needs', 'inputs', 'github', 'always', 'cancelled', `return Boolean(${condition.replaceAll('needs.source-tests', 'needs["source-tests"]')});`);
    const evaluate = ({ event = 'pull_request', source = '', guards = 'success', request = 'skipped', cancelled = false } = {}) =>
      check({ 'source-tests': { result: guards }, request: { result: request } }, { source_sha: source }, { event_name: event }, () => true, () => cancelled);
    assert.equal(evaluate(), false, 'ordinary PR cannot start native builds');
    assert.equal(evaluate({ event: 'push' }), false, 'arbitrary push cannot start native builds');
    assert.equal(evaluate({ event: 'push', request: 'success' }), true);
    assert.equal(evaluate({ event: 'workflow_dispatch' }), true);
    assert.equal(evaluate({ event: 'push', source: PROOF }), true, 'existing explicitly pinned reusable route remains supported');
    for (const route of [{ event: 'push', request: 'success' }, { event: 'workflow_dispatch' }, { event: 'push', source: PROOF }]) {
      assert.equal(evaluate({ ...route, guards: 'failure' }), false);
      assert.equal(evaluate({ ...route, guards: 'skipped' }), false);
      assert.equal(evaluate({ ...route, cancelled: true }), false);
    }
  }
});
test('reusable candidate export remains bound to the dedicated validated caller and exact four files', () => {
  const { jobs } = workflowJobs(), job = jobs['windows-cpu-proof'];
  const expected = "${{ inputs.publication && github.repository == 'csic21/luma-subtitle' && github.event_name == 'push' && github.ref == 'refs/heads/feat/optional-asr-engines' && github.workflow == 'Publish reviewed CPU wheel' && startsWith(github.workflow_ref, 'csic21/luma-subtitle/.github/workflows/asr-cpu-wheel-publish.yml@') }}";
  assert.equal(job.match(/^      CPU_CANDIDATE_EXPORT: (.+)$/m)?.[1], expected);
  assert(job.includes("if: success() && env.CPU_CANDIDATE_EXPORT == 'true'"));
  assert(job.includes("if ($env:CPU_PUBLICATION_REQUESTED -eq 'true' -and $env:CPU_CANDIDATE_EXPORT -ne 'true')"));
  const candidatePaths = [...job.matchAll(/^            dist\/ct2-cpu-candidate\/(.+)$/gm)].map(x => x[1]).sort();
  assert.deepEqual(candidatePaths, ['ctranslate2-4.8.2-1lumacpu-cp312-cp312-win_amd64.whl',
    'luma-ct2-cpu-4.8.2-1-notices.zip', 'luma-ct2-cpu-4.8.2-1-sources.zip', 'publication-proof.json'].sort());
  assert(job.includes('candidate_artifact_id: ${{ steps.candidate-upload.outputs.artifact-id }}'));
});
