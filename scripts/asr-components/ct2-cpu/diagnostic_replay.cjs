'use strict';

// One known failed native proof only. This never authorizes release or replaces
// successful same-source publication evidence, and has no remote mutation API.
const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { SHA } = require('../../prepare-release.cjs');
const { validateProofCommit, DIAGNOSTIC_ARTIFACT } = require('./proof_request.cjs');
const REPOSITORY = 'csic21/luma-subtitle';
const REPOSITORY_ID = 1241316830;
const BRANCH = 'feat/optional-asr-engines';
const REQUEST_PATH = '.github/requests/ct2-cpu-replay.json';
const PURPOSE = 'diagnostic-cpu-wheel-replay';
const PRODUCER = Object.freeze({ source_sha: '3006b955bb248e83e95f2dcc48c3001194513800',
  run_id: 37953677195, run_attempt: 1, job_id: 113898825946 });
const ARTIFACT = Object.freeze({ id: 11629892995, bytes: 15478967,
  sha256: '864aa80bbda7cb11f59c6bdc4f418e37ee1ca66c3a7e5583703fcc16b1b9e222' });
const ARTIFACT_NAME = `ct2-cpu-DIAGNOSTIC-NOT-FOR-RELEASE-${PRODUCER.source_sha}-${PRODUCER.run_id}-${PRODUCER.run_attempt}`;
const FILES = Object.freeze([
  { name: 'ctranslate2-4.8.2-1lumacpu-cp312-cp312-win_amd64.whl', bytes: 25058630, sha256: '4644c5eb94492ab61d9749ee122e12c612c03634495d9cb59b6c18e3404cd1c0' },
  { name: 'luma-ct2-cpu-4.8.2-1-sources.zip', bytes: 13935077, sha256: 'c193ede80310da389aab0637b78493f0c7ba7ea0c6debac68701ec961306fd4a' },
  { name: 'luma-ct2-cpu-4.8.2-1-notices.zip', bytes: 84941, sha256: '73667a775f8a46408b265415f1a8b0a085570292be2b64210bad3526b131f69f' },
  { name: 'diagnostic-manifest.json', bytes: 19012, sha256: 'fad46ac92720c75897393521942fc163b2ce183adef62ee7cda9938cb91a30b9' },
].map(Object.freeze));
const MANIFEST = FILES[3];
const MAX_REQUEST = 4096;
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const canonical = value => Array.isArray(value) ? value.map(canonical) : value && typeof value === 'object'
  ? Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])])) : value;
const equal = (a, b) => JSON.stringify(canonical(a)) === JSON.stringify(canonical(b));
const keys = (value, names) => value && typeof value === 'object' && !Array.isArray(value)
  && Object.keys(value).sort().join(',') === names.split(',').sort().join(',');

function parseRequest(text, commit) {
  if (Buffer.byteLength(text) >= MAX_REQUEST) throw new Error('Replay request exceeds bounds');
  const r = JSON.parse(text);
  if (!keys(r, 'schema_version,purpose,source_sha,producer,artifact,manifest') || r.schema_version !== 1
      || r.purpose !== PURPOSE || !SHA.test(r.source_sha || '') || commit.parents?.length !== 1
      || commit.parents[0].sha !== r.source_sha || !equal(r.producer, PRODUCER)
      || !equal(r.artifact, ARTIFACT) || !equal(r.manifest, MANIFEST)) {
    throw new Error('Replay must name its sole parent and the one exact approved failed-build artifact');
  }
  return r;
}
async function tree(github, repo, sha) {
  if (!sha) return [];
  const { data } = await github.rest.git.getTree({ ...repo, tree_sha: sha });
  if (data.truncated || !Array.isArray(data.tree) || data.tree.length > 10000
      || new Set(data.tree.map(e => e.path)).size !== data.tree.length
      || data.tree.some(e => typeof e.path !== 'string' || !e.path || /[\\/]/.test(e.path) || ['.', '..'].includes(e.path) || !SHA.test(e.sha || ''))) {
    throw new Error('Incomplete or invalid replay request tree');
  }
  return data.tree;
}
const siblings = (entries, name) => entries.filter(e => e.path !== name)
  .map(({ path, mode, type, sha }) => ({ path, mode, type, sha })).sort((a, b) => a.path.localeCompare(b.path));
