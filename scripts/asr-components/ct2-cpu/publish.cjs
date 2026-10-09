'use strict';

// A future, reviewed request-only commit is the only mutation entry point.
// Proof success never authorizes whole-runtime, CRT, Python or model publication.
const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { Readable } = require('node:stream');
const { SHA, assertRequestOnlyTree, tagCommit } = require('../../prepare-release.cjs');
const { publicApi } = require('../../recover-release.cjs');
const { validateProofCommit, PROOF_JOB } = require('./proof_request.cjs');
const REPOSITORY = 'csic21/luma-subtitle';
const BRANCH = 'feat/optional-asr-engines';
const REQUEST_PATH = '.github/asr-cpu-wheel-request.json';
const TAG = 'asr-ct2-cpu-4.8.2-1';
const VARIANT = 'luma-cpu-seq-1';
const BUILD_STRATEGY = { kind: 'fresh-fixed-native-root', native_root_relative: 'native-build', builds: 2,
  native_object_cache_reused: false, path_independence_claim: false, cross_machine_claim: false };
const NAMES = {
  wheel: 'ctranslate2-4.8.2-1lumacpu-cp312-cp312-win_amd64.whl',
  source: 'luma-ct2-cpu-4.8.2-1-sources.zip',
  notices: 'luma-ct2-cpu-4.8.2-1-notices.zip',
  proof: 'publication-proof.json',
};
// Independently inventoried projection of the exact locked upstream archive.
// The original archive remains the native build input.
const CT2_SOURCE_EXPORT = {
  "build_tests": "OFF",
  "component": "ctranslate2",
  "exported": {
    "bytes": 5213184,
    "name": "upstream/ctranslate2-d44d2d069eb88c7b7804da864c10c201501cb4a9-source-only.tar",
    "sha256": "9aaf2794e536922999ab0f6c032769659585f361693f52c804c888e309305d88"
  },
  "omitted_members": [
    {
      "bytes": 391462,
      "name": "CTranslate2-d44d2d069eb88c7b7804da864c10c201501cb4a9/tests/data/models/v1/aren-transliteration-i16/model.bin",
      "sha256": "0679202c8d70910962443093d3c9b981374bbf176677592e8b2947329b7e230e"
    },
    {
      "bytes": 744294,
      "name": "CTranslate2-d44d2d069eb88c7b7804da864c10c201501cb4a9/tests/data/models/v1/aren-transliteration/model.bin",
      "sha256": "dcc65eeba74b8cef968489e6b7740af9fea9f42ce45a567f2b95b176e851c39f"
    },
    {
      "bytes": 392956,
      "name": "CTranslate2-d44d2d069eb88c7b7804da864c10c201501cb4a9/tests/data/models/v2/aren-transliteration-i16/model.bin",
      "sha256": "199790bdebb09c4552fef17baa261e3f14661cafb123bd4f75162735f4b78eab"
    },
    {
      "bytes": 233984,
      "name": "CTranslate2-d44d2d069eb88c7b7804da864c10c201501cb4a9/tests/data/models/v2/aren-transliteration-i8/model.bin",
      "sha256": "f99f62a5b5037a951c36be758668457cf7c13af8ce19cdd15b2f0d45d0f073f2"
    },
    {
      "bytes": 741720,
      "name": "CTranslate2-d44d2d069eb88c7b7804da864c10c201501cb4a9/tests/data/models/v2/aren-transliteration/model.bin",
      "sha256": "8870c0c32eb5dcce3f0a004cd5e13383d365d16d905a6f3c8d9732b98a30112d"
    }
  ],
  "original": {
    "bytes": 3434163,
    "name": "upstream/ctranslate2-d44d2d069eb88c7b7804da864c10c201501cb4a9.tar.gz",
    "sha256": "9ff1d14d178f2ba1334b100bc0fb37049ce03b0676520be51e4fbeb9bc682295",
    "url": "https://codeload.github.com/OpenNMT/CTranslate2/tar.gz/d44d2d069eb88c7b7804da864c10c201501cb4a9"
  },
  "retained_member_count": 570,
  "retained_member_records_preserved": true,
  "retained_members_sha256": "98d15144fe967bc6e52cfa518c57a207937fca2d5ed1d77ad08c4d4dbcefcec4",
  "transformation": "Only five reviewed unused model.bin test fixtures omitted; all retained tar records are byte-for-byte unchanged."
};
const MAX_ASSET = 100_000_000;
const MAX_JSON = 4_000_000;
const CHECKS = ['wheel_reproduced', 'whole_runtime_closure', 'native_inference', 'cold_and_warm',
  'isolated', 'private_crt', 'cpu_only', 'compiler_probe'];
