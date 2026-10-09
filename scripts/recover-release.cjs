'use strict';
const { TAG, SHA, parseReleaseRequest, assertRequestOnlyTree, validateVersions, tagCommit, sourceMarker } = require('./prepare-release.cjs');
const { requiredAssets, publishRelease } = require('./publish-release.cjs');
const REQUEST_PATH = '.github/release-recovery.json';
const integer = (value) => Number.isSafeInteger(value) && value > 0;

function parseRecoveryRequest(text, commit) {
  const value = JSON.parse(text);
  if (!value || Array.isArray(value)
      || Object.keys(value).sort().join(',') !== 'assets,build_run_attempt,build_run_id,controller_sha,release_id,schema_version,source_sha,tag'
      || value.schema_version !== 1 || !TAG.test(value.tag || '')
      || !SHA.test(value.source_sha || '') || !SHA.test(value.controller_sha || '')
      || !integer(value.release_id) || !integer(value.build_run_id) || !integer(value.build_run_attempt)) {
    throw new Error('Invalid release recovery request');
  }
  if (commit.parents?.length !== 1 || commit.parents[0].sha !== value.controller_sha) {
    throw new Error('Recovery controller must be the request commit’s only parent');
  }
  const names = requiredAssets(value.tag.slice(1));
  if (!Array.isArray(value.assets) || value.assets.length !== names.length
      || new Set(value.assets.map((asset) => asset?.id)).size !== names.length
      || [...value.assets].map((asset) => asset?.name).sort().join(',') !== names.sort().join(',')) {
    throw new Error('Recovery must pin exactly the seven original asset identities');
  }
  for (const asset of value.assets) {
    if (!asset || Object.keys(asset).sort().join(',') !== 'digest,id,name,size'
        || !integer(asset.id) || !integer(asset.size) || !/^sha256:[a-f0-9]{64}$/.test(asset.digest || '')) {
      throw new Error('Recovery requires exact asset IDs, sizes and SHA-256 digests');
    }
  }
  return value;
}

async function readFile(github, repo, file, ref) {
  const { data } = await github.rest.repos.getContent({ ...repo, path: file, ref });
  if (data.type !== 'file' || data.encoding !== 'base64') throw new Error(`Invalid source file ${file}`);
  return Buffer.from(data.content, 'base64').toString('utf8');
}

// These are public read-only API resources. No token or extra Actions permission
// is sent, and redirects/other hosts are never followed.
async function publicApi(repo, suffix, fetchImpl = fetch) {
  if (!/^[A-Za-z0-9_.-]+$/.test(repo.owner) || !/^[A-Za-z0-9_.-]+$/.test(repo.repo)
      || !/^actions\/runs\/\d+(?:\/attempts\/\d+\/jobs\?per_page=100)?$/.test(suffix)) {
    throw new Error('Invalid public provenance API path');
  }
  const url = `https://api.github.com/repos/${repo.owner}/${repo.repo}/${suffix}`;
  const response = await fetchImpl(url, {
    redirect: 'error', signal: AbortSignal.timeout(15000),
    headers: { accept: 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28' },
  });
  if (!response.ok) throw new Error(`Cannot verify public build provenance: HTTP ${response.status}`);
  return response.json();
}

function validateBuildRun(run, jobs, repo, request) {
  const repository = `${repo.owner}/${repo.repo}`.toLowerCase();
  if (run.id !== request.build_run_id || run.run_attempt !== request.build_run_attempt
      || run.repository?.full_name?.toLowerCase() !== repository
      || run.head_repository?.full_name?.toLowerCase() !== repository
      || run.event !== 'push' || run.head_branch !== 'main'
      || run.path !== '.github/workflows/release.yml' || run.status !== 'completed'
      || !SHA.test(run.head_sha || '')) {
    throw new Error('Original build run repository, attempt, workflow or status mismatch');
  }
  if (!Array.isArray(jobs.jobs) || jobs.total_count !== jobs.jobs.length || jobs.total_count > 100) {
    throw new Error('Incomplete original build jobs response');
  }
  for (const name of ['Validate pinned source and prepare draft', 'Build Windows x64', 'Build macOS Apple Silicon']) {
    const matches = jobs.jobs.filter((job) => job.name === name);
    if (matches.length !== 1 || matches[0].run_id !== run.id || matches[0].head_sha !== run.head_sha
        || matches[0].status !== 'completed' || matches[0].conclusion !== 'success') {
      throw new Error(`Original required build job was not successful: ${name}`);
    }
  }
}

async function recoverRelease({ github, context, core, fetchImpl = fetch, publish = publishRelease }) {
  if (context.eventName !== 'push' || context.ref !== 'refs/heads/main' || context.payload.deleted) {
    throw new Error('Recovery requires a request-only main push');
  }
  const repo = context.repo;
  const { data: commit } = await github.rest.repos.getCommit({ ...repo, ref: context.sha });
  const request = parseRecoveryRequest(await readFile(github, repo, REQUEST_PATH, context.sha), commit);
  await assertRequestOnlyTree(github, repo, request.controller_sha, context.sha, 'release-recovery.json');
  const { data: main } = await github.rest.git.getRef({ ...repo, ref: 'heads/main' });
  if (main.object.sha !== context.sha) throw new Error('main advanced; refusing stale recovery request');
  if (await tagCommit(github, repo, request.tag) !== request.source_sha) throw new Error('Original immutable tag/source mismatch');
  const run = await publicApi(repo, `actions/runs/${request.build_run_id}`, fetchImpl);
  const jobs = await publicApi(repo, `actions/runs/${request.build_run_id}/attempts/${request.build_run_attempt}/jobs?per_page=100`, fetchImpl);
  validateBuildRun(run, jobs, repo, request);
  const { data: buildCommit } = await github.rest.repos.getCommit({ ...repo, ref: run.head_sha });
  const original = parseReleaseRequest(await readFile(github, repo, '.github/release-request.json', run.head_sha), buildCommit);
  if (original.tag !== request.tag || original.source_sha !== request.source_sha) throw new Error('Original build request source mismatch');
  await assertRequestOnlyTree(github, repo, request.source_sha, run.head_sha);
  const files = await Promise.all(['package.json', 'src-tauri/tauri.conf.json', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock']
    .map((file) => readFile(github, repo, file, request.source_sha)));
  validateVersions(request.tag, ...files);
  const { data: release } = await github.rest.repos.getRelease({ ...repo, release_id: request.release_id });
  if (release.id !== request.release_id || release.tag_name !== request.tag
      || release.prerelease !== request.tag.includes('-')
      || !release.body?.includes(sourceMarker(request.source_sha))) {
    throw new Error('Recovery draft identity or source marker mismatch');
  }
  // The publisher compares this pinned snapshot again during its own checks.
  // Already-published exact matches are revalidated read-only; never overwritten.
  return publish({ github, context, core, sourceSha: request.source_sha, tag: request.tag,
    releaseId: String(request.release_id), config: JSON.parse(files[1]), expectedAssets: request.assets });
}

module.exports = { REQUEST_PATH, parseRecoveryRequest, publicApi, validateBuildRun, recoverRelease };
