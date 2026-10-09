'use strict';

// Read-only gate for one explicitly requested native proof. The tested source
// is the request commit itself: run, job, checkout and report SHAs stay identical.
const { SHA } = require('../../prepare-release.cjs');
const REPOSITORY = 'csic21/luma-subtitle';
const BRANCH = 'feat/optional-asr-engines';
const REQUEST_PATH = '.github/requests/ct2-cpu-proof.json';
const PURPOSE = 'native-cpu-wheel-proof';
const PROOF_JOB = 'Validate request-only CPU proof';
const MAX_REQUEST = 4096;
const keys = (o, expected) => o && typeof o === 'object' && !Array.isArray(o)
  && Object.keys(o).sort().join(',') === expected.split(',').sort().join(',');
function assertRepository(repo) {
  if (`${repo.owner}/${repo.repo}` !== REPOSITORY) throw new Error('CPU proof requires the exact intended repository');
}
function parseProofRequest(text, commit) {
  if (Buffer.byteLength(text) >= MAX_REQUEST) throw new Error('CPU proof request exceeds bounds');
  const request = JSON.parse(text);
  if (!keys(request, 'schema_version,source_sha,purpose') || request.schema_version !== 1
      || request.purpose !== PURPOSE || !SHA.test(request.source_sha || '')
      || commit.parents?.length !== 1 || commit.parents[0].sha !== request.source_sha) {
    throw new Error('CPU proof request must pin its exact sole parent and native-proof purpose');
  }
  return request;
}
async function tree(github, repo, sha) {
  if (!sha) return [];
  const { data } = await github.rest.git.getTree({ ...repo, tree_sha: sha });
  if (data.truncated || !Array.isArray(data.tree) || data.tree.length > 10000
      || new Set(data.tree.map(e => e.path)).size !== data.tree.length
      || data.tree.some(e => typeof e.path !== 'string' || !e.path || e.path.includes('/')
        || e.path === '.' || e.path === '..' || !SHA.test(e.sha || ''))) {
    throw new Error('Cannot verify incomplete or invalid CPU proof tree');
  }
  return data.tree;
}
function siblings(entries, excluded) {
  return entries.filter(e => e.path !== excluded).map(({ path, mode, type, sha }) => ({ path, mode, type, sha }))
    .sort((a, b) => a.path.localeCompare(b.path));
}
async function assertProofRequestOnlyTree(github, repo, parentSha, proofSha) {
  const parts = REQUEST_PATH.split('/'); let beforeSha = parentSha, afterSha = proofSha;
  for (const [index, name] of parts.entries()) {
    const [before, after] = await Promise.all([tree(github, repo, beforeSha), tree(github, repo, afterSha)]);
    if (JSON.stringify(siblings(before, name)) !== JSON.stringify(siblings(after, name))) {
      throw new Error('CPU proof commit must change only its inert request file');
    }
    const a = before.find(e => e.path === name), b = after.find(e => e.path === name);
    if (index === parts.length - 1) {
      if (b?.type !== 'blob' || b.mode !== '100644' || (a && (a.type !== 'blob' || a.mode !== '100644'))
          || a?.sha === b.sha) throw new Error('CPU proof request must be a changed regular 100644 blob');
      return b.sha;
    }
    if (b?.type !== 'tree' || b.mode !== '040000' || (a && (a.type !== 'tree' || a.mode !== '040000'))) {
      throw new Error('CPU proof request ancestors must be ordinary directories');
    }
    beforeSha = a?.sha; afterSha = b.sha;
  }
  throw new Error('Missing CPU proof request path');
}
async function validateProofCommit({ github, repo, sha }) {
  assertRepository(repo);
  if (!SHA.test(sha || '')) throw new Error('Exact immutable proof commit SHA required');
  const { data: commit } = await github.rest.repos.getCommit({ ...repo, ref: sha });
  if (commit.sha !== sha) throw new Error('Resolved proof commit differs from requested SHA');
  const { data } = await github.rest.repos.getContent({ ...repo, path: REQUEST_PATH, ref: sha });
  if (data.type !== 'file' || data.encoding !== 'base64') throw new Error('CPU proof request is not a regular file');
  const bytes = Buffer.from(data.content, 'base64');
  const request = parseProofRequest(bytes.toString('utf8'), commit);
  const blobSha = await assertProofRequestOnlyTree(github, repo, request.source_sha, sha);
  if (data.sha !== blobSha) throw new Error('CPU proof content does not match its exact Git blob');
  return { source_sha: sha, base_sha: request.source_sha, request };
}
async function prepareCpuProof({ github, context, core }) {
  assertRepository(context.repo);
  if (context.eventName !== 'push' || context.ref !== `refs/heads/${BRANCH}` || context.payload.deleted
      || context.payload.repository?.full_name !== REPOSITORY || !SHA.test(context.sha || '')) {
    throw new Error('Native proof requires a request-only push on the intended feature branch');
  }
  const validated = await validateProofCommit({ github, repo: context.repo, sha: context.sha });
  if (context.payload.before !== validated.base_sha || context.payload.after !== context.sha
      || context.payload.forced !== false) throw new Error('Native proof requires one non-forced request-only push from its exact parent');
  const { data: branch } = await github.rest.git.getRef({ ...context.repo, ref: `heads/${BRANCH}` });
  if (branch.object.type !== 'commit' || branch.object.sha !== context.sha) throw new Error('Feature branch advanced; stale native proof request');
  core?.setOutput('source_sha', validated.source_sha);
  core?.setOutput('base_sha', validated.base_sha);
  return validated;
}
module.exports = { REPOSITORY, BRANCH, REQUEST_PATH, PURPOSE, PROOF_JOB, parseProofRequest,
  assertProofRequestOnlyTree, validateProofCommit, prepareCpuProof };
