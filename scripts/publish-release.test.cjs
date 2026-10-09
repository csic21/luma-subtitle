'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { sourceMarker } = require('./prepare-release.cjs');
const { requiredAssets, assetUrl, decodeMinisign, verifySignature, downloadAsset, compareStableVersions, publishRelease } = require('./publish-release.cjs');

const source = 'a'.repeat(40);
const tag = 'v1.2.3';
const version = '1.2.3';
const encodedSignature = Buffer.from('untrusted comment: test signature\ntest\ntrusted comment: timestamp:123\ntest\n').toString('base64');
function fixture() {
  const repo = { owner: 'owner', repo: 'repo' };
  const mutations = [];
  const verified = [];
  const contents = new Map();
  const state = {
    release: { id: 42, tag_name: tag, draft: true, prerelease: false, body: `Release notes\n\n${sourceMarker(source)}`, html_url: 'https://github.com/owner/repo/releases/tag/v1.2.3' },
    assets: [], tagSha: source, latest: 'v1.2.2',
  };
  requiredAssets(version).forEach((name, index) => {
    const data = Buffer.from(name.endsWith('.sig') ? encodedSignature : `test artifact: ${name}`);
    contents.set(index + 1, data);
    state.assets.push({ id: index + 1, name, state: 'uploaded', size: data.length,
      digest: `sha256:${createHash('sha256').update(data).digest('hex')}`, browser_download_url: assetUrl(repo, tag, name) });
  });
  const github = {
    paginate: async () => state.assets,
    rest: {
      git: { getRef: async () => ({ data: { object: { type: 'commit', sha: state.tagSha } } }) },
      repos: {
        getRelease: async () => ({ data: state.release }),
        getLatestRelease: async () => ({ data: { tag_name: state.latest } }),
        listReleaseAssets: () => {},
        deleteReleaseAsset: async ({ asset_id }) => { mutations.push('delete'); state.assets = state.assets.filter((a) => a.id !== asset_id); },
        uploadReleaseAsset: async ({ data, name }) => {
          mutations.push('manifest');
          const asset = { id: 100, name, state: 'uploaded', size: data.length, browser_download_url: assetUrl(repo, tag, name) };
          state.assets.push(asset); contents.set(100, data); return { data: asset };
        },
        updateRelease: async (args) => {
          mutations.push('publish'); assert.equal(args.draft, false);
          state.release = { ...state.release, draft: false }; return { data: state.release };
        },
      },
    },
  };
  const args = { github, context: { repo }, core: { setOutput() {}, info() {} }, sourceSha: source, tag, releaseId: '42',
    config: { version, bundle: { createUpdaterArtifacts: true }, plugins: { updater: { pubkey: 'test-public-key' } } },
    download: async (_github, _repo, asset, destination) => fs.writeFileSync(destination, contents.get(asset.id)),
    verify: (value) => { verified.push(value); }, now: () => new Date('2026-10-09T00:00:00Z') };
  return { args, state, mutations, verified, contents };
}

test('both platforms and three verified updater signatures precede one manifest and one publication', async () => {
  const f = fixture(); const result = await publishRelease(f.args);
  assert.equal(f.verified.length, 3);
  assert.deepEqual(f.mutations, ['manifest', 'publish']);
  assert.deepEqual(Object.keys(result.platforms).sort(), ['darwin-aarch64', 'windows-x86_64']);
  assert.equal(result.platforms['windows-x86_64'].signature, encodedSignature);
  assert.match(result.platforms['windows-x86_64'].url, /windows_x64-setup\.exe$/);
  assert.match(result.platforms['darwin-aarch64'].url, /darwin_aarch64\.app\.tar\.gz$/);
  assert.equal(result.notes, 'Release notes');
  assert.equal(result.pub_date, '2026-10-09T00:00:00.000Z');
});

test('each missing installer, updater archive or signature blocks publication', async () => {
  for (const name of requiredAssets(version)) {
    const f = fixture(); f.state.assets = f.state.assets.filter((a) => a.name !== name);
    await assert.rejects(publishRelease(f.args), /Missing/);
    assert.deepEqual(f.mutations, []);
    assert.equal(f.state.release.draft, true);
  }
});

test('incomplete, empty, duplicate, wrong-URL and unexpected assets are rejected', async () => {
  for (const change of [
    (assets) => { assets[0].state = 'starter'; },
    (assets) => { assets[0].size = 0; },
    (assets) => { assets.push({ ...assets[0] }); },
    (assets) => { assets[0].browser_download_url = 'https://evil.invalid/package'; },
    (assets) => { assets.push({ ...assets[0], name: 'old-package.exe' }); },
  ]) {
    const f = fixture(); change(f.state.assets);
    await assert.rejects(publishRelease(f.args)); assert.deepEqual(f.mutations, []);
  }
});

test('size or GitHub digest mismatch blocks publication', async () => {
  for (const change of [
    (f) => { f.state.assets[0].size++; },
    (f) => { f.state.assets[0].digest = 'sha256:invalid'; },
  ]) {
    const f = fixture(); change(f);
    await assert.rejects(publishRelease(f.args), /mismatch/); assert.deepEqual(f.mutations, []);
  }
});

