'use strict';

const REQUEST_PATH = '.github/release-request.json';
const TAG = /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*)?$/;
const SHA = /^[a-f0-9]{40}$/;

function parseReleaseRequest(content, commit) {
  const request = JSON.parse(content);
  if (!request || Array.isArray(request)
      || Object.keys(request).sort().join(',') !== 'schema_version,source_sha,tag'
      || request.schema_version !== 1 || !TAG.test(request.tag || '')
      || !SHA.test(request.source_sha || '')) {
    throw new Error('Invalid release request: expected schema_version: 1, vX.Y.Z tag and full source_sha');
  }
  if (commit.parents?.length !== 1 || commit.parents[0].sha !== request.source_sha) {
    throw new Error('source_sha must be the release request commit’s only parent');
  }
  return request;
}

async function readContent(github, repo, path, ref) {
  const { data } = await github.rest.repos.getContent({ ...repo, path, ref });
  if (data.type !== 'file' || data.encoding !== 'base64') throw new Error(`Cannot read ${path} at ${ref}`);
  return Buffer.from(data.content, 'base64').toString('utf8');
}

async function tree(github, repo, sha) {
  const { data } = await github.rest.git.getTree({ ...repo, tree_sha: sha });
  if (data.truncated || !Array.isArray(data.tree)) throw new Error('Cannot verify a truncated Git tree');
  return data.tree;
}

function sameEntries(before, after, except) {
  const normalize = (entries) => entries.filter((entry) => entry.path !== except)
    .map(({ path, mode, type, sha }) => ({ path, mode, type, sha }))
    .sort((a, b) => a.path.localeCompare(b.path));
  return JSON.stringify(normalize(before)) === JSON.stringify(normalize(after));
}

async function assertRequestOnlyTree(github, repo, sourceSha, requestSha) {
  const [before, after] = await Promise.all([tree(github, repo, sourceSha), tree(github, repo, requestSha)]);
  const beforeGithub = before.find((entry) => entry.path === '.github');
  const afterGithub = after.find((entry) => entry.path === '.github');
  if (!sameEntries(before, after, '.github') || beforeGithub?.type !== 'tree'
      || afterGithub?.type !== 'tree' || beforeGithub.mode !== afterGithub.mode) {
    throw new Error('Release request commit must change only .github/release-request.json');
  }
  const [beforeFiles, afterFiles] = await Promise.all([
    tree(github, repo, beforeGithub.sha), tree(github, repo, afterGithub.sha),
  ]);
  const beforeRequest = beforeFiles.find((entry) => entry.path === 'release-request.json');
  const afterRequest = afterFiles.find((entry) => entry.path === 'release-request.json');
  if (!sameEntries(beforeFiles, afterFiles, 'release-request.json')
      || afterRequest?.type !== 'blob' || afterRequest.mode !== '100644'
      || beforeRequest?.sha === afterRequest.sha) {
    throw new Error('Release request commit must change only its regular request file');
  }
}

function validateVersions(tag, packageJson, configJson, cargoToml, cargoLock) {
  if (!TAG.test(tag)) throw new Error('Release tag must be vX.Y.Z, optionally with a prerelease suffix');
  const version = tag.slice(1);
  const pkg = JSON.parse(packageJson);
  const config = JSON.parse(configJson);
  const cargoPackage = cargoToml.match(/^\[package\]\s*\r?\n([\s\S]*?)(?=^\[|$(?![\s\S]))/m)?.[1];
  const cargoVersion = cargoPackage?.match(/^version\s*=\s*"([^"]+)"\s*$/m)?.[1];
  const lockedPackage = cargoLock.split(/^\[\[package\]\]\s*$/m)
    .find((section) => /^name\s*=\s*"luma-subtitle"\s*$/m.test(section));
  const lockedVersion = lockedPackage?.match(/^version\s*=\s*"([^"]+)"\s*$/m)?.[1];
  if ([pkg.version, config.version, cargoVersion, lockedVersion].some((value) => value !== version)) {
    throw new Error(`Tag ${tag} must match package.json, tauri.conf.json, Cargo.toml and Cargo.lock versions`);
  }
  if (config.bundle?.createUpdaterArtifacts !== true || !config.plugins?.updater?.pubkey) {
    throw new Error('Release source must enable signed Tauri v2 updater artifacts');
  }
  return version;
}

async function tagCommit(github, repo, tag) {
  let object;
  try {
    object = (await github.rest.git.getRef({ ...repo, ref: `tags/${tag}` })).data.object;
  } catch (error) {
    if (error.status === 404) return null;
    throw error;
  }
  for (let depth = 0; object.type === 'tag' && depth < 8; depth++) {
    object = (await github.rest.git.getTag({ ...repo, tag_sha: object.sha })).data.object;
  }
  if (object.type !== 'commit' || !SHA.test(object.sha)) throw new Error('Release tag must resolve to a commit');
  return object.sha;
}

