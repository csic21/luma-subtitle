'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { Readable } = require('node:stream');
const p = require('./publish.cjs');
const SOURCE = 'a'.repeat(40), REQUEST = 'b'.repeat(40), OTHER = 'c'.repeat(40);
const clone = x => JSON.parse(JSON.stringify(x));
const error404 = () => Object.assign(new Error('Not found'), { status: 404 });
const pin = (name, bytes) => ({ name, bytes: bytes.length, sha256: p.hash(bytes) });
function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'luma-cpu-publisher-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const sourceBytes = fs.readFileSync(path.join(__dirname, 'sources.lock.json'));
  const noticeBytes = fs.readFileSync(path.join(__dirname, 'notices.lock.json'));
  const locks = { sources: JSON.parse(sourceBytes), notices: JSON.parse(noticeBytes) };
  const request = { schema_version: 1, repository: p.REPOSITORY, branch: p.BRANCH, source_sha: SOURCE, tag: p.TAG,
    proof: { run_id: 123, run_attempt: 1, summary: null }, assets: [],
    locks: { sources: pin('sources.lock.json', sourceBytes), notices: pin('notices.lock.json', noticeBytes) },
    review: { scope: 'cpu-wheel-source-notices-only', approved: true } };
  for (const name of [p.NAMES.wheel, p.NAMES.source, p.NAMES.notices].sort()) {
    const data = Buffer.from(`reviewed candidate ${name}`); fs.writeFileSync(path.join(root, name), data);
    request.assets.push(pin(name, data));
  }
  const proof = { schema: 1, source_sha: SOURCE, variant: p.VARIANT, publication_authorized: false,
    assets: clone(request.assets), locks: clone(request.locks), source_exports: [clone(p.CT2_SOURCE_EXPORT)], checks: Object.fromEntries(p.CHECKS.map(k => [k, true])),
    provenance: { schema: 1, variant: p.VARIANT, luma_source_sha: SOURCE, publication_authorized: false,
      upstream: locks.sources.sources, build_wheels: locks.sources.build_wheels, ct2_cmake: locks.sources.ct2_cmake,
      onednn_cmake: locks.sources.onednn_cmake, notices: locks.notices, source_date_epoch: locks.sources.source_date_epoch,
      python_runtime: { bytes: 123, sha256: 'd'.repeat(64) },
      native_compile_flags: '/Brepro /Z7 /experimental:deterministic /pathmap:<build-root>=C:\\luma-ct2-build',
      native_link_flags: '/Brepro /INCREMENTAL:NO', build_strategy: clone(p.BUILD_STRATEGY), toolchain: { ...locks.sources.toolchain,
        binary_hashes: Object.fromEntries(['cl.exe','link.exe','lib.exe','rc.exe','mt.exe'].map(k => [k, { sha256: 'e'.repeat(64) }])) } } };
  const saveProof = () => { const bytes = Buffer.from(JSON.stringify(proof)); fs.writeFileSync(path.join(root, p.NAMES.proof), bytes); request.proof.summary = pin(p.NAMES.proof, bytes); };
  saveProof();
  const context = { repo: { owner: 'csic21', repo: 'luma-subtitle' }, eventName: 'push', ref: `refs/heads/${p.BRANCH}`,
    sha: REQUEST, payload: { deleted: false, repository: { full_name: p.REPOSITORY } } };
  const run = { id: 123, run_attempt: 1, repository: { full_name: p.REPOSITORY }, head_repository: { full_name: p.REPOSITORY },
    event: 'pull_request', head_branch: p.BRANCH, head_sha: SOURCE, path: '.github/workflows/asr-ct2-cpu.yml', status: 'completed', conclusion: 'success',
    pull_requests: [{ head: { ref: p.BRANCH, sha: OTHER, repo: { url: `https://api.github.com/repos/${p.REPOSITORY}` } },
      base: { ref: 'main', repo: { url: `https://api.github.com/repos/${p.REPOSITORY}` } } }] };
  const jobs = { total_count: 2, jobs: ['windows-cpu-proof', 'windows-direct-crt-signatures'].map(name => ({ name, run_id: 123, head_sha: SOURCE, status: 'completed', conclusion: 'success' })) };
  const updater = Buffer.from('{"version":"1.1.15"}');
  const state = { request, run, jobs, head: REQUEST, tag: null, release: null, assets: [], bytes: new Map([[8, updater]]), mutations: [],
    commit: { parents: [{ sha: SOURCE }] }, extraTree: false, stableTag: OTHER,
    latest: { id: 7, tag_name: 'v1.1.15', target_commitish: OTHER, draft: false, prerelease: false },
    latestAssets: [{ id: 8, name: 'latest.json', size: updater.length, digest: `sha256:${p.hash(updater)}` }] };
  const data = x => ({ data: clone(x) });
  const github = { rest: { repos: {}, git: {} }, paginate: async (method, args) => (await method(args)).data };
  github.rest.git.getRef = async ({ ref }) => {
    if (ref === `heads/${p.BRANCH}`) return data({ object: { sha: state.head, type: 'commit' } });
    if (ref === `tags/${p.TAG}` && state.tag) return data({ object: { sha: state.tag, type: 'commit' } });
    if (ref === 'tags/v1.1.15' && state.stableTag) return data({ object: { sha: state.stableTag, type: 'commit' } });
    throw error404();
  };
  github.rest.git.getTree = async ({ tree_sha }) => {
    if ([SOURCE, REQUEST].includes(tree_sha)) return data({ tree: [
      { path: '.github', mode: '040000', type: 'tree', sha: tree_sha === SOURCE ? 'source-github' : 'request-github' },
      { path: 'package.json', mode: '100644', type: 'blob', sha: state.extraTree && tree_sha === REQUEST ? 'modified' : 'same' },
    ] });
    return data({ tree: [{ path: 'workflows', mode: '040000', type: 'tree', sha: 'same' },
      ...(tree_sha === 'request-github' ? [{ path: path.basename(p.REQUEST_PATH), mode: '100644', type: 'blob', sha: 'new' }] : [])] });
  };
  github.rest.git.createRef = async args => { state.mutations.push(['tag', args]); state.tag = args.sha; return data({ object: { sha: args.sha } }); };
  github.rest.repos.getCommit = async () => data(state.commit);
  github.rest.repos.getContent = async ({ path: name, ref }) => {
    const bytes = name === p.REQUEST_PATH ? Buffer.from(JSON.stringify(state.request)) : name.endsWith('/sources.lock.json') ? sourceBytes : noticeBytes;
    assert.equal(ref, name === p.REQUEST_PATH ? REQUEST : SOURCE);
    return data({ type: 'file', encoding: 'base64', content: bytes.toString('base64') });
  };
  github.rest.repos.listReleases = async () => data(state.release ? [state.release] : []);
  github.rest.repos.getLatestRelease = async () => data(state.latest);
  github.rest.repos.getRelease = async () => data(state.release);
  github.rest.repos.listReleaseAssets = async ({ release_id }) => data(release_id === 7 ? state.latestAssets : state.assets);
  github.rest.repos.createRelease = async args => { state.mutations.push(['create', args]); state.release = { ...args, id: 10 }; return data(state.release); };
  github.rest.repos.uploadReleaseAsset = async args => {
    const chunks = []; for await (const chunk of args.data) chunks.push(chunk); const bytes = Buffer.concat(chunks);
    const id = 100 + state.assets.length;
    const asset = { id, name: args.name, size: bytes.length, digest: `sha256:${p.hash(bytes)}`, state: 'uploaded',
      url: `https://api.github.com/repos/${p.REPOSITORY}/releases/assets/${id}`,
      browser_download_url: `https://github.com/${p.REPOSITORY}/releases/download/untagged-${'e'.repeat(20)}/${args.name}` };
    state.assets.push(asset); state.bytes.set(id, bytes); state.mutations.push(['upload', { name: args.name }]); return data(asset);
  };
  github.rest.repos.getReleaseAsset = async ({ asset_id, request }) => {
    assert.equal(request.parseSuccessResponseBody, false); return { data: Readable.from([state.bytes.get(asset_id)]) };
  };
  github.rest.repos.updateRelease = async args => {
    state.mutations.push(['publish', args]); Object.assign(state.release, args);
    for (const asset of state.assets) asset.browser_download_url = `https://github.com/${p.REPOSITORY}/releases/download/${p.TAG}/${asset.name}`;
    return data(state.release);
  };
  const fetchImpl = async (url, opts) => {
    assert(url.startsWith(`https://api.github.com/repos/${p.REPOSITORY}/actions/runs/123`));
    assert.equal(opts.redirect, 'error'); assert.equal(opts.headers.authorization, undefined);
    return { ok: true, json: async () => clone(url.includes('/jobs?') ? state.jobs : state.run) };
  };
  return { root, request, proof, saveProof, locks, context, state, github, fetchImpl,
    prepare: () => p.prepareWheel({ github, context, fetchImpl }),
    publish: () => p.publishWheel({ github, context, root, fetchImpl }) };
}