const positive = n => Number.isSafeInteger(n) && n > 0;
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const keys = (o, expected) => o && typeof o === 'object' && !Array.isArray(o)
  && Object.keys(o).sort().join(',') === expected.split(',').sort().join(',');
const canonical = value => JSON.stringify(value, function (key, item) {
  return item && typeof item === 'object' && !Array.isArray(item)
    ? Object.fromEntries(Object.entries(item).sort(([a], [b]) => a.localeCompare(b))) : item;
});
const equal = (a, b) => canonical(a) === canonical(b);
function assertPin(pin, name, max = MAX_ASSET) {
  if (!keys(pin, 'name,bytes,sha256') || pin.name !== name || !positive(pin.bytes)
      || pin.bytes >= max || !/^[a-f0-9]{64}$/.test(pin.sha256 || '')) throw new Error('Invalid exact filename, size or SHA-256 pin');
}
function assertAssets(assets) {
  const names = [NAMES.wheel, NAMES.source, NAMES.notices].sort();
  if (!Array.isArray(assets) || !equal(assets.map(a => a?.name).sort(), names)) throw new Error('Expected only wheel, complete source and notices');
  for (const item of assets) assertPin(item, item.name);
}
function assertLockPins(locks) {
  if (!keys(locks, 'sources,notices')) throw new Error('Missing exact source/notice locks');
  assertPin(locks.sources, 'sources.lock.json', MAX_JSON);
  assertPin(locks.notices, 'notices.lock.json', MAX_JSON);
}
function parseRequest(text, commit) {
  if (Buffer.byteLength(text) >= MAX_JSON) throw new Error('Oversized request');
  const r = JSON.parse(text);
  if (!keys(r, 'schema_version,repository,branch,source_sha,tag,proof,assets,locks,review')
      || r.schema_version !== 1 || r.repository !== REPOSITORY || r.branch !== BRANCH
      || r.tag !== TAG || !SHA.test(r.source_sha || '')
      || commit.parents?.length !== 1 || commit.parents[0].sha !== r.source_sha
      || !keys(r.proof, 'run_id,run_attempt,summary') || !positive(r.proof.run_id) || !positive(r.proof.run_attempt)
      || !keys(r.review, 'scope,approved') || r.review.scope !== 'cpu-wheel-source-notices-only'
      || r.review.approved !== true) throw new Error('Invalid narrow publication request or sole tested-source parent');
  assertPin(r.proof.summary, NAMES.proof, MAX_JSON);
  assertAssets(r.assets); assertLockPins(r.locks);
  return r;
}
function assertContext(context) {
  if (`${context.repo.owner}/${context.repo.repo}` !== REPOSITORY || context.eventName !== 'push'
      || context.ref !== `refs/heads/${BRANCH}` || context.payload.deleted
      || context.payload.repository?.full_name !== REPOSITORY || !SHA.test(context.sha || '')) {
    throw new Error('Only the intended repository and request-only feature-branch push can publish');
  }
}
async function readSource(github, repo, file, ref) {
  const { data } = await github.rest.repos.getContent({ ...repo, path: file, ref });
  if (data.type !== 'file' || data.encoding !== 'base64') throw new Error(`Invalid source file: ${file}`);
  const bytes = Buffer.from(data.content, 'base64');
  if (!positive(bytes.length) || bytes.length >= MAX_JSON) throw new Error('Source file exceeds bounds');
  return bytes;
}
function validateProvenance(run, jobs, r, proofCommit = null) {
  if (run.id !== r.proof.run_id || run.run_attempt !== r.proof.run_attempt
      || run.repository?.full_name !== REPOSITORY || run.head_repository?.full_name !== REPOSITORY
      || !['pull_request', 'workflow_dispatch', 'push'].includes(run.event) || run.head_branch !== BRANCH
      || run.head_sha !== r.source_sha || run.path !== '.github/workflows/asr-ct2-cpu.yml'
      || run.status !== 'completed' || run.conclusion !== 'success') throw new Error('Historical native proof identity/status mismatch');
  if (run.event === 'push' && (proofCommit?.source_sha !== r.source_sha
      || proofCommit.base_sha !== proofCommit.request?.source_sha)) throw new Error('Push proof requires independently validated request-only commit');
  // Never use run.pull_requests[].head.sha: GitHub mutates that field on later pushes.
  if (run.event === 'pull_request' && (!Array.isArray(run.pull_requests) || run.pull_requests.length !== 1
      || run.pull_requests[0].head?.ref !== BRANCH || run.pull_requests[0].base?.ref !== 'main'
      || run.pull_requests[0].head?.repo?.url !== `https://api.github.com/repos/${REPOSITORY}`
      || run.pull_requests[0].base?.repo?.url !== `https://api.github.com/repos/${REPOSITORY}`)) throw new Error('Historical native proof PR repository mismatch');
  if (!Array.isArray(jobs.jobs) || jobs.total_count !== jobs.jobs.length || jobs.total_count > 100
      || jobs.jobs.some(j => j.run_id !== run.id || j.head_sha !== run.head_sha
        || j.status !== 'completed' || (j.conclusion !== 'success'
          && !(run.event === 'workflow_dispatch' && j.name === PROOF_JOB && j.conclusion === 'skipped')))) throw new Error('Incomplete or unsuccessful historical native jobs');
  const required = ['windows-cpu-proof', 'windows-direct-crt-signatures'];
  if (run.event === 'push') required.push('CPU proof source guards', PROOF_JOB);
  for (const name of required) {
    if (jobs.jobs.filter(j => j.name === name && j.conclusion === 'success').length !== 1) throw new Error(`Missing unique successful native job: ${name}`);
  }
}
function validateLocks(sources, notices) {
  if (sources.schema !== 1 || sources.variant !== VARIANT || sources.wheel_build_tag !== '1lumacpu'
      || sources.python !== '3.12.15' || sources.publication_authorized !== false
      || notices.schema !== 1 || notices.notice_inputs_complete !== true || !equal(notices.blockers, [])
      || notices.independent_redistribution_review_passed !== false || !Array.isArray(notices.files)
      || notices.files.length < 11) throw new Error('Unreviewed build/notice scope or whole-runtime authorization');
  const ct = sources.ct2_cmake, dn = sources.onednn_cmake;
  for (const key of ['WITH_MKL','WITH_CUDA','WITH_CUDNN','WITH_HIP','WITH_RUY','WITH_OPENBLAS','WITH_ACCELERATE','WITH_TENSOR_PARALLEL','WITH_FLASH_ATTN','CUDA_DYNAMIC_LOADING']) {
    if (ct?.[key] !== 'OFF') throw new Error('Forbidden CPU-wheel backend');
  }
  if (ct.BUILD_TESTS !== 'OFF' || ct.OPENMP_RUNTIME !== 'NONE' || ct.WITH_DNNL !== 'ON' || ct.BUILD_SHARED_LIBS !== 'ON'
      || dn?.DNNL_CPU_RUNTIME !== 'SEQ' || dn.DNNL_GPU_RUNTIME !== 'NONE'
      || dn.DNNL_LIBRARY_TYPE !== 'STATIC' || dn.DNNL_BLAS_VENDOR !== 'NONE'
      || ct.CMAKE_MSVC_RUNTIME_LIBRARY !== 'MultiThreadedDLL' || dn.CMAKE_MSVC_RUNTIME_LIBRARY !== 'MultiThreadedDLL') throw new Error('Unreviewed oneDNN/CRT build options');
}
async function prepareWheel({ github, context, core, fetchImpl = fetch, inspectPublished = false }) {
  assertContext(context);
  const repo = context.repo;
  const { data: commit } = await github.rest.repos.getCommit({ ...repo, ref: context.sha });
  const request = parseRequest((await readSource(github, repo, REQUEST_PATH, context.sha)).toString('utf8'), commit);
  await assertRequestOnlyTree(github, repo, request.source_sha, context.sha, path.basename(REQUEST_PATH));
  const { data: branch } = await github.rest.git.getRef({ ...repo, ref: `heads/${BRANCH}` });
  if (branch.object.sha !== context.sha) throw new Error('Feature branch advanced; stale request');
  const locks = {};
  for (const [kind, pin] of Object.entries(request.locks)) {
    const bytes = await readSource(github, repo, `scripts/asr-components/ct2-cpu/${pin.name}`, request.source_sha);
    if (bytes.length !== pin.bytes || hash(bytes) !== pin.sha256) throw new Error('Request differs from exact tested-source lock');
    locks[kind] = JSON.parse(bytes.toString('utf8'));
  }
  validateLocks(locks.sources, locks.notices);
  const run = await publicApi(repo, `actions/runs/${request.proof.run_id}`, fetchImpl);
  const jobs = await publicApi(repo, `actions/runs/${request.proof.run_id}/attempts/${request.proof.run_attempt}/jobs?per_page=100`, fetchImpl);
  const proofCommit = run.event === 'push'
    ? await validateProofCommit({ github, repo, sha: request.source_sha }) : null;
  validateProvenance(run, jobs, request, proofCommit);
  const target = await tagCommit(github, repo, TAG);
  if (target && target !== request.source_sha) throw new Error('Immutable component tag has another source');
  core?.setOutput('source_sha', request.source_sha);
  const alreadyPublished = inspectPublished ? await verifyPublished({ github, context, request, locks }) : false;
  core?.setOutput('already_published', String(alreadyPublished));
  return { request, locks, alreadyPublished };
}
async function digestStream(stream, max) {
  const digest = createHash('sha256'); let bytes = 0;
  for await (const chunk of stream) { bytes += chunk.length; if (bytes > max) throw new Error('Asset exceeds pinned size'); digest.update(chunk); }
  return { bytes, sha256: digest.digest('hex') };
}
function validateProof(proof, request, locks) {
  if (!keys(proof, 'schema,source_sha,variant,publication_authorized,assets,locks,checks,provenance,source_exports')
      || proof.schema !== 1 || proof.source_sha !== request.source_sha || proof.variant !== VARIANT
      || proof.publication_authorized !== false || !equal(proof.assets, request.assets)
      || !equal(proof.locks, request.locks) || !keys(proof.checks, CHECKS.join(','))
      || CHECKS.some(check => proof.checks[check] !== true)) throw new Error('Missing or mismatched native/reproducibility/closure/inference proof');
  if (!equal(proof.source_exports, [CT2_SOURCE_EXPORT])) throw new Error('Source-only export or exact omitted-model inventory differs');
  const ct2 = locks.sources.sources.find(item => item.name === 'ctranslate2');
  if (!ct2 || ct2.url !== CT2_SOURCE_EXPORT.original.url || ct2.bytes !== CT2_SOURCE_EXPORT.original.bytes
      || ct2.sha256 !== CT2_SOURCE_EXPORT.original.sha256) throw new Error('Original CT2 source archive pin changed');
  const p = proof.provenance, s = locks.sources;
  if (p?.schema !== 1 || p.variant !== VARIANT || p.luma_source_sha !== request.source_sha
      || p.publication_authorized !== false || !equal(p.build_strategy, BUILD_STRATEGY) || !equal(p.upstream, s.sources) || !equal(p.build_wheels, s.build_wheels)
      || !equal(p.ct2_cmake, s.ct2_cmake) || !equal(p.onednn_cmake, s.onednn_cmake)
      || !equal(p.notices, locks.notices) || p.source_date_epoch !== s.source_date_epoch
      || p.native_compile_flags !== '/Brepro /Z7 /experimental:deterministic /pathmap:<build-root>=C:\\luma-ct2-build'
      || p.native_link_flags !== '/Brepro /INCREMENTAL:NO'
      || !p.python_runtime || !/^[a-f0-9]{64}$/.test(p.python_runtime.sha256 || '')
      || !positive(p.python_runtime.bytes) || !p.toolchain?.binary_hashes) throw new Error('Rebuild source/download/toolchain/provenance mismatch');
  for (const key of ['visual_studio_version', 'vc_tools_version', 'windows_sdk_version']) {
    if (p.toolchain[key] !== s.toolchain[key]) throw new Error('Unreviewed compiler/SDK version');
  }
  for (const name of ['cl.exe', 'link.exe', 'lib.exe', 'rc.exe', 'mt.exe']) {
    if (!/^[a-f0-9]{64}$/.test(p.toolchain.binary_hashes[name]?.sha256 || '')) throw new Error('Missing compiler binary digest');
  }
}
async function validateArtifacts(root, request, locks) {
  const stat = fs.lstatSync(root);
  if (!stat.isDirectory() || stat.isSymbolicLink() || !equal(fs.readdirSync(root).sort(), Object.values(NAMES).sort())) throw new Error('Unexpected publication directory; private runtime/model files are forbidden');
  const files = [];
  for (const pin of [...request.assets, request.proof.summary]) {
    const file = path.join(root, pin.name); const stat = fs.lstatSync(file);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.size !== pin.bytes) throw new Error('Missing, symlink or changed candidate file');
    const digest = await digestStream(fs.createReadStream(file), pin.bytes);
    if (digest.bytes !== pin.bytes || digest.sha256 !== pin.sha256) throw new Error('Rebuild differs from reviewed successful-proof bytes');
    files.push({ ...pin, file });
  }
  validateProof(JSON.parse(fs.readFileSync(path.join(root, NAMES.proof), 'utf8')), request, locks);
  return files.sort((a, b) => a.name.localeCompare(b.name));
}
const assetUrl = name => `https://github.com/${REPOSITORY}/releases/download/${TAG}/${name}`;
function bodyFor(request) {
  return `Luma CPU-only CTranslate2 4.8.2 (Windows x64 / CPython 3.12).\n\nOnly the reviewed wheel, its compilation source and notices, and exact build provenance are included. The clearly labeled source-only export omits exactly five unused upstream model.bin test fixtures; every other source archive member and its metadata is retained unchanged. The original source archive remains the pinned build input. Python, Microsoft CRT, FFmpeg, other runtime wheels and models are not redistributed. Native CPU cold/warm Tiny inference, private-runtime dependency closure and two fresh builds at one fixed native directory passed for the pinned source and toolchain; no path-independence, cross-machine, cross-toolchain or performance claim is made.\n\nReviewed source: ${request.source_sha}\nNative proof: https://github.com/${REPOSITORY}/actions/runs/${request.proof.run_id}/attempts/${request.proof.run_attempt}\n\n<!-- luma-ct2-cpu:${hash(canonical(request))} -->`;
}
function validateRelease(release, request, body, id = release?.id) {
  if (!positive(id) || release.id !== id || release.tag_name !== TAG || release.prerelease !== true
      || typeof release.draft !== 'boolean' || release.target_commitish !== request.source_sha
      || release.name !== 'Luma CPU-only CTranslate2 4.8.2 build 1' || release.body !== body) throw new Error('Existing component release differs; refusing overwrite');
}
function validateRemoteAssets(assets, files, draft, complete = false) {
  if (!Array.isArray(assets) || new Set(assets.map(a => a.id)).size !== assets.length
      || new Set(assets.map(a => a.name)).size !== assets.length || (complete && assets.length !== files.length)) throw new Error('Incomplete or duplicate component assets');
  for (const asset of assets) {
    const pin = files.find(f => f.name === asset.name);
    const prefix = `https://github.com/${REPOSITORY}/releases/download/`;
    const tail = typeof asset.browser_download_url === 'string' && asset.browser_download_url.startsWith(prefix)
      ? asset.browser_download_url.slice(prefix.length).split('/') : [];
    const urlOkay = asset.browser_download_url === assetUrl(asset.name)
      || (draft && tail.length === 2 && /^untagged-[a-f0-9]{20}$/.test(tail[0]) && tail[1] === asset.name);
    if (!pin || !positive(asset.id) || asset.state !== 'uploaded' || asset.size !== pin.bytes
        || asset.digest !== `sha256:${pin.sha256}` || !urlOkay
        || asset.url !== `https://api.github.com/repos/${REPOSITORY}/releases/assets/${asset.id}`) throw new Error('Existing asset differs; refusing replacement');
  }
}
async function verifyRemoteAsset(github, repo, asset, capture = false) {
  if (capture && (!positive(asset.size) || asset.size >= MAX_JSON)) throw new Error('Remote proof exceeds bounds');
  const { data } = await github.rest.repos.getReleaseAsset({ ...repo, asset_id: asset.id,
    headers: { accept: 'application/octet-stream' }, request: { parseSuccessResponseBody: false } });
  let stream;
  if (data?.getReader) stream = Readable.fromWeb(data);
  else if (data?.[Symbol.asyncIterator]) stream = data;
  else if (data instanceof ArrayBuffer || ArrayBuffer.isView(data)) stream = Readable.from([Buffer.from(data)]);
  else throw new Error('Asset API did not return bytes');
  const chunks = [];
  const tapped = (async function* () { for await (const chunk of stream) { if (capture) chunks.push(Buffer.from(chunk)); yield chunk; } })();
  const result = await digestStream(tapped, asset.size);
  if (result.bytes !== asset.size || `sha256:${result.sha256}` !== asset.digest) throw new Error('Downloaded remote bytes differ');
  return capture ? Buffer.concat(chunks) : result;
}
const assetIdentity = assets => canonical(assets.map(({ id, name, size, digest }) => ({ id, name, size, digest })).sort((a, b) => a.name.localeCompare(b.name)));
async function latestIdentity(github, repo) {
  const { data: latest } = await github.rest.repos.getLatestRelease(repo);
  if (!positive(latest.id) || latest.draft || latest.prerelease || !/^v\d+\.\d+\.\d+$/.test(latest.tag_name || '')) throw new Error('Cannot establish stable latest app release');
  const assets = await github.paginate(github.rest.repos.listReleaseAssets, { ...repo, release_id: latest.id, per_page: 100 });
  const manifests = assets.filter(a => a.name === 'latest.json');
  if (manifests.length !== 1 || !positive(manifests[0].id) || !positive(manifests[0].size)
      || manifests[0].size >= MAX_JSON || !/^sha256:[a-f0-9]{64}$/.test(manifests[0].digest || '')
      || new Set(assets.map(a => a.id)).size !== assets.length) throw new Error('Cannot establish app updater identity');
  await verifyRemoteAsset(github, repo, manifests[0]);
  const target = await tagCommit(github, repo, latest.tag_name);
  if (!target) throw new Error('Stable application tag is missing');
  return canonical({ id: latest.id, tag: latest.tag_name, target_commitish: latest.target_commitish, target, assets: assetIdentity(assets) });
}
async function verifyPublished({ github, context, request, locks }) {
  const repo = context.repo;
  const releases = (await github.paginate(github.rest.repos.listReleases, { ...repo, per_page: 100 })).filter(r => r.tag_name === TAG);
  if (releases.length > 1) throw new Error('Ambiguous component releases');
  if (!releases.length) return false;
  const release = releases[0], body = bodyFor(request), files = [...request.assets, request.proof.summary];
  validateRelease(release, request, body);
  if (await tagCommit(github, repo, TAG) !== request.source_sha) throw new Error('Existing release tag/source mismatch');
  if (release.draft) return false;
  const latest = await latestIdentity(github, repo);
  const assets = await github.paginate(github.rest.repos.listReleaseAssets, { ...repo, release_id: release.id, per_page: 100 });
  validateRemoteAssets(assets, files, false, true);
  for (const asset of assets) {
    const bytes = await verifyRemoteAsset(github, repo, asset, asset.name === NAMES.proof);
    if (asset.name === NAMES.proof) validateProof(JSON.parse(bytes.toString('utf8')), request, locks);
  }
  const { data: after } = await github.rest.repos.getRelease({ ...repo, release_id: release.id });
  validateRelease(after, request, body, release.id);
  const afterAssets = await github.paginate(github.rest.repos.listReleaseAssets, { ...repo, release_id: release.id, per_page: 100 });
  validateRemoteAssets(afterAssets, files, false, true);
  const { data: branch } = await github.rest.git.getRef({ ...repo, ref: `heads/${BRANCH}` });
  if (after.draft || assetIdentity(afterAssets) !== assetIdentity(assets) || await latestIdentity(github, repo) !== latest
      || await tagCommit(github, repo, TAG) !== request.source_sha || branch.object.sha !== context.sha) throw new Error('Published retry identity changed');
  return true;
}