function sourceMarker(sourceSha) {
  return `<!-- luma-release-source:${sourceSha} -->`;
}

function assertDraft(release, tag, sourceSha) {
  if (!release.draft) throw new Error('Refusing to modify an already-published release');
  if (release.tag_name !== tag || !release.body?.includes(sourceMarker(sourceSha))) {
    throw new Error('Draft release source does not match the pinned source');
  }
}

function releaseNotes(changelog, version) {
  const lines = changelog.split(/\r?\n/);
  const heading = `## ${version}`;
  const start = lines.findIndex((line) => line === heading || line.startsWith(`${heading} `));
  if (start < 0) throw new Error(`CHANGELOG.md has no release notes for ${version}`);
  let end = start + 1;
  while (end < lines.length && !lines[end].startsWith('## ')) end++;
  const notes = lines.slice(start + 1, end).join('\n').trim();
  if (!notes) throw new Error(`CHANGELOG.md has empty release notes for ${version}`);
  return notes;
}

async function prepareRelease({ github, context, core }) {
  const repo = context.repo;
  let tag;
  let sourceSha;
  if (context.eventName === 'push' && context.ref === 'refs/heads/main') {
    if (context.payload.deleted) throw new Error('Deleted refs cannot request a release');
    const { data: commit } = await github.rest.repos.getCommit({ ...repo, ref: context.sha });
    const request = parseReleaseRequest(await readContent(github, repo, REQUEST_PATH, context.sha), commit);
    tag = request.tag;
    sourceSha = request.source_sha;
    await assertRequestOnlyTree(github, repo, sourceSha, context.sha);
    const { data: main } = await github.rest.git.getRef({ ...repo, ref: 'heads/main' });
    if (main.object.sha !== context.sha) throw new Error('main advanced; refusing a stale release request');
  } else if (context.eventName === 'push' && context.ref.startsWith('refs/tags/')) {
    if (context.payload.deleted) throw new Error('Deleted tags cannot request a release');
    tag = context.ref.slice('refs/tags/'.length);
    if (!TAG.test(tag)) throw new Error('Invalid release tag');
    sourceSha = await tagCommit(github, repo, tag);
    // A moved tag must not silently select a source newer than this push event.
    if (sourceSha !== context.sha) throw new Error('Tag no longer matches the push commit');
  } else if (context.eventName === 'workflow_dispatch') {
    tag = context.payload.inputs?.tag_name;
    if (!TAG.test(tag || '')) throw new Error('Manual releases require an existing vX.Y.Z tag');
    sourceSha = await tagCommit(github, repo, tag);
    if (!sourceSha) throw new Error('Manual releases require an existing tag');
  } else {
    throw new Error('Unsupported release event');
  }
  if (!SHA.test(sourceSha || '')) throw new Error('Invalid release source SHA');
  const files = await Promise.all(['package.json', 'src-tauri/tauri.conf.json', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock']
    .map((path) => readContent(github, repo, path, sourceSha)));
  const version = validateVersions(tag, ...files);
  const existingTag = await tagCommit(github, repo, tag);
  if (existingTag && existingTag !== sourceSha) throw new Error('Existing release tag points to different source; it will not be overwritten');
  if (!existingTag) await github.rest.git.createRef({ ...repo, ref: `refs/tags/${tag}`, sha: sourceSha });

  let release;
  try {
    release = (await github.rest.repos.getReleaseByTag({ ...repo, tag })).data;
  } catch (error) {
    if (error.status !== 404) throw error;
  }
  const prerelease = tag.includes('-') || context.payload.inputs?.prerelease === 'true';
  if (release) {
    assertDraft(release, tag, sourceSha);
    if (release.prerelease !== prerelease) throw new Error('Existing draft has different prerelease status');
  } else {
    const notes = releaseNotes(await readContent(github, repo, 'CHANGELOG.md', sourceSha), version);
    release = (await github.rest.repos.createRelease({
      ...repo, tag_name: tag, target_commitish: sourceSha,
      name: context.payload.inputs?.release_name || `Luma Subtitle ${tag}`,
      body: `${notes}\n\n${sourceMarker(sourceSha)}`,
      draft: true, prerelease,
    })).data;
  }
  const outputs = { source_sha: sourceSha, tag, version, release_id: String(release.id), prerelease: String(prerelease) };
  for (const [name, value] of Object.entries(outputs)) core.setOutput(name, value);
  return outputs;
}

module.exports = { REQUEST_PATH, TAG, SHA, parseReleaseRequest, assertRequestOnlyTree, validateVersions, tagCommit, sourceMarker, assertDraft, releaseNotes, prepareRelease };