test('exact narrow request validates without mutating anything', async t => { const f = fixture(t); await f.prepare(); assert.deepEqual(f.state.mutations, []); });
for (const [label, mutate] of [
  ['v tag', r => r.tag = 'v1.2.0'], ['wrong repo', r => r.repository = 'other/luma-subtitle'],
  ['main branch', r => r.branch = 'main'], ['extra field', r => r.token = 'secret'],
  ['unapproved review', r => r.review.approved = false], ['whole-runtime scope', r => r.review.scope = 'runtime'],
  ['bad source', r => r.source_sha = 'main'], ['wrong attempt', r => r.proof.run_attempt = 0],
  ['unsafe run id', r => r.proof.run_id = Number.MAX_SAFE_INTEGER + 1],
  ['wrong proof name', r => r.proof.summary.name = 'latest.json'], ['bad hash', r => r.assets[0].sha256 = 'A'.repeat(64)],
  ['oversize', r => r.assets[0].bytes = p.MAX_ASSET], ['zero bytes', r => r.assets[0].bytes = 0],
  ['traversal', r => r.assets[0].name = '../file'], ['missing asset', r => r.assets.pop()],
  ['extra asset', r => r.assets.push({ ...r.assets[0], name: 'runtime.zip' })],
  ['duplicate asset', r => r.assets[1] = r.assets[0]], ['missing lock', r => delete r.locks.notices],
]) test(`request rejects ${label}`, t => { const f = fixture(t); mutate(f.request); assert.throws(() => p.parseRequest(JSON.stringify(f.request), f.state.commit)); });
for (const [label, mutate] of [
  ['multiple parents', f => f.state.commit.parents.push({ sha: OTHER })],
  ['wrong source parent', f => f.state.commit.parents[0].sha = OTHER],
  ['source lock drift', f => f.request.locks.sources.sha256 = 'f'.repeat(64)],
  ['request mixed with code changes', f => f.state.extraTree = true],
  ['stale branch', f => f.state.head = OTHER], ['fork', f => f.context.repo.owner = 'other'],
  ['dispatch trigger', f => f.context.eventName = 'workflow_dispatch'], ['deleted push', f => f.context.payload.deleted = true],
  ['wrong payload', f => f.context.payload.repository.full_name = 'other/repo'],
  ['different immutable run SHA', f => f.state.run.head_sha = OTHER],
  ['different immutable job SHA', f => f.state.jobs.jobs[0].head_sha = OTHER],
  ['wrong workflow', f => f.state.run.path = '.github/workflows/release.yml'],
  ['proof failure', f => f.state.run.conclusion = 'failure'], ['new run attempt', f => f.state.run.run_attempt = 2],
  ['incomplete job list', f => f.state.jobs.total_count++], ['skipped native job', f => f.state.jobs.jobs[0].conclusion = 'skipped'],
  ['foreign job run', f => f.state.jobs.jobs[0].run_id++], ['wrong job name', f => f.state.jobs.jobs[0].name = 'fake'],
  ['foreign PR', f => f.state.run.pull_requests[0].head.repo.url = 'https://api.github.com/repos/other/repo'],
  ['moved existing tag', f => f.state.tag = OTHER],
]) test(`live provenance rejects ${label} before mutation`, async t => { const f = fixture(t); mutate(f); await assert.rejects(f.publish()); assert.deepEqual(f.state.mutations, []); });
test('mutable PR head metadata does not rewrite historical run/job provenance', async t => { const f = fixture(t); f.state.run.pull_requests[0].head.sha = REQUEST; await f.prepare(); });
test('manual proof requires its immutable run source, not arbitrary dispatch checkout claims', async t => { const f = fixture(t); f.state.run.event = 'workflow_dispatch'; delete f.state.run.pull_requests; await f.prepare(); f.state.run.head_sha = OTHER; await assert.rejects(f.prepare()); });
for (const check of p.CHECKS) test(`rebuild cannot omit ${check}`, async t => { const f = fixture(t); f.proof.checks[check] = false; f.saveProof(); await assert.rejects(f.publish()); assert.deepEqual(f.state.mutations, []); });
for (const [label, mutate] of [
  ['binary byte drift', f => fs.appendFileSync(path.join(f.root, p.NAMES.wheel), '!')],
  ['runtime leakage', f => fs.writeFileSync(path.join(f.root, 'python.exe'), 'no')],
  ['missing notices', f => fs.unlinkSync(path.join(f.root, p.NAMES.notices))],
  ['source change', f => { f.proof.source_sha = OTHER; f.saveProof(); }],
  ['authorization widening', f => { f.proof.publication_authorized = true; f.saveProof(); }],
  ['foreign build wheels', f => { f.proof.provenance.build_wheels = []; f.saveProof(); }],
  ['toolchain drift', f => { f.proof.provenance.toolchain.vc_tools_version = 'different'; f.saveProof(); }],
  ['missing compiler hash', f => { delete f.proof.provenance.toolchain.binary_hashes['cl.exe']; f.saveProof(); }],
]) test(`candidate validation rejects ${label}`, async t => { const f = fixture(t); mutate(f); await assert.rejects(f.publish()); assert.deepEqual(f.state.mutations, []); });
test('symlink wheel cannot become a release asset', async t => { const f = fixture(t); const file = path.join(f.root, p.NAMES.wheel), outside = `${f.root}-wheel`; fs.renameSync(file, outside); t.after(() => fs.rmSync(outside, { force: true })); fs.symlinkSync(outside, file); await assert.rejects(f.publish()); assert.deepEqual(f.state.mutations, []); });
test('publishes exactly four immutable assets as non-latest prerelease', async t => {
  const f = fixture(t), latest = clone(f.state.latest), stable = clone(f.state.latestAssets); await f.publish();
  assert.equal(f.state.release.draft, false); assert.equal(f.state.release.prerelease, true);
  assert.equal(f.state.tag, SOURCE); assert.equal(f.state.assets.length, 4);
  assert.deepEqual(f.state.latest, latest); assert.deepEqual(f.state.latestAssets, stable);
  for (const [kind, args] of f.state.mutations) {
    if (['create', 'publish'].includes(kind)) assert.equal(args.make_latest, 'false');
    if (kind === 'tag') assert.equal(args.ref, `refs/tags/${p.TAG}`);
    if (kind === 'upload') assert(Object.values(p.NAMES).includes(args.name));
  }
});
test('successful exact retry performs no mutation', async t => { const f = fixture(t); await f.publish(); f.state.mutations = []; await f.publish(); assert.deepEqual(f.state.mutations, []); });
test('interrupted draft resumes missing assets without replacing earlier bytes', async t => { const f = fixture(t); const original = f.github.rest.repos.uploadReleaseAsset; let count = 0; f.github.rest.repos.uploadReleaseAsset = async args => { if (++count === 2) throw new Error('transport'); return original(args); }; await assert.rejects(f.publish(), /transport/); const id = f.state.assets[0].id; f.state.mutations = []; f.github.rest.repos.uploadReleaseAsset = original; await f.publish(); assert.equal(f.state.assets[0].id, id); assert.equal(f.state.mutations.filter(x => x[0] === 'upload').length, 3); });
for (const [label, mutate] of [
  ['source', f => f.state.release.target_commitish = OTHER], ['stable flag', f => f.state.release.prerelease = false],
  ['release body', f => f.state.release.body += 'changed'], ['release title', f => f.state.release.name = 'different'],
  ['extra asset', f => f.state.assets[0].name = 'latest.json'], ['asset size', f => f.state.assets[0].size++],
  ['asset digest', f => f.state.assets[0].digest = `sha256:${'f'.repeat(64)}`],
  ['duplicate asset', f => f.state.assets.push(clone(f.state.assets[0]))],
  ['foreign asset URL', f => f.state.assets[0].url = 'https://attacker.invalid/file'],
  ['foreign browser URL', f => f.state.assets[0].browser_download_url = 'https://attacker.invalid/file'],
  ['provisional published URL', f => f.state.assets[0].browser_download_url = `https://github.com/${p.REPOSITORY}/releases/download/untagged-${'e'.repeat(20)}/${f.state.assets[0].name}`],
  ['remote byte corruption', f => f.state.bytes.set(f.state.assets[0].id, Buffer.from('bad'))],
  ['missing tag', f => f.state.tag = null],
]) test(`exact retry rejects changed ${label} read-only`, async t => { const f = fixture(t); await f.publish(); f.state.mutations = []; mutate(f); await assert.rejects(f.publish()); assert.deepEqual(f.state.mutations, []); });
test('updater downloaded-byte mismatch fails before any mutation', async t => { const f = fixture(t); f.state.bytes.set(8, Buffer.from('corrupt')); await assert.rejects(f.publish()); assert.deepEqual(f.state.mutations, []); });
for (const kind of ['updater', 'stable-tag', 'branch', 'component-tag']) test(`${kind} race stops before draft publication`, async t => { const f = fixture(t), original = f.github.rest.repos.uploadReleaseAsset; f.github.rest.repos.uploadReleaseAsset = async args => { const value = await original(args); if (kind === 'updater') f.state.latestAssets[0].id++; if (kind === 'stable-tag') f.state.stableTag = SOURCE; if (kind === 'branch') f.state.head = OTHER; if (kind === 'component-tag') f.state.tag = null; return value; }; await assert.rejects(f.publish()); assert(!f.state.mutations.some(x => x[0] === 'publish')); });
test('same-byte replacement of an asset still blocks publication', async t => { const f = fixture(t), original = f.github.rest.repos.uploadReleaseAsset; let count = 0; f.github.rest.repos.uploadReleaseAsset = async args => { const value = await original(args); if (++count === 2) { const a = f.state.assets[0]; f.state.bytes.set(900, f.state.bytes.get(a.id)); a.id = 900; a.url = `https://api.github.com/repos/${p.REPOSITORY}/releases/assets/900`; } return value; }; await assert.rejects(f.publish(), /replaced/); assert(!f.state.mutations.some(x => x[0] === 'publish')); });
test('permission denial does not retry, add access or change target', async t => { const f = fixture(t); f.github.rest.repos.createRelease = async () => { throw Object.assign(new Error('Contents permission denied'), { status: 403 }); }; await assert.rejects(f.publish(), /permission denied/); assert.deepEqual(f.state.mutations.map(x => x[0]), ['tag']); });
test('public proof API denial fails closed without adding Actions permission', async t => { const f = fixture(t); await assert.rejects(p.prepareWheel({ ...f, fetchImpl: async () => ({ ok: false, status: 403 }) }), /HTTP 403/); assert.deepEqual(f.state.mutations, []); });
test('stream digest is bounded and byte-exact', async () => { await assert.rejects(p.digestStream(Readable.from([Buffer.alloc(5)]), 4)); assert.deepEqual(await p.digestStream(Readable.from([Buffer.from('ok')]), 2), { bytes: 2, sha256: p.hash('ok') }); });
test('only validated publish job has write permission and no new secrets/Actions grant', () => {
  const workflow = fs.readFileSync(path.join(__dirname, '../../../.github/workflows/asr-cpu-wheel-publish.yml'), 'utf8');
  assert(workflow.includes('pull_request:')); assert(workflow.includes('paths: [\'.github/asr-cpu-wheel-request.json\']'));
  assert.equal((workflow.match(/contents: write/g) || []).length, 1);
  assert(workflow.indexOf('contents: write') > workflow.indexOf('  publish:'));
  assert(workflow.includes('needs: [prepare, build]')); assert(workflow.includes('publication: true'));
  assert(!/^\s+(actions|id-token|packages|workflows):/m.test(workflow));
  assert(!/secrets:|github-token:|run-id:|workflow_dispatch:|pull_request_target:/.test(workflow));
  assert(workflow.includes("'luma-release-pipeline'"));
});

