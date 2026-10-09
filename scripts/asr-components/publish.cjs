'use strict';

// This controller publishes only reviewed component bytes. It never changes the
// application release, updater manifest, credentials, or an existing asset.
const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { Readable } = require('node:stream');
const { SHA, assertRequestOnlyTree, tagCommit } = require('../prepare-release.cjs');
const { publicApi } = require('../recover-release.cjs');
const REPOSITORY = 'csic21/luma-subtitle';
const BRANCH = 'feat/optional-asr-engines';
const REQUEST_PATH = '.github/asr-components-request.json';
const VERSION = '1.2.0-r1';
const TAG = `asr-components-${VERSION}`;
const MAX_ARCHIVE = 2_000_000_000; // Strictly below GitHub's 2 GiB asset ceiling.
const MAX_JSON = 16_000_000;
const PACKS = [
  { id: 'faster-whisper-cpu-windows-x64', platform: 'windows-x64', engine: 'whisper-accelerated', backend: 'faster-whisper', device: 'cpu', entrypoint: 'python.exe' },
  { id: 'mlx-whisper-metal-macos-arm64', platform: 'macos-arm64', engine: 'whisper-accelerated', backend: 'mlx-whisper', device: 'metal', entrypoint: 'bin/python3' },
  { id: 'qwen3-asr-cpu-windows-x64', platform: 'windows-x64', engine: 'qwen3-asr', backend: 'qwen3-asr', device: 'cpu', entrypoint: 'python.exe' },
  { id: 'qwen3-asr-cpu-macos-arm64', platform: 'macos-arm64', engine: 'qwen3-asr', backend: 'qwen3-asr', device: 'cpu', entrypoint: 'bin/python3' },
];
const positive = (n) => Number.isSafeInteger(n) && n > 0;
const hash = (bytes) => createHash('sha256').update(bytes).digest('hex');
const equal = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const keys = (o, expected) => o && !Array.isArray(o) && typeof o === 'object'
  && Object.keys(o).sort().join(',') === expected.split(',').sort().join(',');
