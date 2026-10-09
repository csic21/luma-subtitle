'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { Readable } = require('node:stream');
const p = require('./diagnostic_replay.cjs');
const BASE = 'a'.repeat(40), REPLAY = 'b'.repeat(40), OTHER = 'c'.repeat(40);
const G1 = '1'.repeat(40), G2 = '2'.repeat(40), R1 = '3'.repeat(40), R2 = '4'.repeat(40), BLOB = '5'.repeat(40), OLD = '6'.repeat(40), SAME = '7'.repeat(40);
const clone = x => JSON.parse(JSON.stringify(x));
const entry = (path, sha, type = 'blob', mode = type === 'tree' ? '040000' : '100644') => ({ path, sha, type, mode });
// Test parsing only: Windows checkout line endings must not alter source bytes.
const normalizeWorkflow = text => text.replace(/\r\n/g, '\n');
const readWorkflow = (name = 'asr-ct2-cpu-replay.yml') => normalizeWorkflow(fs.readFileSync(path.join(__dirname, '../../../.github/workflows', name), 'utf8'));
function requiredMarker(text, marker) {
  const index = text.indexOf(marker);
  assert(index >= 0, `Required workflow marker missing: ${JSON.stringify(marker)}`);
  assert.equal(text.indexOf(marker, index + marker.length), -1, `Duplicate workflow marker: ${JSON.stringify(marker)}`);
  return index;
}
function fixture() {
  const request = { schema_version: 1, purpose: p.PURPOSE, source_sha: BASE,
    producer: clone(p.PRODUCER), artifact: clone(p.ARTIFACT), manifest: clone(p.MANIFEST) };
  const context = { repo: { owner: 'csic21', repo: 'luma-subtitle' }, eventName: 'push', ref: `refs/heads/${p.BRANCH}`,
    sha: REPLAY, payload: { repository: { full_name: p.REPOSITORY }, deleted: false, before: BASE, after: REPLAY, forced: false } };
  const state = { request, commit: { sha: REPLAY, parents: [{ sha: BASE }] }, blob: BLOB, head: REPLAY, type: 'file',
    trees: { [BASE]: [entry('.github', G1, 'tree'), entry('unchanged', SAME)],
      [REPLAY]: [entry('.github', G2, 'tree'), entry('unchanged', SAME)],
      [G1]: [entry('workflows', SAME, 'tree'), entry('requests', R1, 'tree')],
      [G2]: [entry('workflows', SAME, 'tree'), entry('requests', R2, 'tree')],
      [R1]: [entry('ct2-cpu-proof.json', SAME)],
      [R2]: [entry('ct2-cpu-proof.json', SAME), entry('ct2-cpu-replay.json', BLOB)] } };
  const data = value => ({ data: clone(value) });
  const github = { rest: { repos: {}, git: {} } };
  github.rest.repos.getCommit = async ({ ref }) => { assert.equal(ref, REPLAY); return data(state.commit); };
  github.rest.repos.getContent = async ({ path, ref }) => { assert.equal(path, p.REQUEST_PATH); assert.equal(ref, REPLAY);
    return data({ type: state.type, encoding: 'base64', sha: state.blob, content: Buffer.from(JSON.stringify(state.request)).toString('base64') }); };
  github.rest.git.getTree = async ({ tree_sha }) => { assert(state.trees[tree_sha]); return data({ tree: state.trees[tree_sha], truncated: state.truncated || false }); };
  github.rest.git.getRef = async ({ ref }) => { assert.equal(ref, `heads/${p.BRANCH}`); return data({ object: { type: 'commit', sha: state.head } }); };
  return { state, context, github, prepare: () => p.prepareReplayRequest({ github, context }) };
}
test('only an inert sole-parent request pins the known original artifact and distinct verifier source', async () => {
  const f = fixture(), outputs = {};
  const result = await p.prepareReplayRequest({ ...f, core: { setOutput: (name, value) => { outputs[name] = value; } } });
  assert.equal(result.verifier_source_sha, REPLAY); assert.equal(result.base_sha, BASE);
  assert.deepEqual(outputs, { verifier_source_sha: REPLAY, base_sha: BASE });
  assert.notEqual(result.verifier_source_sha, result.request.producer.source_sha);
});
for (const [label, mutate] of [
  ['repository', f => f.context.repo.owner = 'fork'], ['payload repository', f => f.context.payload.repository.full_name = 'fork/repo'],
  ['branch', f => f.context.ref = 'refs/heads/main'], ['PR', f => f.context.eventName = 'pull_request'],
  ['manual', f => f.context.eventName = 'workflow_dispatch'], ['deleted', f => f.context.payload.deleted = true],
  ['forced push', f => f.context.payload.forced = true], ['missing push force field', f => delete f.context.payload.forced],
  ['bundled push', f => f.context.payload.before = OTHER], ['push after', f => f.context.payload.after = OTHER],
  ['live branch advanced', f => f.state.head = OTHER], ['resolved commit', f => f.state.commit.sha = OTHER],
  ['merge parents', f => f.state.commit.parents.push({ sha: OTHER })], ['request parent', f => f.state.request.source_sha = OTHER],
  ['moving source', f => f.state.request.source_sha = 'main'], ['schema', f => f.state.request.schema_version = 2],
  ['purpose', f => f.state.request.purpose = 'publish'], ['generic override', f => f.state.request.workflow = 'arbitrary.yml'],
  ['different producer source', f => f.state.request.producer.source_sha = OTHER], ['different run', f => f.state.request.producer.run_id++],
  ['different attempt', f => f.state.request.producer.run_attempt++], ['different job', f => f.state.request.producer.job_id++],
  ['different artifact', f => f.state.request.artifact.id++], ['artifact bytes', f => f.state.request.artifact.bytes++],
  ['artifact hash', f => f.state.request.artifact.sha256 = '0'.repeat(64)], ['manifest hash', f => f.state.request.manifest.sha256 = '0'.repeat(64)],
  ['content type', f => f.state.type = 'symlink'], ['Git blob mismatch', f => f.state.blob = OTHER],
  ['truncated tree', f => f.state.truncated = true], ['root change', f => f.state.trees[REPLAY][1].sha = OTHER],
  ['workflow change', f => f.state.trees[G2][0].sha = OTHER], ['proof request changed', f => f.state.trees[R2][0].sha = OTHER],
  ['extra file', f => f.state.trees[R2].push(entry('other', OTHER))],
  ['symlink request', f => f.state.trees[R2][1].mode = '120000'], ['executable request', f => f.state.trees[R2][1].mode = '100755'],
  ['symlink ancestor', f => f.state.trees[G2][1].type = 'blob'], ['duplicate entry', f => f.state.trees[R2].push(entry('ct2-cpu-replay.json', OTHER))],
  ['traversal path', f => f.state.trees[R2].push(entry('../outside', OTHER))],
]) test(`replay request rejects ${label}`, async () => { const f = fixture(); mutate(f); await assert.rejects(f.prepare()); });
test('updating one prior inert request is allowed, unchanged or nonregular prior request is rejected', async () => {
  const f = fixture(); f.state.trees[R1].push(entry('ct2-cpu-replay.json', OLD)); await f.prepare();
  f.state.trees[R1][1].sha = BLOB; await assert.rejects(f.prepare());
  f.state.trees[R1][1] = entry('ct2-cpu-replay.json', OLD, 'blob', '120000'); await assert.rejects(f.prepare());
});
test('oversized and malformed request fail before any execution', () => {
  assert.throws(() => p.parseRequest(' '.repeat(4096), {}), /bounds/);
  assert.throws(() => p.parseRequest('{', {}));
});
function producer() {
  const run = { id: p.PRODUCER.run_id, run_attempt: 1, head_sha: p.PRODUCER.source_sha,
    repository: { full_name: p.REPOSITORY, id: p.REPOSITORY_ID }, head_repository: { full_name: p.REPOSITORY, id: p.REPOSITORY_ID },
    event: 'push', head_branch: p.BRANCH, path: '.github/workflows/asr-ct2-cpu.yml', status: 'completed', conclusion: 'failure',
    pull_requests: [{ head: { sha: OTHER } }] };
  const now = Date.now(), started = new Date(now - 600000).toISOString(), completed = new Date(now - 2000).toISOString();
  const jobs = { total_count: 4, jobs: ['CPU proof source guards', 'Validate request-only CPU proof', 'windows-cpu-proof', 'windows-direct-crt-signatures'].map((name, index) => ({
    id: name === 'windows-cpu-proof' ? p.PRODUCER.job_id : index + 1, name, head_sha: p.PRODUCER.source_sha,
    run_id: p.PRODUCER.run_id, status: 'completed', conclusion: name === 'windows-cpu-proof' ? 'failure' : 'success', started_at: started, completed_at: completed,
    steps: [ ['Test input locks and binary inventory logic', 'success'], ['Inventory official toolchain and build two CPU wheels', 'failure'],
      ['Retain explicit failed-verifier diagnostic (never a release candidate)', 'success'], ['Export the three reviewed candidate assets and proof only', 'skipped'] ]
      .map(([name, conclusion]) => ({ name, conclusion, status: 'completed' })) })) };
  const cpu = jobs.jobs[2];
  const artifact = { id: p.ARTIFACT.id, name: p.ARTIFACT_NAME, expired: false, size_in_bytes: p.ARTIFACT.bytes, digest: `sha256:${p.ARTIFACT.sha256}`,
    archive_download_url: `https://api.github.com/repos/${p.REPOSITORY}/actions/artifacts/${p.ARTIFACT.id}/zip`,
    workflow_run: { id: p.PRODUCER.run_id, head_sha: p.PRODUCER.source_sha, head_branch: p.BRANCH,
      repository_id: p.REPOSITORY_ID, head_repository_id: p.REPOSITORY_ID },
    created_at: new Date(now - 5000).toISOString(), expires_at: new Date(now + 3600000).toISOString() };
  return { run, jobs, cpu, artifact, now };
}
test('producer remains failed and uses immutable run/job SHAs rather than a later PR head', () => {
  const f = producer(); assert.equal(p.validateProducer(f.run, f.jobs), f.cpu);
  assert.equal(p.validateArtifact(f.artifact, f.cpu, f.now), f.artifact);
});
for (const [label, mutate] of [
  ['successful producer relabel', f => f.run.conclusion = 'success'], ['run SHA', f => f.run.head_sha = OTHER],
  ['attempt', f => f.run.run_attempt = 2], ['repository', f => f.run.repository.id++], ['fork', f => f.run.head_repository.full_name = 'fork/repo'],
  ['event', f => f.run.event = 'workflow_dispatch'], ['workflow path', f => f.run.path = 'other.yml'],
  ['missing job', f => f.jobs.jobs.pop()], ['job head', f => f.cpu.head_sha = OTHER], ['job ID', f => f.cpu.id++],
  ['job success', f => f.cpu.conclusion = 'success'], ['failed source guard', f => f.jobs.jobs[0].conclusion = 'failure'],
  ['missing artifact upload step', f => f.cpu.steps[2].conclusion = 'skipped'], ['publication executed', f => f.cpu.steps[3].conclusion = 'success'],
]) test(`producer provenance rejects ${label}`, () => { const f = producer(); mutate(f); assert.throws(() => p.validateProducer(f.run, f.jobs)); });
for (const [label, mutate] of [
  ['artifact ID', f => f.artifact.id++], ['name', f => f.artifact.name += '-other'], ['expired', f => f.artifact.expired = true],
  ['bytes', f => f.artifact.size_in_bytes++], ['digest', f => f.artifact.digest = 'sha256:'+'0'.repeat(64)],
  ['origin SHA', f => f.artifact.workflow_run.head_sha = OTHER], ['origin run', f => f.artifact.workflow_run.id++],
  ['fork artifact', f => f.artifact.workflow_run.head_repository_id++], ['download route', f => f.artifact.archive_download_url = 'https://example.com'],
  ['retention extension', f => f.artifact.expires_at = new Date(f.now+2*86400000).toISOString()],
  ['expired timestamp', f => f.artifact.expires_at = new Date(f.now-1).toISOString()],
  ['created outside producer job', f => f.artifact.created_at = new Date(f.now-900000).toISOString()],
  ['missing timestamp', f => delete f.cpu.completed_at],
]) test(`artifact provenance rejects ${label}`, () => { const f = producer(); mutate(f); assert.throws(() => p.validateArtifact(f.artifact, f.cpu, f.now)); });
const response = (chunks, length = null) => ({ headers: { get: name => name === 'content-length' ? length : null }, body: Readable.from(chunks) });
test('stream verifier enforces both size and SHA before exposing archive bytes', async () => {
  const bytes = Buffer.from('verified fixture'), pin = { bytes: bytes.length, sha256: p.hash(bytes) };
  assert.deepEqual(await p.readPinnedArchive(response([bytes.subarray(0, 3), bytes.subarray(3)]), pin), bytes);
  await assert.rejects(p.readPinnedArchive(response([bytes], '999'), pin), /header/);
  await assert.rejects(p.readPinnedArchive(response([bytes, Buffer.from('extra')]), pin), /exceeds/);
  await assert.rejects(p.readPinnedArchive(response([bytes.subarray(1)]), pin), /bytes\/hash/);
  await assert.rejects(p.readPinnedArchive(response([Buffer.alloc(bytes.length)]), pin), /bytes\/hash/);
});
test('signed redirect must be official HTTPS storage with no credentials, port, fragment or further redirect', () => {
  assert.equal(p.artifactLocation('https://productionresultssa10.blob.core.windows.net/container/artifact?sig=value').protocol, 'https:');
  for (const url of ['http://productionresultssa10.blob.core.windows.net/a', 'https://example.com/a',
    'https://productionresultssa10.blob.core.windows.net.evil.test/a', 'https://user:secret@x.blob.core.windows.net/a',
    'https://x.blob.core.windows.net:8443/a', 'https://x.blob.core.windows.net/a#fragment']) assert.throws(() => p.artifactLocation(url));
});
test('download permission error is propagated unchanged without fallback or output', async t => {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'ct2-replay-'))); t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const destination = path.join(root, 'diagnostic.zip'), denied = Object.assign(new Error('Resource not accessible by integration'), { status: 403 });
  let count = 0;
  await assert.rejects(p.downloadExactArchive({ github: { request: async (route, args) => {
    count++; assert.equal(args.artifact_id, p.ARTIFACT.id); assert.equal(args.request.redirect, 'manual'); throw denied;
  } }, repo: { owner: 'csic21', repo: 'luma-subtitle' }, destination, fetchImpl: () => { throw new Error('must not fetch'); } }), error => error === denied);
  assert.equal(count, 1); assert.equal(fs.existsSync(destination), false);
});
test('storage fetch never receives API authorization and changed bytes leave no output', async t => {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'ct2-replay-'))); t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const destination = path.join(root, 'diagnostic.zip');
  const github = { request: async () => ({ status: 302, headers: { location: 'https://productionresultssa10.blob.core.windows.net/a?sig=temporary' } }) };
  await assert.rejects(p.downloadExactArchive({ github, repo: { owner: 'csic21', repo: 'luma-subtitle' }, destination, fetchImpl: async (url, options) => {
    assert.equal(options.redirect, 'error'); assert.equal(options.credentials, 'omit'); assert.equal(options.headers, undefined);
    return { ...response([Buffer.from('wrong')]), ok: true };
  } }), /bytes\/hash/);
  assert.equal(fs.existsSync(destination), false);
});
test('existing archive is never overwritten', async t => {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'ct2-replay-'))); t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const destination = path.join(root, 'diagnostic.zip'); fs.writeFileSync(destination, 'keep');
  await assert.rejects(p.downloadExactArchive({ github: {}, repo: { owner: 'csic21', repo: 'luma-subtitle' }, destination }), /fresh/);
  assert.equal(fs.readFileSync(destination, 'utf8'), 'keep');
});
function assertWorkflowGuards(raw) {
  const text = normalizeWorkflow(raw);
  assert(text.includes("paths: ['.github/requests/ct2-cpu-replay.json']"));
  assert(!/workflow_dispatch:|workflow_call:|pull_request_target:|contents: write|actions: write|secrets: inherit|id-token:/.test(text));
  assert.equal((text.match(/actions: read/g) || []).length, 1);
  assert(text.includes("if: ${{ !cancelled() && github.event_name == 'push' && needs.request.result == 'success' }}"));
  assert(text.includes('ref: ${{ env.VERIFIER_SOURCE_SHA }}'));
  assert(text.includes('retrieveReplayArtifact({ github, context, core,'));
  assert(text.includes('--verifier-source-sha "$env:VERIFIER_SOURCE_SHA"'));
  assert(!text.includes('candidate_artifact_id'));
  assert(text.includes('retention-days: 1'));
  const uploads = [...text.matchAll(/path: \|\n((?:            [^\n]+\n?)+)/g)].map(m => m[1]);
  assert.equal(uploads.length, 1);
  assert.equal(uploads[0].trim(), '${{ runner.temp }}/ct2-replay-controller/retrieval.json\n            ${{ runner.temp }}/ct2-replay-reports/**/*.json\n            ${{ runner.temp }}/ct2-replay-reports/**/*.log');
  for (const [, ref] of text.matchAll(/uses: ([^\s]+)/g)) assert(/@[a-f0-9]{40}$/.test(ref), ref);
}
test('workflow grants Actions read only to the explicit retrieval proof, with metadata-only outputs', () => assertWorkflowGuards(readWorkflow()));
test('replay retrieval metadata remains ineligible for the unchanged publisher', () => {
  const publisher = require('./publish.cjs');
  assert.throws(() => publisher.validateProof({ schema_version: 1, kind: 'ct2-cpu-diagnostic-retrieval',
    publication_authorized: false, verifier_source_sha: REPLAY, original_build: p.PRODUCER }, {}, {}));
});