test('verified published workflow retry skips native build and artifact upload read-only', async t => {
  const f = fixture(t); await f.publish(); f.state.mutations = [];
  const outputs = {};
  const result = await p.prepareWheel({ ...f, core: { setOutput: (k, v) => { outputs[k] = v; } }, inspectPublished: true });
  assert.equal(result.alreadyPublished, true); assert.equal(outputs.already_published, 'true');
  assert.deepEqual(f.state.mutations, []);
});
test('published retry cannot skip if remote proof bytes or native evidence differ', async t => {
  const f = fixture(t); await f.publish(); f.state.mutations = [];
  const proof = f.state.assets.find(a => a.name === p.NAMES.proof); f.state.bytes.set(proof.id, Buffer.from('bad'));
  await assert.rejects(p.prepareWheel({ ...f, inspectPublished: true })); assert.deepEqual(f.state.mutations, []);
});
test('new publication and matching interrupted draft require rebuilding', async t => {
  const f = fixture(t); assert.equal((await p.prepareWheel({ ...f, inspectPublished: true })).alreadyPublished, false);
  f.github.rest.repos.uploadReleaseAsset = async () => { throw new Error('interrupted'); };
  await assert.rejects(f.publish(), /interrupted/); f.state.mutations = [];
  assert.equal((await p.prepareWheel({ ...f, inspectPublished: true })).alreadyPublished, false);
  assert.deepEqual(f.state.mutations, []);
});