async function publishWheel({ github, context, core, root, fetchImpl = fetch }) {
  // Repeat all live read-only source/proof guards after the long native rebuild.
  const { request, locks } = await prepareWheel({ github, context, fetchImpl });
  const files = await validateArtifacts(root, request, locks);
  const repo = context.repo, body = bodyFor(request), latest = await latestIdentity(github, repo);
  let tagEstablished = false;
  const assertUnchanged = async () => {
    if (await latestIdentity(github, repo) !== latest) throw new Error('Latest application release or updater changed');
    const { data: branch } = await github.rest.git.getRef({ ...repo, ref: `heads/${BRANCH}` });
    if (branch.object.sha !== context.sha) throw new Error('Feature branch advanced during publication');
    const target = await tagCommit(github, repo, TAG);
    if ((tagEstablished && target !== request.source_sha) || (target && target !== request.source_sha)) throw new Error('Component tag changed or disappeared');
  };
  const releases = (await github.paginate(github.rest.repos.listReleases, { ...repo, per_page: 100 })).filter(r => r.tag_name === TAG);
  if (releases.length > 1) throw new Error('Ambiguous component releases');
  let release = releases[0], target = await tagCommit(github, repo, TAG);
  if (release) { validateRelease(release, request, body); if (target !== request.source_sha) throw new Error('Existing release tag/source mismatch'); }
  await assertUnchanged();
  if (!target) {
    await github.rest.git.createRef({ ...repo, ref: `refs/tags/${TAG}`, sha: request.source_sha });
    if (await tagCommit(github, repo, TAG) !== request.source_sha) throw new Error('Created component tag/source mismatch');
  }
  tagEstablished = true;
  if (!release) {
    await assertUnchanged();
    release = (await github.rest.repos.createRelease({ ...repo, tag_name: TAG, target_commitish: request.source_sha,
      name: 'Luma CPU-only CTranslate2 4.8.2 build 1', body, prerelease: true, draft: true, make_latest: 'false' })).data;
    validateRelease(release, request, body);
  }
  const releaseId = release.id, seen = new Map();
  const getState = async (complete = false) => {
    const current = (await github.rest.repos.getRelease({ ...repo, release_id: releaseId })).data;
    validateRelease(current, request, body, releaseId);
    const assets = await github.paginate(github.rest.repos.listReleaseAssets, { ...repo, release_id: releaseId, per_page: 100 });
    validateRemoteAssets(assets, files, current.draft, complete || !current.draft);
    for (const a of assets) { if (seen.has(a.name) && seen.get(a.name) !== a.id) throw new Error('Existing asset was replaced'); seen.set(a.name, a.id); }
    for (const name of seen.keys()) if (!assets.some(a => a.name === name)) throw new Error('Existing asset disappeared');
    return { current, assets };
  };
  let state = await getState();
  for (const asset of state.assets) await verifyRemoteAsset(github, repo, asset);
  for (const file of files) {
    if (state.assets.some(a => a.name === file.name)) continue;
    await assertUnchanged(); state = await getState();
    if (!state.current.draft || state.assets.some(a => a.name === file.name)) throw new Error('Concurrent publication/upload; refusing replacement');
    if (!equal(await digestStream(fs.createReadStream(file.file), file.bytes), { bytes: file.bytes, sha256: file.sha256 })) throw new Error('Local candidate changed before upload');
    const stream = fs.createReadStream(file.file), closed = new Promise(resolve => stream.once('close', resolve));
    stream.on('error', () => {});
    let uploaded;
    try {
      ({ data: uploaded } = await github.rest.repos.uploadReleaseAsset({ ...repo, release_id: releaseId, name: file.name,
        headers: { 'content-type': file.name.endsWith('.json') ? 'application/json' : 'application/zip', 'content-length': file.bytes }, data: stream }));
    } finally { stream.destroy(); await closed; }
    if (uploaded.name !== file.name) throw new Error('Upload returned another name');
    validateRemoteAssets([uploaded], files, true); await verifyRemoteAsset(github, repo, uploaded);
    state = await getState();
  }
  state = await getState(true); const before = assetIdentity(state.assets);
  await assertUnchanged();
  if (state.current.draft) await github.rest.repos.updateRelease({ ...repo, release_id: releaseId, draft: false, prerelease: true, make_latest: 'false' });
  const after = await getState(true);
  if (after.current.draft || assetIdentity(after.assets) !== before) throw new Error('Published asset identity changed');
  await assertUnchanged();
  core?.info(`Verified immutable ${TAG}; latest app release and latest.json are unchanged.`);
  return { tag: TAG, source_sha: request.source_sha, release_id: releaseId };
}
module.exports = { REPOSITORY, BRANCH, REQUEST_PATH, TAG, VARIANT, NAMES, MAX_ASSET, CHECKS, CT2_SOURCE_EXPORT, BUILD_STRATEGY,
  parseRequest, assertContext, validateProvenance, validateLocks, prepareWheel, digestStream,
  validateProof, validateArtifacts, bodyFor, validateRelease, validateRemoteAssets, verifyRemoteAsset,
  latestIdentity, verifyPublished, publishWheel, hash, equal };