test('any verifier failure leaves the release draft and never writes a manifest', async () => {
  for (const failure of [1, 2, 3]) {
    const f = fixture(); let calls = 0;
    f.args.verify = () => { if (++calls === failure) throw new Error('bad signature'); };
    await assert.rejects(publishRelease(f.args), /bad signature/);
    assert.deepEqual(f.mutations, []); assert.equal(f.state.release.draft, true);
  }
});

test('published release or moved source is never mutated', async () => {
  const f = fixture(); f.state.release.draft = false;
  await assert.rejects(publishRelease(f.args), /already-published/); assert.deepEqual(f.mutations, []);
  f.state.release.draft = true; f.state.tagSha = 'b'.repeat(40);
  await assert.rejects(publishRelease(f.args), /moved/); assert.deepEqual(f.mutations, []);
});

test('manifest must round-trip before publication', async () => {
  const f = fixture(); const download = f.args.download;
  f.args.download = async (...args) => { await download(...args); if (args[2].name === 'latest.json') fs.appendFileSync(args[3], 'corrupted'); };
  await assert.rejects(publishRelease(f.args), /round-trip/);
  assert.deepEqual(f.mutations, ['manifest']); assert.equal(f.state.release.draft, true);
});

test('asset replacement during verification blocks publication', async () => {
  const f = fixture(); let lists = 0;
  f.args.github.paginate = async () => {
    if (++lists === 2) f.state.assets = f.state.assets.map((a, i) => i === 0 ? { ...a, id: 999 } : a);
    return f.state.assets;
  };
  await assert.rejects(publishRelease(f.args), /changed during/);
  assert.equal(f.state.release.draft, true);
});

test('tag movement during verification blocks publication', async () => {
  const f = fixture(); f.args.verify = () => { f.state.tagSha = 'b'.repeat(40); };
  await assert.rejects(publishRelease(f.args), /moved during/); assert.equal(f.state.release.draft, true);
});

test('equal or newer stable latest release cannot be downgraded', async () => {
  for (const latest of ['v1.2.3', 'v1.2.4', 'v2.0.0']) {
    const f = fixture(); f.state.latest = latest;
    await assert.rejects(publishRelease(f.args), /equal or older/); assert.equal(f.state.release.draft, true);
  }
  assert.equal(compareStableVersions('v1.2.10', 'v1.2.9'), 1);
});

test('existing draft manifest is replaced only after artifact verification', async () => {
  const f = fixture(); f.state.assets.push({ id: 98, name: 'latest.json' });
  await publishRelease(f.args);
  assert.deepEqual(f.mutations, ['delete', 'manifest', 'publish']);
});

test('strict base64 decoding rejects malformed signature files', () => {
  assert.match(decodeMinisign(encodedSignature, 'signature').toString(), /^untrusted comment:/);
  for (const value of ['', 'not-base64', encodedSignature + '!', Buffer.from('wrong format').toString('base64')]) {
    assert.throws(() => decodeMinisign(value, 'signature'), /Invalid base64/);
  }
});

test('verification invokes official minisign with decoded files and propagates rejection', () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'luma-test-'));
  const pubkey = Buffer.from('untrusted comment: minisign public key\ntest\n').toString('base64');
  try {
    let invoked = false;
    verifySignature({ directory, artifactPath: '/artifact', signature: encodedSignature, publicKey: pubkey,
      execute: (command, args) => {
        invoked = true; assert.equal(command, 'minisign');
        assert.deepEqual(args.slice(0, 3), ['-V', '-m', '/artifact']);
        assert.equal(fs.readFileSync(args[4], 'utf8'), Buffer.from(pubkey, 'base64').toString());
        assert.equal(fs.readFileSync(args[6], 'utf8'), Buffer.from(encodedSignature, 'base64').toString());
      } });
    assert.equal(invoked, true);
    assert.throws(() => verifySignature({ directory, artifactPath: '/artifact', signature: encodedSignature, publicKey: pubkey,
      execute: () => { throw new Error('Signature verification failed'); } }), /verification failed/);
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});

test('asset download requests authenticated binary API content, rejecting JSON metadata', async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'luma-test-'));
  try {
    const github = { rest: { repos: { getReleaseAsset: async (args) => {
      assert.equal(args.headers.accept, 'application/octet-stream'); assert.equal(args.request.parseSuccessResponseBody, false); return { data: Uint8Array.from([1, 2, 3]).buffer };
    } } } };
    await downloadAsset(github, {}, { id: 1, name: 'asset' }, path.join(directory, 'asset'));
    assert.deepEqual([...fs.readFileSync(path.join(directory, 'asset'))], [1, 2, 3]);
    github.rest.repos.getReleaseAsset = async () => ({ data: { name: 'metadata' } });
    await assert.rejects(downloadAsset(github, {}, { id: 1, name: 'asset' }, path.join(directory, 'asset')), /binary data/);
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});


test('raw streamed JSON and text assets retain their exact bytes', async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'luma-test-'));
  try {
    for (const bytes of [Buffer.from('{"version":"1.2.3"}\n'), Buffer.from(encodedSignature)]) {
      const github = { rest: { repos: { getReleaseAsset: async () => ({ data: new ReadableStream({
        start(controller) { controller.enqueue(new Uint8Array(bytes)); controller.close(); },
      }) }) } } };
      const destination = path.join(directory, 'asset');
      await downloadAsset(github, {}, { id: 1, name: 'asset' }, destination);
      assert.ok(fs.readFileSync(destination).equals(bytes));
    }
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});