test('CPU proof and publisher github-script pins use the independently verified official v7.0.1 commit', () => {
  const official = 'actions/github-script@60a0d83039c74a4aee543508d2ffcb1c3799cdea';
  for (const [name, count] of [['asr-ct2-cpu.yml', 1], ['asr-cpu-wheel-publish.yml', 2], ['asr-ct2-cpu-replay.yml', 2]]) {
    const text = readWorkflow(name);
    const pins = [...text.matchAll(/uses: (actions\/github-script@\S+)/g)].map(match => match[1]);
    assert.deepEqual(pins, Array(count).fill(official));
  }
});

function cleanupScript(raw = readWorkflow()) {
  const workflow = normalizeWorkflow(raw);
  const upload = requiredMarker(workflow, '      - name: Upload bounded replay metadata');
  const start = requiredMarker(workflow, '      - name: Remove only owned replay private inputs');
  assert(start > upload, 'Cleanup must follow metadata upload');
  const cleanup = workflow.slice(start);
  const condition = requiredMarker(cleanup, '        if: always()\n');
  const timeout = requiredMarker(cleanup, '        timeout-minutes: 5\n');
  const shell = requiredMarker(cleanup, '        shell: python\n');
  const marker = '        run: |\n', run = requiredMarker(cleanup, marker);
  assert(condition < timeout && timeout < shell && shell < run, 'Cleanup headers must precede its Python body');
  const lines = cleanup.slice(run + marker.length).split('\n');
  assert(lines.every(line => line === '' || line.startsWith('          ')), 'Unexpected YAML after cleanup body');
  const script = lines.map(line => line.replace(/^          /, '')).join('\n');
  assert(script.trim(), 'Cleanup Python body must not be empty');
  // Compile before every execution, especially failures: invalid extraction must
  // never masquerade as a passing containment/drain negative test.
  const compiled = require('node:child_process').spawnSync('python', ['-B', '-c', "import sys; compile(sys.stdin.read(), '<workflow-cleanup>', 'exec')"], { input: script, encoding: 'utf8' });
  assert.equal(compiled.status, 0, compiled.stderr || String(compiled.error));
  return script;
}
test('LF and CRLF workflow inputs have identical guards and executable cleanup', t => {
  const { spawnSync } = require('node:child_process');
  const lf = readWorkflow(), crlf = lf.replace(/\n/g, '\r\n');
  assert(crlf.includes('\r\n'));
  assertWorkflowGuards(lf); assertWorkflowGuards(crlf);
  assert.equal(cleanupScript(lf), cleanupScript(crlf));
  for (const workflow of [lf, crlf]) {
    const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'ct2-newlines-'))); t.after(() => fs.rmSync(root, { recursive: true, force: true }));
    fs.mkdirSync(path.join(root, 'ct2-replay-input')); fs.writeFileSync(path.join(root, 'ct2-replay-input', 'owned'), 'fixture');
    fs.mkdirSync(path.join(root, 'unrelated')); fs.writeFileSync(path.join(root, 'unrelated', 'keep'), 'unchanged');
    const result = spawnSync('python', ['-B', '-c', cleanupScript(workflow)], { env: { ...process.env, RUNNER_TEMP: root }, encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(fs.existsSync(path.join(root, 'ct2-replay-input')), false);
    assert.equal(fs.readFileSync(path.join(root, 'unrelated', 'keep'), 'utf8'), 'unchanged');
  }
});
test('cleanup extraction rejects missing, duplicate, reordered or invalid Python markers before execution', () => {
  const raw = readWorkflow(), title = '      - name: Remove only owned replay private inputs';
  const start = requiredMarker(raw, title), prefix = raw.slice(0, start), cleanup = raw.slice(start);
  for (const marker of [title, '        if: always()\n', '        timeout-minutes: 5\n', '        shell: python\n', '        run: |\n']) {
    assert.throws(() => cleanupScript(prefix + cleanup.replace(marker, '')), /Required workflow marker missing/);
    assert.throws(() => cleanupScript(prefix + cleanup.replace(marker, marker + marker)), /Duplicate workflow marker/);
  }
  assert.throws(() => cleanupScript(prefix + cleanup.replace('        shell: python\n        run: |\n', '        run: |\n        shell: python\n')), /headers must precede/);
  assert.throws(() => cleanupScript(prefix + cleanup.replace('          import json, os, shutil, stat', '          this is not valid Python :')), /SyntaxError/);
});
test('final cleanup removes only exact owned private roots and preserves reports/checkouts', t => {
  const { spawnSync } = require('node:child_process');
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'ct2-cleanup-'))); t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const owned = ['ct2-replay-private','ct2-replay-downloads','ct2-replay-input','ct2-replay-rust-fixture'];
  for (const name of [...owned, 'ct2-replay-reports', 'ct2-replay-controller', 'unrelated']) {
    fs.mkdirSync(path.join(root, name)); fs.writeFileSync(path.join(root,name,'keep'),name);
  }
  const result = spawnSync('python', ['-B', '-c', cleanupScript()], { env: { ...process.env, RUNNER_TEMP: root }, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  for (const name of owned) assert.equal(fs.existsSync(path.join(root,name)),false);
  for (const name of ['ct2-replay-reports','ct2-replay-controller','unrelated']) assert.equal(fs.readFileSync(path.join(root,name,'keep'),'utf8'), name);
});
test('final cleanup rejects aliased private roots before deleting any target', t => {
  const { spawnSync } = require('node:child_process');
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'ct2-cleanup-'))); t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const outside = path.join(root,'outside'); fs.mkdirSync(outside); fs.writeFileSync(path.join(outside,'keep'),'safe');
  fs.mkdirSync(path.join(root,'ct2-replay-private')); fs.writeFileSync(path.join(root,'ct2-replay-private','keep'),'untouched');
  fs.symlinkSync(outside,path.join(root,'ct2-replay-downloads'),process.platform==='win32'?'junction':'dir');
  const result = spawnSync('python', ['-B', '-c', cleanupScript()], { env: { ...process.env, RUNNER_TEMP: root }, encoding: 'utf8' });
  assert.notEqual(result.status,0); assert.match(result.stderr,/link or reparse/);
  assert.equal(fs.readFileSync(path.join(outside,'keep'),'utf8'),'safe');
  assert.equal(fs.readFileSync(path.join(root,'ct2-replay-private','keep'),'utf8'),'untouched');
});
test('Rust lifecycle can follow a failed Python baseline only after retrieval and a revalidated ready receipt', () => {
  const workflow = readWorkflow();
  const replay = requiredMarker(workflow, '        id: replay\n');
  const start = requiredMarker(workflow, '      - name: Exercise the actual Rust manager');
  const upload = requiredMarker(workflow, '      - name: Upload bounded replay metadata');
  const cleanup = requiredMarker(workflow, '      - name: Remove only owned replay private inputs');
  assert(start > replay); assert(upload > start); assert(cleanup > upload);
  const rust = workflow.slice(start, upload);
  assert(rust.includes("if: ${{ always() && !cancelled() && steps.retrieve.outcome == 'success' && (steps.replay.outcome == 'success' || steps.replay.outcome == 'failure') }}"));
  assert(rust.includes('timeout-minutes: 35'));
  assert(rust.includes('Test-Path -LiteralPath "$env:RUNNER_TEMP/ct2-replay-reports/replay-runtime.json" -PathType Leaf'));
  assert(rust.includes('python -B scripts/asr-components/real_worker_fixture.py'));
  assert(rust.includes('--fixture-dir "$env:RUNNER_TEMP/ct2-replay-rust-fixture"'));
  assert(rust.includes('--verifier-source-sha "$env:VERIFIER_SOURCE_SHA"'));
  assert(rust.includes('if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }'));
  assert(!workflow.includes('continue-on-error'));
});
for (const [label, receipt, succeeds] of [
  ['drained tree', { schema: 1, kind: 'ct2-rust-manager-lifecycle', tree_drained: true }, true],
  ['undrained tree', { schema: 1, kind: 'ct2-rust-manager-lifecycle', tree_drained: false }, false],
  ['incomplete receipt', { schema: 1, kind: 'ct2-rust-manager-lifecycle' }, false],
  ['malformed receipt', '{', false],
]) test(`private cleanup checks Rust ${label} before deletion`, t => {
  const { spawnSync } = require('node:child_process');
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'ct2-cleanup-'))); t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  fs.mkdirSync(path.join(root,'ct2-replay-private')); fs.writeFileSync(path.join(root,'ct2-replay-private','keep'),'owned');
  fs.mkdirSync(path.join(root,'ct2-replay-reports'));
  fs.writeFileSync(path.join(root,'ct2-replay-reports','rust-lifecycle.json'), typeof receipt === 'string' ? receipt : JSON.stringify(receipt));
  const result = spawnSync('python', ['-B', '-c', cleanupScript()], { env: { ...process.env, RUNNER_TEMP: root }, encoding: 'utf8' });
  assert.equal(result.status === 0, succeeds, result.stderr);
  assert.equal(fs.existsSync(path.join(root,'ct2-replay-private')), !succeeds);
  assert(fs.existsSync(path.join(root,'ct2-replay-reports','rust-lifecycle.json')));
});
for (const [label, receipt, succeeds] of [
  ['drained direct child', { schema: 1, owner: 'replay-driver', case: 'default-eof', drained: true, pid: 123 }, true],
  ['undrained direct child', { schema: 1, owner: 'replay-driver', case: 'one-unload', drained: false, pid: 123 }, false],
  ['prelaunch checkpoint', { schema: 1, owner: 'replay-driver', case: 'one-switch', drained: false, pid: null }, false],
  ['invalid PID', { schema: 1, owner: 'replay-driver', case: 'default-eof', drained: true, pid: true }, false],
  ['malformed checkpoint', '{', false],
  ['missing checkpoint for ready runtime', null, false],
]) test(`private cleanup checks driver ${label} before deletion`, t => {
  const { spawnSync } = require('node:child_process');
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'ct2-cleanup-'))); t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  fs.mkdirSync(path.join(root,'ct2-replay-private')); fs.writeFileSync(path.join(root,'ct2-replay-private','keep'),'owned');
  fs.mkdirSync(path.join(root,'ct2-replay-reports'));
  fs.writeFileSync(path.join(root,'ct2-replay-reports','replay-runtime.json'), '{}');
  if (receipt !== null) fs.writeFileSync(path.join(root,'ct2-replay-reports','replay-child-drain.json'), typeof receipt === 'string' ? receipt : JSON.stringify(receipt));
  const result = spawnSync('python', ['-B', '-c', cleanupScript()], { env: { ...process.env, RUNNER_TEMP: root }, encoding: 'utf8' });
  assert.equal(result.status === 0, succeeds, result.stderr);
  assert.equal(fs.existsSync(path.join(root,'ct2-replay-private')), !succeeds);
});