test('publish-only rerun downloads the successful build artifact ID, not current attempt name', () => {
  const workflow = fs.readFileSync(path.join(__dirname, '../../../.github/workflows/asr-cpu-wheel-publish.yml'), 'utf8');
  assert(workflow.includes('artifact-ids: ${{ needs.build.outputs.candidate_artifact_id }}'));
  assert(workflow.includes('merge-multiple: true'));
  assert(!workflow.includes('github.run_attempt'));
  const build = fs.readFileSync(path.join(__dirname, '../../../.github/workflows/asr-ct2-cpu.yml'), 'utf8');
  assert(build.includes('candidate_artifact_id:'));
  assert(build.includes('artifact-id'));
});

for (const [label, mutate] of [
  ['missing inventory', f => f.proof.source_exports = []],
  ['extra omitted file', f => f.proof.source_exports[0].omitted_members.push({ name: 'LICENSE', bytes: 1, sha256: 'a'.repeat(64) })],
  ['changed model pin', f => f.proof.source_exports[0].omitted_members[0].sha256 = 'a'.repeat(64)],
  ['changed export hash', f => f.proof.source_exports[0].exported.sha256 = 'a'.repeat(64)],
  ['changed retained contents', f => f.proof.source_exports[0].retained_members_sha256 = 'a'.repeat(64)],
  ['changed original hash', f => f.proof.source_exports[0].original.sha256 = 'a'.repeat(64)],
]) test(`publication rejects ${label} in source-only export`, async t => {
  const f = fixture(t); mutate(f); f.saveProof(); await assert.rejects(f.publish()); assert.deepEqual(f.state.mutations, []);
});
for (const method of ['createRef', 'createRelease']) test(`Workflows permission denial from ${method} is surfaced unchanged without retargeting`, async t => {
  const f = fixture(t), denied = Object.assign(new Error('Resource not accessible: Workflows permission is required'), { status: 403 });
  const calls = []; const api = method === 'createRef' ? f.github.rest.git : f.github.rest.repos;
  api[method] = async args => { calls.push(args); throw denied; };
  await assert.rejects(f.publish(), error => error === denied && error.status === 403);
  assert.equal(calls.length, 1);
  if (method === 'createRef') { assert.equal(calls[0].sha, SOURCE); assert.equal(calls[0].ref, `refs/tags/${p.TAG}`); }
  else { assert.equal(calls[0].target_commitish, SOURCE); assert.equal(calls[0].tag_name, p.TAG); }
  assert(!f.state.mutations.some(x => ['upload', 'publish'].includes(x[0])));
});

