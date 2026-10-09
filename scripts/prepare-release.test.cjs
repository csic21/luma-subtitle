'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { parseReleaseRequest, validateVersions, assertRequestOnlyTree, prepareRelease, sourceMarker, releaseNotes } = require('./prepare-release.cjs');

const source = 'a'.repeat(40);
const head = 'b'.repeat(40);
const tag = 'v1.2.3';
const request = { schema_version: 1, tag, source_sha: source };
const versions = [
  JSON.stringify({ version: '1.2.3' }),
  JSON.stringify({ version: '1.2.3', bundle: { createUpdaterArtifacts: true }, plugins: { updater: { pubkey: 'test-public-key' } } }),
  '[package]\nname = "luma-subtitle"\nversion = "1.2.3"\n\n[dependencies]\nfoo = "1"\n',
  'version = 3\n[[package]]\nname = "other"\nversion = "0.0.1"\n[[package]]\nname = "luma-subtitle"\nversion = "1.2.3"\n',
];
const entry = (path, sha, type = 'blob') => ({ path, sha, type, mode: type === 'tree' ? '040000' : '100644' });
function fixture() {
  const mutations = [];
  const outputs = {};
  const state = {
    request, parents: [{ sha: source }], main: head, existingTag: null,
    trees: {
      [source]: [entry('.github', 'before', 'tree'), entry('package.json', 'unchanged')],
      [head]: [entry('.github', 'after', 'tree'), entry('package.json', 'unchanged')],
      before: [entry('workflows', 'workflows', 'tree')],
      after: [entry('workflows', 'workflows', 'tree'), entry('release-request.json', 'request')],
    },
    versions: [...versions], release: null,
  };
  const missing = () => { throw Object.assign(new Error('Not Found'), { status: 404 }); };
  const github = { rest: {
    git: {
      getTree: async ({ tree_sha }) => ({ data: { tree: state.trees[tree_sha], truncated: state.truncated || false } }),
      getRef: async ({ ref }) => ref === 'heads/main'
        ? { data: { object: { type: 'commit', sha: state.main } } }
        : state.existingTag ? { data: { object: state.existingTag } } : missing(),
      getTag: async () => ({ data: { object: { type: 'commit', sha: source } } }),
      createRef: async (args) => { mutations.push(['tag', args]); state.existingTag = { type: 'commit', sha: args.sha }; },
    },
    repos: {
      getCommit: async () => ({ data: { parents: state.parents } }),
      getContent: async ({ path, ref }) => {
        if (path === 'CHANGELOG.md') {
          assert.equal(ref, source);
          return { data: { type: 'file', encoding: 'base64', content: Buffer.from('# Changelog\n\n## 1.2.3 — 2026-10-09\n\n### Improvements\n- Release improvements.\n\n## 1.2.2\nOld notes').toString('base64') } };
        }
        const index = ['package.json', 'src-tauri/tauri.conf.json', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock'].indexOf(path);
        if (index !== -1) assert.equal(ref, source, 'versions must be read from the pinned source');
        return { data: { type: 'file', encoding: 'base64', content: Buffer.from(index === -1 ? JSON.stringify(state.request) : state.versions[index]).toString('base64') } };
      },
      getReleaseByTag: async () => state.release ? { data: state.release } : missing(),
      createRelease: async (args) => { mutations.push(['draft', args]); return { data: { ...args, id: 42 } }; },
    },
  } };
  const context = { eventName: 'push', ref: 'refs/heads/main', sha: head, payload: {}, repo: { owner: 'owner', repo: 'repo' } };
  const core = { setOutput: (name, value) => { outputs[name] = value; } };
  return { state, mutations, outputs, github, context, core };
}

test('valid source-pinned request creates the matching tag and only a draft', async () => {
  const f = fixture();
  await prepareRelease(f);
  assert.equal(f.outputs.source_sha, source);
  assert.equal(f.outputs.release_id, '42');
  assert.equal(f.mutations[0][1].sha, source);
  assert.equal(f.mutations[1][1].draft, true);
  assert.equal(f.mutations[1][1].target_commitish, source);
});

test('rejects malformed, incomplete and expanded request schemas', () => {
  for (const value of [null, [], {}, { ...request, schema_version: 2 }, { ...request, extra: true },
    { ...request, source_sha: 'abc' }, { ...request, tag: 'v01.2.3' }, { ...request, tag: 'v1.2.3\nother' }]) {
    assert.throws(() => parseReleaseRequest(JSON.stringify(value), { parents: [{ sha: source }] }));
  }
  assert.throws(() => parseReleaseRequest('{bad', { parents: [{ sha: source }] }));
});

test('rejects wrong source parent, merge commit and missing parent', () => {
  for (const parents of [[], [{ sha: head }], [{ sha: source }, { sha: head }]]) {
    assert.throws(() => parseReleaseRequest(JSON.stringify(request), { parents }), /only parent/);
  }
});

test('rejects source version mismatch independently in all four files', () => {
  for (let i = 0; i < versions.length; i++) {
    const files = [...versions];
    files[i] = files[i].replace('1.2.3', '1.2.4');
    assert.throws(() => validateVersions(tag, ...files), /must match/);
  }
});

test('rejects missing updater artifacts or public key', () => {
  for (const config of [{ version: '1.2.3' }, { version: '1.2.3', bundle: { createUpdaterArtifacts: true } }]) {
    const files = [...versions]; files[1] = JSON.stringify(config);
    assert.throws(() => validateVersions(tag, ...files), /signed Tauri/);
  }
});

test('request tree comparison rejects other root changes', async () => {
  const f = fixture(); f.state.trees[head][1].sha = 'changed';
  await assert.rejects(prepareRelease(f), /change only/);
  assert.deepEqual(f.mutations, []);
});

test('request tree comparison rejects workflow changes', async () => {
  const f = fixture(); f.state.trees.after[0].sha = 'changed-workflow';
  await assert.rejects(prepareRelease(f), /change only/);
  assert.deepEqual(f.mutations, []);
});

test('request file must be changed and a regular file, not a symlink', async () => {
  for (const mode of ['120000', '100755']) {
    const f = fixture(); f.state.trees.after[1].mode = mode;
    await assert.rejects(prepareRelease(f), /regular request file/);
  }
  const f = fixture(); f.state.trees.before.push(f.state.trees.after[1]);
  await assert.rejects(prepareRelease(f), /regular request file/);
});

test('rejects truncated Git tree instead of trusting partial evidence', async () => {
  const f = fixture(); f.state.truncated = true;
  await assert.rejects(assertRequestOnlyTree(f.github, f.context.repo, source, head), /truncated/);
});

test('stale main request cannot mutate tags or releases', async () => {
  const f = fixture(); f.state.main = source;
  await assert.rejects(prepareRelease(f), /main advanced/);
  assert.deepEqual(f.mutations, []);
});

test('an existing tag at another source is never retargeted', async () => {
  const f = fixture(); f.state.existingTag = { type: 'commit', sha: head };
  await assert.rejects(prepareRelease(f), /different source/);
  assert.deepEqual(f.mutations, []);
});

test('a matching existing annotated tag is accepted without recreating it', async () => {
  const f = fixture(); f.state.existingTag = { type: 'tag', sha: head };
  await prepareRelease(f);
  assert.deepEqual(f.mutations.map(([name]) => name), ['draft']);
});

test('published releases cannot be overwritten', async () => {
  const f = fixture(); f.state.existingTag = { type: 'commit', sha: source };
  f.state.release = { id: 42, tag_name: tag, draft: false, body: sourceMarker(source) };
  await assert.rejects(prepareRelease(f), /already-published/);
  assert.deepEqual(f.mutations, []);
});

test('a matching failed draft is reused, while mismatched draft source is rejected', async () => {
  const f = fixture(); f.state.existingTag = { type: 'commit', sha: source };
  f.state.release = { id: 42, tag_name: tag, draft: true, prerelease: false, body: sourceMarker(source) };
  await prepareRelease(f);
  assert.deepEqual(f.mutations, []);
  f.state.release.body = sourceMarker(head);
  await assert.rejects(prepareRelease(f), /source does not match/);
});

test('ordinary tag push builds that tag and rejects a moved or deleted tag', async () => {
  const f = fixture(); f.state.existingTag = { type: 'commit', sha: source };
  Object.assign(f.context, { ref: `refs/tags/${tag}`, sha: source });
  await prepareRelease(f);
  assert.equal(f.outputs.source_sha, source);
  f.context.sha = head;
  await assert.rejects(prepareRelease(f), /no longer matches/);
  f.context.payload.deleted = true;
  await assert.rejects(prepareRelease(f), /Deleted tags/);
});

test('manual release requires and resolves an existing version tag', async () => {
  const f = fixture(); f.context.eventName = 'workflow_dispatch'; f.context.payload.inputs = { tag_name: tag };
  await assert.rejects(prepareRelease(f), /existing tag/);
  f.state.existingTag = { type: 'commit', sha: source };
  await prepareRelease(f);
  assert.equal(f.outputs.source_sha, source);
});

test('release notes include only the requested version and reject missing or empty sections', () => {
  assert.equal(releaseNotes('# Changelog\n## 1.2.3 — 2026-10-09\nNew notes\n## 1.2.2\nOld notes', '1.2.3'), 'New notes');
  assert.throws(() => releaseNotes('## 1.2.30\nWrong version', '1.2.3'), /no release notes/);
  assert.throws(() => releaseNotes('## 1.2.3\n\n## 1.2.2\nOld notes', '1.2.3'), /empty release notes/);
});
