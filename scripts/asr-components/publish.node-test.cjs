'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { Readable } = require('node:stream');
const p = require('./publish.cjs');
const SOURCE = 'a'.repeat(40), REQUEST = 'b'.repeat(40), OTHER = 'c'.repeat(40);
const hash = (bytes) => createHash('sha256').update(bytes).digest('hex');
const clone = (value) => JSON.parse(JSON.stringify(value));
const error404 = () => Object.assign(new Error('Not found'), { status: 404 });
function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'luma-components-test-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const request = { schema_version: 1, source_sha: SOURCE, tag: p.TAG, pr_run_id: 123, pr_run_attempt: 1, packs: [] };
  for (const pack of p.PACKS) {
    const dir = path.join(root, `asr-component-${pack.id}`); fs.mkdirSync(dir);
    const names = p.filenames(pack.id);
    const archive = Buffer.from(`trusted archive ${pack.id}`);
    const pin = { id: pack.id, archive: { name: names.archive, bytes: archive.length, sha256: hash(archive) } };
    const manifest = Buffer.from(JSON.stringify({ schema: 1, source_sha: SOURCE, release_tag: p.TAG, ...pack,
      version: p.VERSION, installed_bytes: 1000000, max_files: 123,
      archive: { url: `https://github.com/${p.REPOSITORY}/releases/download/${p.TAG}/${names.archive}`, bytes: pin.archive.bytes, sha256: pin.archive.sha256 } }));
    pin.manifest = { name: names.manifest, bytes: manifest.length, sha256: hash(manifest) };
    request.packs.push(pin);
    fs.writeFileSync(path.join(dir, names.archive), archive);
    fs.writeFileSync(path.join(dir, names.manifest), manifest);
    fs.writeFileSync(path.join(dir, names.smoke), JSON.stringify({ schema: 1, pack_id: pack.id, source_sha: SOURCE,
      archive_sha256: pin.archive.sha256, relocated: true, isolated: true, system_python_used: false,
      system_packages_used: false, offline_protocol_tested: true,
      imports: { schema: 1, pack_id: pack.id, source_sha: SOURCE, python: '3.12.15',
        machine: pack.platform === 'windows-x64' ? 'AMD64' : 'arm64',
        isolated: true, user_site: false, relocatable: true, private_native_libraries_checked: 15,
        inference_tested: false, versions: { backend: '1.0' }, tested_device: pack.device,
        metal_tested: pack.device === 'metal', metal_available: pack.device === 'metal' },
      worker_test: { passed: true, checks: ['json-lines', 'offline-path-rejection', 'clean-eof-shutdown', 'idle-termination', 'recovery'] },
      inference: { tested: pack.backend === 'faster-whisper', backend: pack.backend, device: pack.device,
        cold_and_warm: true, segments: [{ start_ms: 0, end_ms: 1000, text: 'country' }] } }));
    fs.writeFileSync(path.join(dir, names.licenses), JSON.stringify([{ name: 'Python', version: '3.12', notice_files: ['LICENSE'] }, { name: 'package', version: '1', notice_files: ['LICENSE'] }]));
  }
  const context = { repo: { owner: 'csic21', repo: 'luma-subtitle' }, eventName: 'push', ref: `refs/heads/${p.BRANCH}`,
    sha: REQUEST, payload: { deleted: false, repository: { full_name: p.REPOSITORY } } };
  const run = { id: 123, run_attempt: 1, repository: { full_name: p.REPOSITORY }, head_repository: { full_name: p.REPOSITORY },
    event: 'pull_request', head_branch: p.BRANCH, head_sha: SOURCE, path: '.github/workflows/asr-components.yml', status: 'completed', conclusion: 'success',
    pull_requests: [{ head: { ref: p.BRANCH, sha: REQUEST, repo: { name: 'luma-subtitle', url: `https://api.github.com/repos/${p.REPOSITORY}` } },
      base: { ref: 'main', repo: { url: `https://api.github.com/repos/${p.REPOSITORY}` } } }] };
  const jobs = { total_count: 4, jobs: p.PACKS.map((pack) => ({ name: `Build component ${pack.id}`, run_id: 123, head_sha: SOURCE, status: 'completed', conclusion: 'success' })) };
  const state = { request, run, jobs, head: REQUEST, tag: null, release: null, assets: [], bytes: new Map(), mutations: [],
    commit: { parents: [{ sha: SOURCE }] }, extraTree: false,
    latest: { id: 7, tag_name: 'v1.1.15', target_commitish: OTHER, draft: false, prerelease: false },
    latestAssets: [{ id: 8, name: 'latest.json', size: 10, digest: `sha256:${'d'.repeat(64)}` }] };
  const data = (d) => ({ data: clone(d) });
  const catalog = { schema: 1, version: p.VERSION, release_tag: p.TAG, repository: p.REPOSITORY, packs: p.PACKS };
  const github = { rest: { repos: {}, git: {} }, paginate: async (method, args) => (await method(args)).data };
  github.rest.git.getRef = async ({ ref }) => {
    if (ref === `heads/${p.BRANCH}`) return data({ object: { sha: state.head, type: 'commit' } });
    if (ref === `tags/${p.TAG}` && state.tag) return data({ object: { sha: state.tag, type: 'commit' } });
    throw error404();
  };
  github.rest.git.getTree = async ({ tree_sha }) => {
    if ([SOURCE, REQUEST].includes(tree_sha)) return data({ tree: [
      { path: '.github', mode: '040000', type: 'tree', sha: tree_sha === SOURCE ? 'source-github' : 'request-github' },
      { path: 'package.json', mode: '100644', type: 'blob', sha: state.extraTree && tree_sha === REQUEST ? 'modified' : 'same' },
    ] });
    return data({ tree: [{ path: 'workflows', mode: '040000', type: 'tree', sha: 'unchanged' },
      { path: 'release-request.json', mode: '100644', type: 'blob', sha: 'unchanged-app-request' },
      ...(tree_sha === 'request-github' ? [{ path: 'asr-components-request.json', mode: '100644', type: 'blob', sha: 'new' }] : [])] });
  };
  github.rest.git.createRef = async (args) => { state.mutations.push(['tag', args]); state.tag = args.sha; return data({ object: { sha: args.sha } }); };
  github.rest.repos.getCommit = async () => data(state.commit);
  github.rest.repos.getContent = async ({ path: file }) => data({ type: 'file', encoding: 'base64', content: Buffer.from(JSON.stringify(file === p.REQUEST_PATH ? state.request : catalog)).toString('base64') });
  github.rest.repos.listReleases = async () => data(state.release ? [state.release] : []);
  github.rest.repos.getLatestRelease = async () => data(state.latest);
  github.rest.repos.getRelease = async () => data(state.release);
  github.rest.repos.listReleaseAssets = async ({ release_id }) => data(release_id === 7 ? state.latestAssets : state.assets);
  github.rest.repos.createRelease = async (args) => { state.mutations.push(['create', args]); state.release = { ...args, id: 10 }; return data(state.release); };
  github.rest.repos.uploadReleaseAsset = async (args) => {
    const chunks = []; for await (const chunk of args.data) chunks.push(chunk); const bytes = Buffer.concat(chunks);
    const id = 100 + state.assets.length;
    const asset = { id, name: args.name, size: bytes.length, digest: `sha256:${hash(bytes)}`, state: 'uploaded',
      url: `https://api.github.com/repos/${p.REPOSITORY}/releases/assets/${id}`,
      browser_download_url: `https://github.com/${p.REPOSITORY}/releases/download/untagged-${'e'.repeat(20)}/${args.name}` };
    state.assets.push(asset); state.bytes.set(id, bytes); state.mutations.push(['upload', { name: args.name }]); return data(asset);
  };
  github.rest.repos.getReleaseAsset = async ({ asset_id, request }) => {
    assert.equal(request.parseSuccessResponseBody, false); return { data: Readable.from([state.bytes.get(asset_id)]) };
  };
  github.rest.repos.updateRelease = async (args) => {
    state.mutations.push(['publish', args]); Object.assign(state.release, args);
    for (const asset of state.assets) asset.browser_download_url = `https://github.com/${p.REPOSITORY}/releases/download/${p.TAG}/${asset.name}`;
    return data(state.release);
  };
  const fetchImpl = async (url, opts) => {
    assert(url.startsWith(`https://api.github.com/repos/${p.REPOSITORY}/actions/runs/123`));
    assert.equal(opts.redirect, 'error'); assert.equal(opts.headers.authorization, undefined);
    return { ok: true, json: async () => clone(url.includes('/jobs?') ? state.jobs : state.run) };
  };
  return { root, request, context, run, jobs, state, github, fetchImpl,
    prepare: () => p.prepareComponents({ github, context, fetchImpl }),
    publish: () => p.publishComponents({ github, context, root, fetchImpl }) };
}

