'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { sourceMarker } = require('./prepare-release.cjs');
const { requiredAssets, assetUrl, apiAssetUrl, validBrowserUrl, decodeMinisign, verifySignature, downloadAsset, compareStableVersions, publishRelease } = require('./publish-release.cjs');

const source = 'a'.repeat(40);
const tag = 'v1.2.3';
const version = '1.2.3';
const draftTag = 'untagged-e212d3f8ec305b2b3d51';
const digest = (bytes) => `sha256:${createHash('sha256').update(bytes).digest('hex')}`;
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
      digest: digest(data), url: apiAssetUrl(repo, index + 1), browser_download_url: assetUrl(repo, draftTag, name) });
  });
  const github = {
    paginate: async () => state.assets.map((asset) => ({ ...asset })),
    rest: {
      git: { getRef: async () => ({ data: { object: { type: 'commit', sha: state.tagSha } } }) },
      repos: {
        getRelease: async () => ({ data: { ...state.release } }),
        getLatestRelease: async () => ({ data: { tag_name: state.latest } }),
        listReleaseAssets: () => {},
        deleteReleaseAsset: async ({ asset_id }) => { mutations.push('delete'); state.assets = state.assets.filter((a) => a.id !== asset_id); },
        uploadReleaseAsset: async ({ data, name }) => {
          mutations.push('manifest');
          const asset = { id: 100, name, state: 'uploaded', size: data.length, digest: digest(data),
            url: apiAssetUrl(repo, 100), browser_download_url: assetUrl(repo, draftTag, name) };
          state.assets.push(asset); contents.set(100, data); return { data: asset };
        },
        updateRelease: async (args) => {
          mutations.push('publish'); assert.equal(args.draft, false);
          state.release = { ...state.release, draft: false };
          state.assets = state.assets.map((asset) => ({ ...asset, browser_download_url: assetUrl(repo, tag, asset.name) }));
          state.afterPublish?.();
          return { data: state.release };
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
    await assert.rejects(publishRelease(f.args), /mismatch|metadata/); assert.deepEqual(f.mutations, []);
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

test('incomplete published release or moved source is never mutated', async () => {
  const f = fixture(); f.state.release.draft = false;
  await assert.rejects(publishRelease(f.args), /Missing/); assert.deepEqual(f.mutations, []);
  f.state.release.draft = true; f.state.tagSha = 'b'.repeat(40);
  await assert.rejects(publishRelease(f.args), /moved/); assert.deepEqual(f.mutations, []);
});

test('manifest must round-trip before publication', async () => {
  const f = fixture(); const download = f.args.download;
  f.args.download = async (...args) => { await download(...args); if (args[2].name === 'latest.json') fs.appendFileSync(args[3], 'corrupted'); };
  await assert.rejects(publishRelease(f.args), /round-trip|mismatch/);
  assert.deepEqual(f.mutations, ['manifest']); assert.equal(f.state.release.draft, true);
});

test('asset replacement during verification blocks publication', async () => {
  const f = fixture(); let lists = 0;
  f.args.github.paginate = async () => {
    if (++lists === 2) f.state.assets = f.state.assets.map((a, i) => i === 0 ? { ...a, id: 999, url: apiAssetUrl(f.args.context.repo, 999) } : a);
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
  const f = fixture(); const data = Buffer.from('old manifest');
  f.contents.set(98, data);
  f.state.assets.push({ id: 98, name: 'latest.json', state: 'uploaded', size: data.length,
    digest: digest(data), url: apiAssetUrl(f.args.context.repo, 98),
    browser_download_url: assetUrl(f.args.context.repo, draftTag, 'latest.json') });
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


test('the observed provisional draft URLs become canonical URLs only on publication', async () => {
  const f = fixture();
  assert.ok(f.state.assets.every((asset) => asset.browser_download_url.includes(`/${draftTag}/`)));
  const manifest = await publishRelease(f.args);
  assert.ok(f.state.assets.every((asset) => asset.browser_download_url.includes(`/${tag}/`)));
  assert.ok(Object.values(manifest.platforms).every((platform) => platform.url.includes(`/${tag}/`)));
});

test('canonical URLs are also accepted for a draft but provisional URLs are never accepted as published', async () => {
  const f = fixture(); f.state.assets.forEach((asset) => { asset.browser_download_url = assetUrl(f.args.context.repo, tag, asset.name); });
  await publishRelease(f.args);
  const name = requiredAssets(version)[0];
  assert.equal(validBrowserUrl(assetUrl(f.args.context.repo, draftTag, name), f.args.context.repo, tag, name, false), false);
});

test('wrong host, owner, repository, tag, name and malicious provisional URLs cannot pass', async () => {
  const f = fixture(); const good = f.state.assets[0].browser_download_url;
  for (const bad of [
    good.replace('github.com', 'github.com.evil.invalid'),
    good.replace('https://', 'http://'),
    good.replace('github.com/', 'user@github.com/'),
    good.replace('/owner/', '/attacker/'),
    good.replace('/repo/', '/wrong-repo/'),
    good.replace(draftTag, 'v1.2.2'),
    good.replace(draftTag, 'untagged-nothex'),
    good.replace(draftTag, `${draftTag}a`),
    good.replace(draftTag, `${draftTag}/..`),
    good.replace(draftTag, `${draftTag}%2f..`),
    good.replace(draftTag, draftTag.toUpperCase()),
    `${good}?token=anything`, `${good}#anything`, `${good}/extra`,
    good.replace('.dmg', '.exe'),
  ]) {
    const next = fixture(); next.state.assets[0].browser_download_url = bad;
    await assert.rejects(publishRelease(next.args), /identity|URL/);
    assert.deepEqual(next.mutations, []);
  }
});

test('missing or malformed digests and mismatched API asset identities fail before any write', async () => {
  for (const change of [
    (asset) => { delete asset.digest; },
    (asset) => { asset.digest = 'sha256:abc'; },
    (asset) => { asset.digest = 'md5:' + 'a'.repeat(64); },
    (asset) => { delete asset.url; },
    (asset) => { asset.url = asset.url.replace('api.github.com', 'evil.invalid'); },
    (asset) => { asset.url = asset.url.replace('/owner/', '/attacker/'); },
    (asset) => { asset.url = asset.url.replace('/repo/', '/wrong-repo/'); },
    (asset) => { asset.url += '?anything'; },
    (asset) => { asset.id = 999; },
    (asset) => { asset.id = '1'; },
    (asset) => { asset.id = -1; },
  ]) {
    const f = fixture(); change(f.state.assets[0]);
    await assert.rejects(publishRelease(f.args), /metadata/); assert.deepEqual(f.mutations, []);
  }
});

test('post-publication canonical URL or immutable asset mismatch is reported without corrective writes', async () => {
  for (const change of [
    (f) => { f.state.assets[0].browser_download_url = assetUrl(f.args.context.repo, draftTag, f.state.assets[0].name); },
    (f) => { f.state.assets[0].browser_download_url = f.state.assets[0].browser_download_url.replace(tag, 'v1.2.2'); },
    (f) => { f.state.assets[0].digest = 'sha256:' + 'f'.repeat(64); },
    (f) => { f.state.assets[0].id = 999; f.state.assets[0].url = apiAssetUrl(f.args.context.repo, 999); },
  ]) {
    const f = fixture(); f.state.afterPublish = () => change(f);
    await assert.rejects(publishRelease(f.args), /URL|digests/);
    assert.deepEqual(f.mutations, ['manifest', 'publish']); assert.equal(f.state.release.draft, false);
  }
});

test('an exact published retry revalidates every signature and manifest without any writes', async () => {
  const f = fixture(); const first = await publishRelease(f.args);
  f.mutations.length = 0; f.verified.length = 0;
  f.args.now = () => new Date('2027-01-01T00:00:00Z');
  const second = await publishRelease(f.args);
  assert.deepEqual(second, first); assert.equal(f.verified.length, 3); assert.deepEqual(f.mutations, []);
});

test('a publication whose immediate URL check failed can be safely retried after metadata converges', async () => {
  const f = fixture(); f.state.afterPublish = () => {
    f.state.assets[0].browser_download_url = assetUrl(f.args.context.repo, draftTag, f.state.assets[0].name);
  };
  await assert.rejects(publishRelease(f.args), /URL/);
  f.state.assets.forEach((asset) => { asset.browser_download_url = assetUrl(f.args.context.repo, tag, asset.name); });
  f.mutations.length = 0; f.verified.length = 0;
  await publishRelease(f.args);
  assert.equal(f.verified.length, 3); assert.deepEqual(f.mutations, []);
});

test('published retry rejects incorrect source, tag, ID, manifest or signature without repair', async () => {
  for (const change of [
    (f) => { f.state.release.id = 999; },
    (f) => { f.state.release.tag_name = 'v1.2.2'; },
    (f) => { f.state.release.body = sourceMarker('b'.repeat(40)); },
    (f) => { f.state.tagSha = 'b'.repeat(40); },
    (f) => { f.args.verify = () => { throw new Error('bad signature'); }; },
    (f) => { f.contents.set(100, Buffer.from('corrupted')); },
    (f) => {
      const manifest = JSON.parse(f.contents.get(100)); manifest.platforms['darwin-aarch64'].url = 'https://evil.invalid/asset';
      const bytes = Buffer.from(JSON.stringify(manifest, null, 2) + '\n'); f.contents.set(100, bytes);
      Object.assign(f.state.assets.find((a) => a.id === 100), { size: bytes.length, digest: digest(bytes) });
    },
    (f) => {
      const manifest = JSON.parse(f.contents.get(100)); manifest.pub_date = 'not-a-date';
      const bytes = Buffer.from(JSON.stringify(manifest, null, 2) + '\n'); f.contents.set(100, bytes);
      Object.assign(f.state.assets.find((a) => a.id === 100), { size: bytes.length, digest: digest(bytes) });
    },
  ]) {
    const f = fixture(); await publishRelease(f.args); f.mutations.length = 0; change(f);
    await assert.rejects(publishRelease(f.args)); assert.deepEqual(f.mutations, []);
  }
});

test('external publication during draft verification prevents all writes', async () => {
  const f = fixture(); f.args.verify = () => { f.state.release.draft = false; };
  await assert.rejects(publishRelease(f.args), /already-published/); assert.deepEqual(f.mutations, []);
});


test('request-pinned asset identities and digests are checked within the publisher', async () => {
  const good = fixture();
  good.args.expectedAssets = good.state.assets.map(({ name, id, size, digest }) => ({ name, id, size, digest })).reverse();
  await publishRelease(good.args);
  for (const change of [
    (assets) => { assets.pop(); },
    (assets) => { assets[0].id++; },
    (assets) => { assets[0].size++; },
    (assets) => { assets[0].digest = 'sha256:' + 'a'.repeat(64); },
    (assets) => { assets[0].name = 'wrong'; },
  ]) {
    const f = fixture(); f.args.expectedAssets = f.state.assets.map(({ name, id, size, digest }) => ({ name, id, size, digest }));
    change(f.args.expectedAssets);
    await assert.rejects(publishRelease(f.args), /request-pinned/);
    assert.deepEqual(f.mutations, []);
  }
});

test('release state is checked again immediately before the manifest upload', async () => {
  const f = fixture(); let reads = 0;
  const getRelease = f.args.github.rest.repos.getRelease;
  f.args.github.rest.repos.getRelease = async (...args) => {
    if (++reads === 3) f.state.release.draft = false;
    return getRelease(...args);
  };
  await assert.rejects(publishRelease(f.args), /already-published/);
  assert.deepEqual(f.mutations, []);
});

test('published notes changing during read-only verification are rejected', async () => {
  const f = fixture(); await publishRelease(f.args); f.mutations.length = 0;
  f.args.verify = () => { f.state.release.body = `Changed notes\n\n${sourceMarker(source)}`; };
  await assert.rejects(publishRelease(f.args), /notes or prerelease status changed/);
  assert.deepEqual(f.mutations, []);
});


function starterManifest(f) {
  return { id: 100, name: 'latest.json', state: 'starter', size: 0, digest: null,
    url: apiAssetUrl(f.args.context.repo, 100), browser_download_url: assetUrl(f.args.context.repo, draftTag, 'latest.json') };
}

test('a failed draft manifest upload leaving a starter asset is recoverable on retry', async () => {
  const f = fixture(); const upload = f.args.github.rest.repos.uploadReleaseAsset;
  f.args.github.rest.repos.uploadReleaseAsset = async () => {
    f.mutations.push('manifest'); f.state.assets.push(starterManifest(f));
    throw Object.assign(new Error('Upload interrupted'), { status: 502 });
  };
  await assert.rejects(publishRelease(f.args), /Upload interrupted/);
  assert.equal(f.state.release.draft, true); assert.deepEqual(f.mutations, ['manifest']);
  f.args.github.rest.repos.uploadReleaseAsset = upload;
  f.mutations.length = 0; f.verified.length = 0;
  await publishRelease(f.args);
  assert.equal(f.verified.length, 3); assert.deepEqual(f.mutations, ['delete', 'manifest', 'publish']);
});

test('starter cleanup is draft-manifest-only and requires exact identity and empty metadata', async () => {
  for (const change of [
    (asset) => { asset.size = 1; },
    (asset) => { asset.digest = 'sha256:' + 'a'.repeat(64); },
    (asset) => { asset.state = 'uploaded'; },
    (asset) => { asset.id = -1; },
    (asset) => { asset.url = 'https://api.github.com/repos/attacker/repo/releases/assets/100'; },
    (asset) => { asset.browser_download_url += '?anything'; },
  ]) {
    const f = fixture(); const starter = starterManifest(f); change(starter); f.state.assets.push(starter);
    await assert.rejects(publishRelease(f.args), /metadata/); assert.deepEqual(f.mutations, []);
  }
  const published = fixture(); published.state.assets.push(starterManifest(published)); published.state.release.draft = false;
  published.state.assets.forEach((asset) => { asset.browser_download_url = assetUrl(published.args.context.repo, tag, asset.name); });
  await assert.rejects(publishRelease(published.args), /metadata/); assert.deepEqual(published.mutations, []);
  const binary = fixture(); Object.assign(binary.state.assets[0], { state: 'starter', size: 0, digest: null });
  await assert.rejects(publishRelease(binary.args), /metadata/); assert.deepEqual(binary.mutations, []);
});

test('even a valid draft starter is not deleted unless signatures pass', async () => {
  const f = fixture(); f.state.assets.push(starterManifest(f));
  f.args.verify = () => { throw new Error('bad signature'); };
  await assert.rejects(publishRelease(f.args), /bad signature/); assert.deepEqual(f.mutations, []);
});

test('publication response cannot silently change release notes or prerelease status', async () => {
  for (const change of [
    (f) => { f.state.release.body = `Changed notes\n\n${sourceMarker(source)}`; },
    (f) => { f.state.release.prerelease = true; },
  ]) {
    const f = fixture(); f.state.afterPublish = () => change(f);
    await assert.rejects(publishRelease(f.args), /Published release notes or prerelease status/);
    assert.deepEqual(f.mutations, ['manifest', 'publish']);
  }
});