function enablePushProof(f) {
  const baseGit = '1'.repeat(40), sourceGit = '2'.repeat(40), publishGit = '3'.repeat(40), requests = '4'.repeat(40);
  const blob = '5'.repeat(40), same = '6'.repeat(40);
  const entry = (path, sha, type = 'blob') => ({ path, sha, type, mode: type === 'tree' ? '040000' : '100644' });
  f.state.run.event = 'push'; f.state.run.pull_requests = [];
  for (const name of ['CPU proof source guards', 'Validate request-only CPU proof']) {
    f.state.jobs.jobs.push({ name, run_id: 123, head_sha: SOURCE, status: 'completed', conclusion: 'success' });
  }
  f.state.jobs.total_count = f.state.jobs.jobs.length;
  f.state.proofRequest = { schema_version: 1, source_sha: OTHER, purpose: 'native-cpu-wheel-proof' };
  f.state.proofCommit = { sha: SOURCE, parents: [{ sha: OTHER }] };
  f.state.proofTrees = {
    [OTHER]: [entry('.github', baseGit, 'tree'), entry('package.json', same)],
    [SOURCE]: [entry('.github', sourceGit, 'tree'), entry('package.json', same)],
    [REQUEST]: [entry('.github', publishGit, 'tree'), entry('package.json', same)],
    [baseGit]: [entry('workflows', same, 'tree')],
    [sourceGit]: [entry('workflows', same, 'tree'), entry('requests', requests, 'tree')],
    [publishGit]: [entry('workflows', same, 'tree'), entry('requests', requests, 'tree'), entry('asr-cpu-wheel-request.json', '7'.repeat(40))],
    [requests]: [entry('ct2-cpu-proof.json', blob)],
  };
  const previousCommit = f.github.rest.repos.getCommit, previousContent = f.github.rest.repos.getContent;
  f.github.rest.repos.getCommit = async args => args.ref === SOURCE ? { data: clone(f.state.proofCommit) } : previousCommit(args);
  f.github.rest.repos.getContent = async args => args.path === '.github/requests/ct2-cpu-proof.json'
    ? { data: { type: 'file', encoding: 'base64', sha: blob, content: Buffer.from(JSON.stringify(f.state.proofRequest)).toString('base64') } }
    : previousContent(args);
  f.github.rest.git.getTree = async args => { assert(f.state.proofTrees[args.tree_sha]); return { data: { tree: clone(f.state.proofTrees[args.tree_sha]), truncated: false } }; };
}
test('exact request-only push native proof publishes with equal immutable source/run/job SHAs', async t => {
  const f = fixture(t); enablePushProof(f); await f.publish(); assert.equal(f.state.tag, SOURCE);
  assert.equal(f.state.release.target_commitish, SOURCE);
});
for (const [label, mutate] of [
  ['wrong proof parent', f => f.state.proofCommit.parents[0].sha = REQUEST],
  ['multiple proof parents', f => f.state.proofCommit.parents.push({ sha: REQUEST })],
  ['wrong purpose', f => f.state.proofRequest.purpose = 'publish'],
  ['generic source changes', f => f.state.proofTrees[SOURCE][1].sha = '8'.repeat(40)],
  ['nested extra file', f => f.state.proofTrees['4'.repeat(40)].push({ path: 'another.json', sha: '8'.repeat(40), type: 'blob', mode: '100644' })],
  ['skipped proof request gate', f => f.state.jobs.jobs.find(j => j.name === 'Validate request-only CPU proof').conclusion = 'skipped'],
  ['missing source guards', f => f.state.jobs.jobs.find(j => j.name === 'CPU proof source guards').name = 'other'],
  ['source checked out as parent', f => f.state.jobs.jobs[0].head_sha = OTHER],
]) test(`push-native provenance rejects ${label} without mutation`, async t => {
  const f = fixture(t); enablePushProof(f); mutate(f); await assert.rejects(f.publish()); assert.deepEqual(f.state.mutations, []);
});
test('generic arbitrary push is never sufficient provenance', async t => {
  const f = fixture(t); f.state.run.event = 'push'; await assert.rejects(f.publish()); assert.deepEqual(f.state.mutations, []);
  enablePushProof(f); assert.throws(() => p.validateProvenance(f.state.run, f.state.jobs, f.request), /independently validated/);
});
test('manual proof may skip only the push-specific request gate', async t => {
  const f = fixture(t); f.state.run.event = 'workflow_dispatch';
  f.state.jobs.jobs.push({ name: 'Validate request-only CPU proof', run_id: 123, head_sha: SOURCE, status: 'completed', conclusion: 'skipped' });
  f.state.jobs.total_count++; await f.prepare();
  f.state.jobs.jobs[2].name = 'unreviewed skipped check'; await assert.rejects(f.prepare());
});

for (const [field, value] of [['kind', 'different-directory'], ['native_object_cache_reused', true],
  ['path_independence_claim', true], ['cross_machine_claim', true], ['canonical_native_root', 'C:/host-specific-path']]) {
  test(`fresh-fixed-root strategy rejects changed ${field}`, async t => {
    const f = fixture(t); f.proof.provenance.build_strategy[field] = value; f.saveProof();
    await assert.rejects(f.publish()); assert.deepEqual(f.state.mutations, []);
  });
}