test('request accepts exactly the reviewed source and four pinned packs', (t) => {
  const f = fixture(t); assert.deepEqual(p.parseRequest(JSON.stringify(f.request), f.state.commit), f.request);
});
for (const [label, mutate] of [
  ['wrong tag', r => r.tag = 'v1.2.0'], ['extra key', r => r.token = 'no'],
  ['traversal filename', r => r.packs[0].archive.name = '../latest.json'],
  ['duplicate pack', r => r.packs[1] = r.packs[0]], ['missing pack', r => r.packs.pop()],
  ['large archive', r => r.packs[0].archive.bytes = p.MAX_ARCHIVE], ['negative size', r => r.packs[0].manifest.bytes = -1],
  ['invalid hash', r => r.packs[0].archive.sha256 = 'A'.repeat(64)], ['unknown pack', r => r.packs[0].id = 'other'],
  ['unsafe run id', r => r.pr_run_id = Number.MAX_SAFE_INTEGER + 1],
]) test(`request rejects ${label}`, (t) => { const f = fixture(t); mutate(f.request); assert.throws(() => p.parseRequest(JSON.stringify(f.request), f.state.commit)); });
test('source must be only parent', (t) => { const f = fixture(t); f.state.commit.parents.push({ sha: OTHER }); assert.throws(() => p.parseRequest(JSON.stringify(f.request), f.state.commit)); });
for (const [label, mutate] of [
  ['fork', f => f.context.repo.owner = 'other'], ['main branch', f => f.context.ref = 'refs/heads/main'],
  ['manual dispatch', f => f.context.eventName = 'workflow_dispatch'], ['deleted ref', f => f.context.payload.deleted = true],
  ['payload repository', f => f.context.payload.repository.full_name = 'other/luma-subtitle'],
  ['advanced branch', f => f.state.head = OTHER], ['non-request tree change', f => f.state.extraTree = true],
  ['different immutable tag', f => f.state.tag = OTHER],
]) test(`preparation rejects ${label} without writes`, async (t) => { const f = fixture(t); mutate(f); await assert.rejects(f.prepare()); assert.deepEqual(f.state.mutations, []); });
test('preparation is read-only and accepts mutable PR head metadata while pinning run SHA', async (t) => { const f = fixture(t); await f.prepare(); assert.deepEqual(f.state.mutations, []); });
for (const [label, mutate] of [
  ['wrong run source', f => f.run.head_sha = OTHER], ['wrong job source', f => f.jobs.jobs[0].head_sha = OTHER],
  ['fork head', f => f.run.head_repository.full_name = 'other/luma-subtitle'],
  ['another workflow', f => f.run.path = '.github/workflows/ci.yml'], ['another event', f => f.run.event = 'push'],
  ['failed run', f => f.run.conclusion = 'failure'], ['wrong attempt', f => f.run.run_attempt = 2],
  ['missing jobs', f => f.jobs.jobs.pop()], ['failed pack', f => f.jobs.jobs[0].conclusion = 'failure'],
  ['duplicate job', f => { f.jobs.jobs[1] = clone(f.jobs.jobs[0]); }],
  ['untrusted PR repo', f => f.run.pull_requests[0].head.repo.url = 'https://api.github.com/repos/other/luma-subtitle'],
]) test(`provenance rejects ${label}`, (t) => { const f = fixture(t); mutate(f); assert.throws(() => p.validateProvenance(f.run, f.jobs, f.request)); });
test('complete rebuilt artifacts match pins and validate reports', async (t) => { const f = fixture(t); assert.equal((await p.validateArtifacts(f.root, f.request)).length, 16); });
for (const [label, mutate] of [
  ['missing directory', (f, dir) => fs.rmSync(dir, { recursive: true })],
  ['unexpected file', (f, dir) => fs.writeFileSync(path.join(dir, 'latest.json'), '{}')],
  ['archive byte change', (f, dir, names) => fs.appendFileSync(path.join(dir, names.archive), 'x')],
  ['missing lifecycle evidence', (f, dir, names) => { const file = path.join(dir, names.smoke); const s=JSON.parse(fs.readFileSync(file)); s.worker_test.checks=[]; fs.writeFileSync(file,JSON.stringify(s)); }],
  ['non-isolated runtime', (f, dir, names) => { const file = path.join(dir, names.smoke); const s=JSON.parse(fs.readFileSync(file)); s.imports.isolated=false; fs.writeFileSync(file,JSON.stringify(s)); }],
  ['incomplete license inventory', (f, dir, names) => fs.writeFileSync(path.join(dir,names.licenses),'[]')],
]) test(`artifact verification rejects ${label} before any write`, async (t) => { const f=fixture(t); const pack=p.PACKS[0]; mutate(f,path.join(f.root,`asr-component-${pack.id}`),p.filenames(pack.id)); await assert.rejects(f.publish()); assert.deepEqual(f.state.mutations,[]); });
test('symlink artifact paths are rejected', async (t) => { const f=fixture(t); const id=p.PACKS[0].id; const file=path.join(f.root,`asr-component-${id}`,p.filenames(id).archive); const target=file+'.target'; fs.renameSync(file,target); fs.symlinkSync(target,file); await assert.rejects(p.validateArtifacts(f.root,f.request)); });
test('Qwen report cannot silently claim tested inference', (t) => { const f=fixture(t); const pack=p.PACKS[2]; const s=JSON.parse(fs.readFileSync(path.join(f.root,`asr-component-${pack.id}`,p.filenames(pack.id).smoke))); s.inference.tested=true; assert.throws(()=>p.validateSmoke(s,f.request,pack,f.request.packs[2].archive.sha256)); });
test('stream hashing is bounded and byte-exact', async () => { await assert.rejects(p.digestStream(Readable.from([Buffer.alloc(5)]),4)); assert.deepEqual(await p.digestStream(Readable.from([Buffer.from('ok')]),2),{bytes:2,sha256:hash('ok')}); });
test('publishes only immutable component prerelease; latest app and updater remain unchanged', async (t) => {
  const f=fixture(t); const latest=clone(f.state.latest); const assets=clone(f.state.latestAssets); await f.publish();
  assert.equal(f.state.assets.length,16); assert.equal(f.state.release.draft,false); assert.equal(f.state.release.prerelease,true);
  assert.deepEqual(f.state.latest,latest); assert.deepEqual(f.state.latestAssets,assets); assert.equal(f.state.tag,SOURCE);
  for (const [kind,args] of f.state.mutations) {
    if (kind==='create'||kind==='publish') assert.equal(args.make_latest,'false');
    if (kind==='tag') assert.equal(args.ref,`refs/tags/${p.TAG}`);
    if (kind==='upload') assert.notEqual(args.name,'latest.json');
  }
});
test('published exact retry is read-only', async (t) => { const f=fixture(t); await f.publish(); f.state.mutations.length=0; await f.publish(); assert.deepEqual(f.state.mutations,[]); });
test('matching interrupted draft resumes missing uploads without replacing any assets', async (t) => {
  const f=fixture(t); const original=f.github.rest.repos.uploadReleaseAsset; let count=0;
  f.github.rest.repos.uploadReleaseAsset=async args=>{ if(++count===3)throw new Error('network'); return original(args); };
  await assert.rejects(f.publish(),/network/); const ids=f.state.assets.map(a=>a.id); f.state.mutations.length=0; f.github.rest.repos.uploadReleaseAsset=original;
  await f.publish(); assert.deepEqual(f.state.assets.slice(0,2).map(a=>a.id),ids); assert.equal(f.state.mutations.filter(x=>x[0]==='upload').length,14);
});
for (const [label, mutate] of [
  ['changed source', f=>f.state.release.target_commitish=OTHER], ['published as stable', f=>f.state.release.prerelease=false],
  ['unexpected asset', f=>f.state.assets[0].name='latest.json'], ['changed asset digest', f=>f.state.assets[0].digest=`sha256:${'f'.repeat(64)}`],
  ['duplicate asset', f=>f.state.assets.push(clone(f.state.assets[0]))], ['foreign asset API', f=>f.state.assets[0].url='https://attacker.invalid/file'],
  ['foreign browser URL', f=>f.state.assets[0].browser_download_url='https://attacker.invalid/file'],
  ['published provisional URL', f=>f.state.assets[0].browser_download_url=`https://github.com/${p.REPOSITORY}/releases/download/untagged-${'f'.repeat(20)}/${f.state.assets[0].name}`],
  ['corrupt downloaded bytes', f=>f.state.bytes.set(f.state.assets[0].id,Buffer.from('corrupt'))],
]) test(`immutable retry rejects ${label} without writes`,async t=>{const f=fixture(t);await f.publish();f.state.mutations.length=0;mutate(f);await assert.rejects(f.publish());assert.deepEqual(f.state.mutations,[]);});
test('permission failure is surfaced without retargeting or alternate mutation',async t=>{const f=fixture(t);f.github.rest.repos.createRelease=async()=>{throw Object.assign(new Error('Workflows permission required'),{status:403});};await assert.rejects(f.publish(),/Workflows permission/);assert.deepEqual(f.state.mutations.map(x=>x[0]),['tag']);});
test('latest-app race stops before publication',async t=>{const f=fixture(t);const upload=f.github.rest.repos.uploadReleaseAsset;f.github.rest.repos.uploadReleaseAsset=async args=>{const result=await upload(args);f.state.latest.tag_name='v1.2.0';return result;};await assert.rejects(f.publish(),/Latest application release changed/);assert(!f.state.mutations.some(x=>x[0]==='publish'));});
test('workflow grants only ephemeral contents write in gated publication and keeps reusable builds read-only',()=>{
  const workflow=fs.readFileSync(path.join(__dirname,'../../.github/workflows/asr-components-publish.yml'),'utf8');
  assert(workflow.includes("branches: ['feat/optional-asr-engines']"));assert(workflow.includes("paths: ['.github/asr-components-request.json']"));
  assert(workflow.includes('contents: read'));assert.equal((workflow.match(/contents: write/g)||[]).length,1);
  assert(!/^\s+(actions|id-token|packages|workflows):/m.test(workflow));assert(!/secrets: inherit|github-token:|run-id:|workflow_dispatch:/m.test(workflow));
  assert(workflow.includes('group: luma-release-pipeline'));assert(workflow.includes('needs: [prepare, rebuild]'));
});