function filenames(id) {
  return { archive: `luma-asr-${id}-${VERSION}.zip`, manifest: `${id}.manifest.json`,
    smoke: `${id}.smoke.json`, licenses: `LICENSES-${id}.json` };
}
function assetUrl(name) {
  return `https://github.com/${REPOSITORY}/releases/download/${TAG}/${name}`;
}
function parseRequest(text, commit) {
  const r = JSON.parse(text);
  if (!keys(r, 'schema_version,source_sha,tag,pr_run_id,pr_run_attempt,packs')
      || r.schema_version !== 1 || r.tag !== TAG || !SHA.test(r.source_sha || '')
      || !positive(r.pr_run_id) || !positive(r.pr_run_attempt)
      || commit.parents?.length !== 1 || commit.parents[0].sha !== r.source_sha) {
    throw new Error('Invalid component request or sole source parent');
  }
  if (!Array.isArray(r.packs) || r.packs.length !== PACKS.length
      || new Set(r.packs.map((p) => p?.id)).size !== PACKS.length) throw new Error('Expected exactly four unique packs');
  for (const pack of r.packs) {
    if (!keys(pack, 'id,archive,manifest') || !PACKS.some((p) => p.id === pack.id)) throw new Error('Unexpected component pack');
    for (const kind of ['archive', 'manifest']) {
      const item = pack[kind];
      if (!keys(item, 'name,bytes,sha256') || item.name !== filenames(pack.id)[kind]
          || !positive(item.bytes) || item.bytes >= (kind === 'archive' ? MAX_ARCHIVE : MAX_JSON)
          || !/^[a-f0-9]{64}$/.test(item.sha256 || '')) throw new Error('Invalid pinned filename, size or hash');
    }
  }
  return r;
}
function assertContext(context) {
  if (`${context.repo.owner}/${context.repo.repo}` !== REPOSITORY || context.eventName !== 'push'
      || context.ref !== `refs/heads/${BRANCH}` || context.payload.deleted
      || context.payload.repository?.full_name !== REPOSITORY || !SHA.test(context.sha || '')) {
    throw new Error('Components require a request-only push in the intended repository and feature branch');
  }
}
async function readSource(github, repo, file, ref) {
  const { data } = await github.rest.repos.getContent({ ...repo, path: file, ref });
  if (data.type !== 'file' || data.encoding !== 'base64') throw new Error(`Invalid source file ${file}`);
  return Buffer.from(data.content, 'base64').toString('utf8');
}
function validateProvenance(run, jobs, r) {
  if (run.id !== r.pr_run_id || run.run_attempt !== r.pr_run_attempt
      || run.repository?.full_name !== REPOSITORY || run.head_repository?.full_name !== REPOSITORY
      || run.event !== 'pull_request' || run.head_branch !== BRANCH || run.head_sha !== r.source_sha
      || run.path !== '.github/workflows/asr-components.yml' || run.status !== 'completed'
      || run.conclusion !== 'success' || !Array.isArray(run.pull_requests) || run.pull_requests.length !== 1
      // PR head.sha is mutable API metadata; immutable provenance is run/job head_sha.
      || run.pull_requests[0].head?.ref !== BRANCH
      || run.pull_requests[0].head?.repo?.url !== `https://api.github.com/repos/${REPOSITORY}`
      || run.pull_requests[0].base?.repo?.url !== `https://api.github.com/repos/${REPOSITORY}`
      || run.pull_requests[0].head?.repo?.name !== 'luma-subtitle'
      || run.pull_requests[0].base?.ref !== 'main') throw new Error('Reviewed PR build provenance mismatch');
  if (!Array.isArray(jobs.jobs) || jobs.total_count !== jobs.jobs.length || jobs.total_count > 100) throw new Error('Incomplete PR job list');
  for (const p of PACKS) {
    const matches = jobs.jobs.filter((j) => j.name === `Build component ${p.id}`);
    if (matches.length !== 1 || matches[0].run_id !== run.id || matches[0].head_sha !== run.head_sha
        || matches[0].status !== 'completed' || matches[0].conclusion !== 'success') throw new Error(`Missing successful PR build: ${p.id}`);
  }
}
async function prepareComponents({ github, context, core, fetchImpl = fetch }) {
  assertContext(context);
  const repo = context.repo;
  const { data: commit } = await github.rest.repos.getCommit({ ...repo, ref: context.sha });
  const request = parseRequest(await readSource(github, repo, REQUEST_PATH, context.sha), commit);
  await assertRequestOnlyTree(github, repo, request.source_sha, context.sha, 'asr-components-request.json');
  const head = await github.rest.git.getRef({ ...repo, ref: `heads/${BRANCH}` });
  if (head.data.object.sha !== context.sha) throw new Error('Feature branch advanced; stale component request');
  const config = JSON.parse(await readSource(github, repo, 'scripts/asr-components/packs.json', request.source_sha));
  if (config.schema !== 1 || config.version !== VERSION || config.release_tag !== TAG || config.repository !== REPOSITORY
      || !Array.isArray(config.packs) || config.packs.length !== PACKS.length
      || !equal(config.packs.map((p) => p.id).sort(), PACKS.map((p) => p.id).sort())) throw new Error('Source component catalog mismatch');
  const run = await publicApi(repo, `actions/runs/${request.pr_run_id}`, fetchImpl);
  const jobs = await publicApi(repo, `actions/runs/${request.pr_run_id}/attempts/${request.pr_run_attempt}/jobs?per_page=100`, fetchImpl);
  validateProvenance(run, jobs, request);
  const existingTag = await tagCommit(github, repo, TAG);
  if (existingTag && existingTag !== request.source_sha) throw new Error('Immutable component tag points to another source');
  core?.setOutput('source_sha', request.source_sha);
  core?.setOutput('tag', TAG);
  return request;
}
async function digestStream(stream, maxBytes) {
  const digest = createHash('sha256'); let bytes = 0;
  for await (const chunk of stream) {
    bytes += chunk.length;
    if (bytes > maxBytes) throw new Error('Asset exceeds pinned size');
    digest.update(chunk);
  }
  return { bytes, sha256: digest.digest('hex') };
}
function plainFile(file, max) {
  const stat = fs.lstatSync(file);
  if (!stat.isFile() || stat.isSymbolicLink() || !positive(stat.size) || stat.size >= max) throw new Error('Invalid local artifact file');
  return stat;
}
function validateManifest(m, r, pin, pack) {
  if (m.schema !== 1 || m.source_sha !== r.source_sha || m.release_tag !== TAG || m.version !== VERSION
      || Object.entries(pack).some(([key, value]) => m[key] !== value)
      || !positive(m.installed_bytes) || m.installed_bytes > 20_000_000_000
      || !positive(m.max_files) || m.max_files > 100_000
      || !keys(m.archive, 'url,bytes,sha256') || m.archive.url !== assetUrl(pin.archive.name)
      || m.archive.bytes !== pin.archive.bytes || m.archive.sha256 !== pin.archive.sha256) throw new Error('Rebuilt manifest identity or bounds mismatch');
}
function validateSmoke(s, r, pack, archiveHash) {
  const imports = s.imports;
  const checks = ['json-lines', 'offline-path-rejection', 'clean-eof-shutdown', 'idle-termination', 'recovery'];
  if (s.schema !== 1 || s.pack_id !== pack.id || s.source_sha !== r.source_sha
      || s.archive_sha256 !== archiveHash || s.relocated !== true || s.isolated !== true
      || s.system_python_used !== false || s.system_packages_used !== false || s.offline_protocol_tested !== true
      || imports?.schema !== 1 || imports.pack_id !== pack.id || imports.source_sha !== r.source_sha
      || imports.python !== '3.12.15' || imports.isolated !== true || imports.user_site !== false
      || imports.relocatable !== true || !positive(imports.private_native_libraries_checked)
      || imports.inference_tested !== false || typeof imports.metal_tested !== 'boolean'
      || !imports.versions || Object.keys(imports.versions).length === 0
      || s.worker_test?.passed !== true || !Array.isArray(s.worker_test.checks)
      || !checks.every((c) => s.worker_test.checks.includes(c))
      || typeof s.inference?.tested !== 'boolean') throw new Error('Incomplete native component smoke');
  const machine = String(imports.machine).toLowerCase();
  if ((pack.platform === 'windows-x64' && !['amd64', 'x86_64'].includes(machine))
      || (pack.platform === 'macos-arm64' && machine !== 'arm64')) throw new Error('Smoke was not run on the target architecture');
  if (pack.backend === 'qwen3-asr' && (s.inference.tested !== false || imports.tested_device !== 'cpu')) {
    throw new Error('Qwen pack requires honest CPU import/API-only evidence');
  }
  if (pack.backend === 'faster-whisper' && (imports.tested_device !== 'cpu' || s.inference.tested !== true || s.inference.backend !== 'faster-whisper'
      || s.inference.device !== 'cpu' || s.inference.cold_and_warm !== true
      || !Array.isArray(s.inference.segments) || s.inference.segments.length === 0)) {
    throw new Error('Faster-whisper requires actual tiny CPU cold/warm inference evidence');
  }
  if (pack.backend === 'mlx-whisper' && (s.inference.tested !== false
      || typeof imports.metal_available !== 'boolean'
      || imports.metal_tested !== imports.metal_available
      || (imports.metal_tested && imports.tested_device !== 'metal'))) {
    throw new Error('MLX must report Metal tensor and untested model inference truthfully');
  }
}
async function validateArtifacts(root, request) {
  const directory = fs.lstatSync(root);
  if (!directory.isDirectory() || directory.isSymbolicLink()) throw new Error('Invalid artifact root');
  const dirs = fs.readdirSync(root).sort();
  if (!equal(dirs, PACKS.map((p) => `asr-component-${p.id}`).sort())) throw new Error('Missing or unexpected artifact directories');
  const files = [];
  for (const pack of PACKS) {
    const dir = path.join(root, `asr-component-${pack.id}`); const stat = fs.lstatSync(dir);
    if (!stat.isDirectory() || stat.isSymbolicLink()) throw new Error('Invalid pack artifact directory');
    const names = filenames(pack.id);
    if (!equal(fs.readdirSync(dir).sort(), Object.values(names).sort())) throw new Error('Missing, duplicate or unexpected artifact files');
    const pin = request.packs.find((p) => p.id === pack.id);
    for (const [kind, name] of Object.entries(names)) {
      const file = path.join(dir, name); const stat = plainFile(file, kind === 'archive' ? MAX_ARCHIVE : MAX_JSON);
      const digest = await digestStream(fs.createReadStream(file), stat.size);
      if (kind === 'archive' || kind === 'manifest') {
        if (digest.bytes !== pin[kind].bytes || digest.sha256 !== pin[kind].sha256) throw new Error('Rebuild differs from reviewed pinned bytes');
      }
      files.push({ name, file, ...digest });
    }
    validateManifest(JSON.parse(fs.readFileSync(path.join(dir, names.manifest), 'utf8')), request, pin, pack);
    validateSmoke(JSON.parse(fs.readFileSync(path.join(dir, names.smoke), 'utf8')), request, pack, pin.archive.sha256);
    const licenses = JSON.parse(fs.readFileSync(path.join(dir, names.licenses), 'utf8'));
    if (!Array.isArray(licenses) || licenses.length < 2 || licenses.some((l) => !l?.name || !l.version || !Array.isArray(l.notice_files))) throw new Error('Incomplete component license inventory');
  }
  return files.sort((a, b) => a.name.localeCompare(b.name));
}
function bodyFor(request, files) {
  const pins = request.packs.slice().sort((a, b) => a.id.localeCompare(b.id));
  const requestHash = hash(JSON.stringify({ ...request, packs: pins }));
  const artifactHash = hash(JSON.stringify(files.map(({ name, bytes, sha256 }) => ({ name, bytes, sha256 }))));
  return `Optional local ASR runtime components ${VERSION}. No model weights are included.\n\nNative import/API, relocation, offline worker protocol and idle-process lifecycle checks passed. Inference coverage is reported per pack. MLX/Qwen model inference is untested; any Metal tensor evidence is reported separately. These components do not change the latest application release.\n\n<!-- luma-asr-components:${request.source_sha}:${requestHash}:${artifactHash} -->`;
}
function validateRelease(release, request, body, id = release?.id) {
  if (!positive(id) || release.id !== id || release.tag_name !== TAG || release.prerelease !== true
      || typeof release.draft !== 'boolean' || release.target_commitish !== request.source_sha
      || release.body !== body) throw new Error('Existing component release identity differs; refusing overwrite');
}
function validateRemoteAssets(assets, files, draft, complete = false) {
  if (!Array.isArray(assets) || new Set(assets.map((a) => a.id)).size !== assets.length
      || new Set(assets.map((a) => a.name)).size !== assets.length || (complete && assets.length !== files.length)) throw new Error('Incomplete or duplicate remote assets');
  for (const asset of assets) {
    const expected = files.find((f) => f.name === asset.name);
    const provisional = `https://github.com/${REPOSITORY}/releases/download/`;
    const tail = typeof asset.browser_download_url === 'string' && asset.browser_download_url.startsWith(provisional)
      ? asset.browser_download_url.slice(provisional.length).split('/') : [];
    const urlOkay = asset.browser_download_url === assetUrl(asset.name)
      || (draft && tail.length === 2 && /^untagged-[a-f0-9]{20}$/.test(tail[0]) && tail[1] === asset.name);
    if (!expected || !positive(asset.id) || asset.state !== 'uploaded' || asset.size !== expected.bytes
        || asset.digest !== `sha256:${expected.sha256}` || !urlOkay
        || asset.url !== `https://api.github.com/repos/${REPOSITORY}/releases/assets/${asset.id}`) throw new Error('Existing asset differs; refusing replacement');
  }
}
async function verifyRemoteAsset(github, repo, asset) {
  const { data } = await github.rest.repos.getReleaseAsset({ ...repo, asset_id: asset.id,
    headers: { accept: 'application/octet-stream' }, request: { parseSuccessResponseBody: false } });
  let stream;
  if (data?.getReader) stream = Readable.fromWeb(data);
  else if (data?.[Symbol.asyncIterator]) stream = data;
  else if (data instanceof ArrayBuffer || ArrayBuffer.isView(data)) stream = Readable.from([Buffer.from(data)]);
  else throw new Error('Asset API did not return raw bytes');
  const result = await digestStream(stream, asset.size);
  if (result.bytes !== asset.size || `sha256:${result.sha256}` !== asset.digest) throw new Error('Remote downloaded asset hash/size mismatch');
}
async function latestIdentity(github, repo) {
  const { data: latest } = await github.rest.repos.getLatestRelease(repo);
  if (!positive(latest.id) || latest.draft || latest.prerelease || !/^v\d+\.\d+\.\d+$/.test(latest.tag_name || '')) throw new Error('Cannot establish latest stable app release');
  const assets = await github.paginate(github.rest.repos.listReleaseAssets, { ...repo, release_id: latest.id, per_page: 100 });
  const manifests = assets.filter((a) => a.name === 'latest.json');
  if (manifests.length !== 1 || !positive(manifests[0].id) || !/^sha256:[a-f0-9]{64}$/.test(manifests[0].digest || '')) throw new Error('Cannot establish app updater identity');
  return JSON.stringify({ id: latest.id, tag: latest.tag_name, target: latest.target_commitish,
    assets: assets.map(({ id, name, size, digest }) => ({ id, name, size, digest })).sort((a, b) => a.name.localeCompare(b.name)) });
}
async function publishComponents({ github, context, core, root, fetchImpl = fetch }) {
  // Repeat every source/branch/provenance guard after the slow native rebuild.
  const request = await prepareComponents({ github, context, fetchImpl });
  const files = await validateArtifacts(root, request);
  const repo = context.repo; const body = bodyFor(request, files);
  const latest = await latestIdentity(github, repo);
  const assertUnchanged = async () => {
    if (await latestIdentity(github, repo) !== latest) throw new Error('Latest application release changed; stopping component publication');
    const { data: branch } = await github.rest.git.getRef({ ...repo, ref: `heads/${BRANCH}` });
    if (branch.object.sha !== context.sha) throw new Error('Feature branch advanced during publication');
    const target = await tagCommit(github, repo, TAG);
    if (target && target !== request.source_sha) throw new Error('Component tag changed during publication');
  };
  const releases = (await github.paginate(github.rest.repos.listReleases, { ...repo, per_page: 100 })).filter((r) => r.tag_name === TAG);
  if (releases.length > 1) throw new Error('Ambiguous component release');
  let release = releases[0];
  if (release) validateRelease(release, request, body);
  let target = await tagCommit(github, repo, TAG);
  if (release && target !== request.source_sha) throw new Error('Existing release has missing or mismatched immutable tag');
  await assertUnchanged();
  if (!target) {
    await github.rest.git.createRef({ ...repo, ref: `refs/tags/${TAG}`, sha: request.source_sha });
    target = await tagCommit(github, repo, TAG);
    if (target !== request.source_sha) throw new Error('Created tag/source mismatch');
  }
  if (!release) {
    await assertUnchanged();
    release = (await github.rest.repos.createRelease({ ...repo, tag_name: TAG, target_commitish: request.source_sha,
      name: `Luma ASR components ${VERSION}`, body, prerelease: true, draft: true, make_latest: 'false' })).data;
    validateRelease(release, request, body);
  }
  const releaseId = release.id;
  const seenIds = new Map();
  const getState = async (complete = false) => {
    const current = (await github.rest.repos.getRelease({ ...repo, release_id: releaseId })).data;
    validateRelease(current, request, body, releaseId);
    const assets = await github.paginate(github.rest.repos.listReleaseAssets, { ...repo, release_id: releaseId, per_page: 100 });
    validateRemoteAssets(assets, files, current.draft, complete || !current.draft);
    for (const asset of assets) {
      if (seenIds.has(asset.name) && seenIds.get(asset.name) !== asset.id) throw new Error('An existing component asset was replaced');
      seenIds.set(asset.name, asset.id);
    }
    for (const name of seenIds.keys()) {
      if (!assets.some((a) => a.name === name)) throw new Error('An existing component asset disappeared');
    }
    return { current, assets };
  };
  let state = await getState();
  // Verify already uploaded files as bytes, including retries after interruption.
  for (const asset of state.assets) await verifyRemoteAsset(github, repo, asset);
  for (const file of files) {
    if (state.assets.some((a) => a.name === file.name)) continue;
    await assertUnchanged();
    state = await getState();
    if (!state.current.draft) throw new Error('Release was published before all uploads completed');
    if (state.assets.some((a) => a.name === file.name)) throw new Error('Concurrent asset upload; stop instead of replacing');
    const checked = await digestStream(fs.createReadStream(file.file), file.bytes);
    if (!equal(checked, { bytes: file.bytes, sha256: file.sha256 })) throw new Error('Local artifact changed before upload');
    const uploadStream = fs.createReadStream(file.file);
    // A transport can reject before it consumes the stream. Always close the
    // owned descriptor before retry/cleanup (Windows prevents removing it).
    const streamClosed = new Promise((resolve) => uploadStream.once('close', resolve));
    uploadStream.on('error', () => {}); // transport/iteration still receives errors
    let uploaded;
    try {
      ({ data: uploaded } = await github.rest.repos.uploadReleaseAsset({ ...repo, release_id: releaseId, name: file.name,
        headers: { 'content-type': file.name.endsWith('.zip') ? 'application/zip' : 'application/json', 'content-length': file.bytes },
        data: uploadStream }));
    } finally {
      uploadStream.destroy();
      await streamClosed;
    }
    if (uploaded.name !== file.name) throw new Error('Upload returned another asset name');
    validateRemoteAssets([uploaded], files, true);
    await verifyRemoteAsset(github, repo, uploaded);
    state = await getState();
  }
  state = await getState(true);
  const before = JSON.stringify(state.assets.map(({ id, name, size, digest }) => ({ id, name, size, digest })).sort((a, b) => a.name.localeCompare(b.name)));
  await assertUnchanged();
  if (state.current.draft) {
    await github.rest.repos.updateRelease({ ...repo, release_id: releaseId, draft: false, prerelease: true, make_latest: 'false' });
  }
  const after = await getState(true);
  if (after.current.draft || before !== JSON.stringify(after.assets.map(({ id, name, size, digest }) => ({ id, name, size, digest })).sort((a, b) => a.name.localeCompare(b.name)))) throw new Error('Published component identity changed');
  await assertUnchanged();
  core?.info(`Verified ${TAG}: ${files.length} immutable assets; latest application release unchanged.`);
  return { release_id: releaseId, tag: TAG, source_sha: request.source_sha };
}
module.exports = { REPOSITORY, BRANCH, REQUEST_PATH, VERSION, TAG, MAX_ARCHIVE, PACKS, filenames,
  parseRequest, assertContext, validateProvenance, prepareComponents, digestStream, validateManifest,
  validateSmoke, validateArtifacts, bodyFor, validateRelease, validateRemoteAssets, verifyRemoteAsset,
  latestIdentity, publishComponents };