async function assertReplayTree(github, repo, parent, request) {
  const parts = REQUEST_PATH.split('/'); let beforeSha = parent, afterSha = request;
  for (const [index, name] of parts.entries()) {
    const [before, after] = await Promise.all([tree(github, repo, beforeSha), tree(github, repo, afterSha)]);
    if (!equal(siblings(before, name), siblings(after, name))) throw new Error('Replay push must change only its inert request');
    const a = before.find(e => e.path === name), b = after.find(e => e.path === name);
    if (index === parts.length - 1) {
      if (b?.type !== 'blob' || b.mode !== '100644' || a?.sha === b.sha
          || (a && (a.type !== 'blob' || a.mode !== '100644'))) throw new Error('Replay request must be a changed regular 100644 file');
      return b.sha;
    }
    if (b?.type !== 'tree' || b.mode !== '040000' || (a && (a.type !== 'tree' || a.mode !== '040000'))) {
      throw new Error('Replay request ancestors must be ordinary directories');
    }
    beforeSha = a?.sha; afterSha = b.sha;
  }
  throw new Error('Missing replay request');
}
async function prepareReplayRequest({ github, context, core }) {
  if (`${context.repo.owner}/${context.repo.repo}` !== REPOSITORY || context.eventName !== 'push'
      || context.ref !== `refs/heads/${BRANCH}` || context.payload.deleted
      || context.payload.repository?.full_name !== REPOSITORY || !SHA.test(context.sha || '')) {
    throw new Error('Replay requires the intended feature-branch request-only push');
  }
  const repo = context.repo;
  const { data: commit } = await github.rest.repos.getCommit({ ...repo, ref: context.sha });
  if (commit.sha !== context.sha) throw new Error('Replay commit resolution changed');
  const { data } = await github.rest.repos.getContent({ ...repo, path: REQUEST_PATH, ref: context.sha });
  if (data.type !== 'file' || data.encoding !== 'base64') throw new Error('Replay request content is not a regular file');
  const request = parseRequest(Buffer.from(data.content, 'base64').toString('utf8'), commit);
  const blob = await assertReplayTree(github, repo, request.source_sha, context.sha);
  if (data.sha !== blob) throw new Error('Replay request content differs from its Git blob');
  if (context.payload.before !== request.source_sha || context.payload.after !== context.sha || context.payload.forced !== false) {
    throw new Error('Replay requires one non-forced request-only commit');
  }
  const { data: branch } = await github.rest.git.getRef({ ...repo, ref: `heads/${BRANCH}` });
  if (branch.object?.type !== 'commit' || branch.object.sha !== context.sha) throw new Error('Feature branch advanced; stale replay request');
  core?.setOutput('verifier_source_sha', context.sha);
  core?.setOutput('base_sha', request.source_sha);
  return { request, verifier_source_sha: context.sha, base_sha: request.source_sha };
}
function validateProducer(run, jobs) {
  if (run.id !== PRODUCER.run_id || run.run_attempt !== PRODUCER.run_attempt || run.head_sha !== PRODUCER.source_sha
      || run.repository?.full_name !== REPOSITORY || run.head_repository?.full_name !== REPOSITORY
      || run.repository.id !== REPOSITORY_ID || run.head_repository.id !== REPOSITORY_ID
      || run.event !== 'push' || run.head_branch !== BRANCH || run.path !== '.github/workflows/asr-ct2-cpu.yml'
      || run.status !== 'completed' || run.conclusion !== 'failure') throw new Error('Wrong original failed native proof identity');
  const expected = ['CPU proof source guards', 'Validate request-only CPU proof', 'windows-cpu-proof', 'windows-direct-crt-signatures'];
  if (!Array.isArray(jobs.jobs) || jobs.total_count !== 4 || jobs.jobs.length !== 4
      || !equal(jobs.jobs.map(j => j.name).sort(), expected.sort())
      || jobs.jobs.some(j => j.run_id !== PRODUCER.run_id || j.head_sha !== PRODUCER.source_sha || j.status !== 'completed'
        || j.conclusion !== (j.name === 'windows-cpu-proof' ? 'failure' : 'success'))) {
    throw new Error('Original run jobs do not preserve their exact source and outcomes');
  }
  const cpu = jobs.jobs.find(j => j.name === 'windows-cpu-proof');
  if (cpu.id !== PRODUCER.job_id || !Array.isArray(cpu.steps)) throw new Error('Wrong original CPU job');
  for (const [name, conclusion] of [
    ['Test input locks and binary inventory logic', 'success'],
    ['Inventory official toolchain and build two CPU wheels', 'failure'],
    ['Retain explicit failed-verifier diagnostic (never a release candidate)', 'success'],
    ['Export the three reviewed candidate assets and proof only', 'skipped'],
  ]) if (cpu.steps.filter(s => s.name === name && s.status === 'completed' && s.conclusion === conclusion).length !== 1) {
    throw new Error('Original diagnostic capture/job steps do not match the failed proof');
  }
  return cpu;
}
function validateArtifact(artifact, cpu, now = Date.now()) {
  const origin = artifact.workflow_run;
  const created = Date.parse(artifact.created_at), expires = Date.parse(artifact.expires_at);
  if (artifact.id !== ARTIFACT.id || artifact.name !== ARTIFACT_NAME || artifact.expired !== false
      || artifact.size_in_bytes !== ARTIFACT.bytes || artifact.digest !== `sha256:${ARTIFACT.sha256}`
      || artifact.archive_download_url !== `https://api.github.com/repos/${REPOSITORY}/actions/artifacts/${ARTIFACT.id}/zip`
      || origin?.id !== PRODUCER.run_id || origin.head_sha !== PRODUCER.source_sha || origin.head_branch !== BRANCH
      || origin.repository_id !== REPOSITORY_ID || origin.head_repository_id !== REPOSITORY_ID
      || !Number.isFinite(created) || !Number.isFinite(expires) || expires <= now || created > now
      || expires <= created || expires - created > 86460000
      || !(created >= Date.parse(cpu.started_at) && created <= Date.parse(cpu.completed_at))) {
    throw new Error('Diagnostic artifact identity, producer attempt, or one-day retention differs');
  }
  return artifact;
}
function artifactLocation(location) {
  let url;
  try { url = new URL(location); } catch { throw new Error('Invalid GitHub artifact storage redirect'); }
  if (url.protocol !== 'https:' || url.username || url.password || url.port || url.hash
      || !(url.hostname.endsWith('.blob.core.windows.net') || url.hostname.endsWith('.actions.githubusercontent.com'))) {
    throw new Error('Unexpected GitHub artifact storage redirect');
  }
  return url;
}
async function readPinnedArchive(fetched, pin) {
  const length = fetched.headers.get('content-length');
  if (length !== null && Number(length) !== pin.bytes) throw new Error('Diagnostic archive length header differs');
  const digest = createHash('sha256'), chunks = []; let bytes = 0;
  for await (const chunk of fetched.body) {
    bytes += chunk.length;
    if (bytes > pin.bytes) throw new Error('Diagnostic archive exceeds its pinned size');
    digest.update(chunk); chunks.push(Buffer.from(chunk));
  }
  if (bytes !== pin.bytes || digest.digest('hex') !== pin.sha256) throw new Error('Diagnostic archive bytes/hash differ');
  return Buffer.concat(chunks);
}
function transportSummary(response) {
  const headers = response?.headers;
  const present = name => typeof headers?.get === 'function' ? headers.get(name) !== null : typeof headers?.[name] === 'string';
  let origin = 'unavailable';
  try {
    const url = new URL(response.url);
    origin = url.origin === 'https://api.github.com' ? 'api.github.com'
      : url.protocol === 'https:' && url.hostname.endsWith('.blob.core.windows.net') ? 'github-blob-storage'
      : url.protocol === 'https:' && url.hostname.endsWith('.actions.githubusercontent.com') ? 'github-actions-storage' : 'other';
  } catch {}
  return { status: Number.isInteger(response?.status) && response.status >= 100 && response.status <= 599 ? response.status : null,
    type: ['basic', 'cors', 'default', 'error', 'opaque', 'opaqueredirect'].includes(response?.type) ? response.type : 'unavailable',
    redirected: typeof response?.redirected === 'boolean' ? response.redirected : null, origin,
    headers: typeof headers?.get === 'function' ? 'fetch' : headers && typeof headers === 'object' ? 'object' : 'unavailable',
    location_present: present('location'), content_type_present: present('content-type'), content_length_present: present('content-length') };
}
function transportError(stage, summary) {
  const error = new Error(`Diagnostic artifact ${stage} failed: ${JSON.stringify(summary)}`);
  if (summary.status !== null) error.status = summary.status;
  return error;
}
// Shared transport only: callers must independently validate their bounded
// source/run/job/artifact policy. This helper never executes or publishes bytes.
async function downloadPinnedArchive({ github, repo, destination, artifact, fetchImpl = fetch, core }) {
  if (`${repo.owner}/${repo.repo}` !== REPOSITORY) throw new Error('Diagnostic downloads are restricted to the exact repository');
  if (!artifact || !Number.isSafeInteger(artifact.id) || artifact.id <= 0
      || !Number.isSafeInteger(artifact.bytes) || artifact.bytes <= 0 || artifact.bytes > 304000000
      || !/^[a-f0-9]{64}$/.test(artifact.sha256 || '')) throw new Error('Invalid bounded artifact transport pin');
  if (fs.existsSync(destination)) throw new Error('Replay download destination must be fresh');
  const apiUrl = `https://api.github.com/repos/${REPOSITORY}/actions/artifacts/${artifact.id}/zip`;
  // github-script v7.0.1 bundles @octokit/request 8.1.1, which does not forward
  // request.redirect. Its supported fetch hook must enforce it at the boundary:
  // https://github.com/octokit/request.js/blob/v8.1.1/src/fetch-wrapper.ts
  // Retain the action's configured (including proxy-aware) API fetch transport.
  const apiFetch = github.request?.endpoint?.DEFAULTS?.request?.fetch;
  if (typeof apiFetch !== 'function') throw new Error('Pinned action API fetch transport is unavailable');
  let apiSummary, response;
  try {
    response = await github.request('GET /repos/{owner}/{repo}/actions/artifacts/{artifact_id}/{archive_format}', {
      ...repo, artifact_id: artifact.id, archive_format: 'zip',
      request: { parseSuccessResponseBody: false, retries: 0, signal: AbortSignal.timeout(90000),
        fetch: async (url, options) => {
          if (url !== apiUrl || options?.method !== 'GET' || options.body != null) throw new Error('Unexpected artifact API request');
          const fetched = await apiFetch(url, { ...options, redirect: 'manual', credentials: 'omit' });
          apiSummary = transportSummary(fetched);
          core?.info(`Diagnostic artifact API response: ${JSON.stringify(apiSummary)}`);
          // No redirect/error body is needed. Cancel it before Octokit can parse
          // a response or include body content in an error object.
          if (fetched.body) await fetched.body.cancel();
          if (fetched.status !== 302 || fetched.redirected !== false || fetched.url !== apiUrl) throw transportError('API response', apiSummary);
          return fetched;
        } },
    });
  } catch (error) {
    // Never log Request/Response objects, authorization, signed URLs or bodies.
    throw transportError('API request', apiSummary || transportSummary(error?.response || error));
  }
  if (response.status !== 302 || response.url !== apiUrl || typeof response.headers?.location !== 'string') {
    throw transportError('API redirect shape', apiSummary || transportSummary(response));
  }
  const location = artifactLocation(response.headers.location);
  // No GitHub token is forwarded to the signed storage URL, or into logs/receipts.
  let fetched, bytes;
  try {
    fetched = await fetchImpl(location, { redirect: 'error', signal: AbortSignal.timeout(90000), credentials: 'omit' });
    core?.info(`Diagnostic artifact storage response: ${JSON.stringify(transportSummary(fetched))}`);
    if (fetched.status !== 200 || fetched.redirected !== false || fetched.url !== location.href || !fetched.body) {
      if (fetched.body) await fetched.body.cancel();
      throw transportError('storage response', transportSummary(fetched));
    }
    bytes = await readPinnedArchive(fetched, artifact);
  } catch {
    if (fetched?.body && !fetched.body.locked) await fetched.body.cancel().catch(() => {});
    throw transportError('storage verification', transportSummary(fetched));
  }
  fs.mkdirSync(path.dirname(destination), { recursive: true });
  fs.writeFileSync(destination, bytes, { flag: 'wx', mode: 0o600 });
  return artifact;
}
async function downloadExactArchive(options) {
  // The old diagnostic route remains pinned to its one reviewed failed proof.
  return downloadPinnedArchive({ ...options, artifact: ARTIFACT });
}
async function retrieveReplayArtifact({ github, context, core, destination, receiptPath, fetchImpl = fetch }) {
  const approved = await prepareReplayRequest({ github, context });
  const repo = context.repo;
  const original = await validateProofCommit({ github, repo, sha: PRODUCER.source_sha });
  if (original.request.diagnostic_artifact !== DIAGNOSTIC_ARTIFACT) throw new Error('Original proof did not opt into diagnostic retention');
  const { data: run } = await github.rest.actions.getWorkflowRunAttempt({ ...repo, run_id: PRODUCER.run_id, attempt_number: PRODUCER.run_attempt });
  const { data: jobs } = await github.rest.actions.listJobsForWorkflowRunAttempt({ ...repo, run_id: PRODUCER.run_id, attempt_number: PRODUCER.run_attempt, per_page: 100 });
  const cpu = validateProducer(run, jobs);
  const { data: before } = await github.rest.actions.getArtifact({ ...repo, artifact_id: ARTIFACT.id });
  validateArtifact(before, cpu);
  if (fs.existsSync(receiptPath)) throw new Error('Replay retrieval receipt must be fresh');
  await downloadExactArchive({ github, repo, destination, fetchImpl, core });
  const { data: after } = await github.rest.actions.getArtifact({ ...repo, artifact_id: ARTIFACT.id });
  validateArtifact(after, cpu);
  if (!equal(before, after)) throw new Error('Diagnostic artifact changed during retrieval');
  const receipt = { schema_version: 1, kind: 'ct2-cpu-diagnostic-retrieval', publication_authorized: false,
    installable: false, original_build: PRODUCER, original_build_conclusion: 'failure',
    verifier_source_sha: approved.verifier_source_sha, verifier_base_sha: approved.base_sha,
    artifact: ARTIFACT, files: FILES, expires_at: after.expires_at };
  fs.mkdirSync(path.dirname(receiptPath), { recursive: true });
  fs.writeFileSync(receiptPath, JSON.stringify(receipt, null, 2) + '\n', { flag: 'wx' });
  core?.info('Verified the exact one-day failed-build diagnostic archive; extraction/provenance gates still precede runtime execution.');
  return receipt;
}
module.exports = { REPOSITORY, REPOSITORY_ID, BRANCH, REQUEST_PATH, PURPOSE, PRODUCER, ARTIFACT, ARTIFACT_NAME,
  FILES, MANIFEST, parseRequest, assertReplayTree, prepareReplayRequest, validateProducer, validateArtifact,
  artifactLocation, readPinnedArchive, transportSummary, downloadPinnedArchive, downloadExactArchive, retrieveReplayArtifact, hash, equal };