test('manifest rejects foreign URLs, traversal entrypoints and installer-exceeding bounds', t=>{
  const f=fixture(t),pack=p.PACKS[0],pin=f.request.packs[0];
  const good=JSON.parse(fs.readFileSync(path.join(f.root,`asr-component-${pack.id}`,p.filenames(pack.id).manifest)));
  for(const mutate of [m=>m.archive.url='https://attacker.invalid/runtime.zip',m=>m.entrypoint='../python.exe',m=>m.max_files=100001,m=>m.installed_bytes=0,m=>m.source_sha=OTHER,m=>m.archive.sha256='f'.repeat(64)]) {
    const m=clone(good);mutate(m);assert.throws(()=>p.validateManifest(m,f.request,pin,pack));
  }
});
test('smoke requires archive binding, target architecture, native library isolation and complete worker checks',t=>{
  const f=fixture(t),pack=p.PACKS[0],pin=f.request.packs[0];
  const good=JSON.parse(fs.readFileSync(path.join(f.root,`asr-component-${pack.id}`,p.filenames(pack.id).smoke)));
  for(const mutate of [s=>s.archive_sha256='f'.repeat(64),s=>s.system_python_used=true,s=>s.imports.machine='arm64',s=>s.imports.private_native_libraries_checked=0,s=>s.worker_test.checks.pop(),s=>s.inference.tested=false]) {
    const s=clone(good);mutate(s);assert.throws(()=>p.validateSmoke(s,f.request,pack,pin.archive.sha256));
  }
});
test('MLX report cannot call a CPU tensor test Metal validation',t=>{
  const f=fixture(t),pack=p.PACKS[1],pin=f.request.packs[1];
  const s=JSON.parse(fs.readFileSync(path.join(f.root,`asr-component-${pack.id}`,p.filenames(pack.id).smoke)));
  s.imports.metal_available=false;s.imports.tested_device='cpu';
  assert.throws(()=>p.validateSmoke(s,f.request,pack,pin.archive.sha256));
});
test('replacement of an earlier asset stops publication even when replacement bytes match',async t=>{
  const f=fixture(t);const upload=f.github.rest.repos.uploadReleaseAsset;let count=0;
  f.github.rest.repos.uploadReleaseAsset=async args=>{const result=await upload(args);if(++count===2){const old=f.state.assets[0];const bytes=f.state.bytes.get(old.id);old.id=999;old.url=`https://api.github.com/repos/${p.REPOSITORY}/releases/assets/999`;f.state.bytes.set(999,bytes);}return result;};
  await assert.rejects(f.publish(),/asset was replaced/);assert(!f.state.mutations.some(x=>x[0]==='publish'));
});
test('PR provenance network denial never falls back to trusting local claims',async t=>{
  const f=fixture(t);await assert.rejects(p.prepareComponents({...f,fetchImpl:async()=>({ok:false,status:403})}),/HTTP 403/);assert.deepEqual(f.state.mutations,[]);
});
test('hosted Mac without Metal may report honest experimental import-only coverage',t=>{
  const f=fixture(t),pack=p.PACKS[1],pin=f.request.packs[1];
  const s=JSON.parse(fs.readFileSync(path.join(f.root,`asr-component-${pack.id}`,p.filenames(pack.id).smoke)));
  s.imports.metal_available=false;s.imports.metal_tested=false;s.imports.tested_device='cpu (runner has no Metal device)';
  assert.doesNotThrow(()=>p.validateSmoke(s,f.request,pack,pin.archive.sha256));
});
