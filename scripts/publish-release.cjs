'use strict';

const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const { createHash } = require('node:crypto');
const { Readable } = require('node:stream');
const { pipeline } = require('node:stream/promises');
const { TAG, SHA, assertDraft, tagCommit, sourceMarker } = require('./prepare-release.cjs');

function requiredAssets(version) {
  const mac = `Luma-Subtitle_${version}_darwin_aarch64`;
  const win = `Luma-Subtitle_${version}_windows_x64`;
  return [
    `${mac}.dmg`, `${mac}.app.tar.gz`, `${mac}.app.tar.gz.sig`,
    `${win}-setup.exe`, `${win}-setup.exe.sig`, `${win}.msi`, `${win}.msi.sig`,
  ];
}

function assetUrl(repo, tag, name) {
  return `https://github.com/${repo.owner}/${repo.repo}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(name)}`;
}

function apiAssetUrl(repo, id) {
  return `https://api.github.com/repos/${repo.owner}/${repo.repo}/releases/assets/${id}`;
}

function validBrowserUrl(value, repo, tag, name, draft) {
  if (value === assetUrl(repo, tag, name)) return true;
  if (!draft || typeof value !== 'string') return false;
  const prefix = `https://github.com/${repo.owner}/${repo.repo}/releases/download/`;
  if (!value.startsWith(prefix)) return false;
  const parts = value.slice(prefix.length).split('/');
  // GitHub assigns this provisional tag to a draft before its first publication.
  return parts.length === 2 && /^untagged-[a-f0-9]{20}$/.test(parts[0])
    && parts[1] === encodeURIComponent(name);
}

function validateAssetMetadata(asset, repo, tag, draft) {
  if (!Number.isSafeInteger(asset.id) || asset.id <= 0
      || asset.url !== apiAssetUrl(repo, asset.id)
      || asset.state !== 'uploaded' || !Number.isSafeInteger(asset.size) || asset.size <= 0
      || !/^sha256:[a-f0-9]{64}$/.test(asset.digest || '')
      || !validBrowserUrl(asset.browser_download_url, repo, tag, asset.name, draft)) {
    throw new Error(`Invalid asset identity, URL, size or SHA-256 metadata: ${asset.name}`);
  }
}

function validateAssets(assets, repo, tag, version, { draft = true, requireManifest = false } = {}) {
  const expected = requiredAssets(version);
  if (assets.some((asset) => !expected.includes(asset.name) && asset.name !== 'latest.json')) {
    throw new Error('Release contains unexpected assets; refusing stale or unverified files');
  }
  if (new Set(assets.map((asset) => asset.id)).size !== assets.length) throw new Error('Duplicate release asset IDs');
  for (const name of [...expected, ...(requireManifest ? ['latest.json'] : [])]) {
    const matches = assets.filter((asset) => asset.name === name);
    if (matches.length !== 1) throw new Error(`Missing or duplicate release asset: ${name}`);
  }
  if (assets.filter((asset) => asset.name === 'latest.json').length > 1) throw new Error('Duplicate updater manifests');
  for (const asset of assets) {
    // GitHub may leave an empty starter asset after a failed draft manifest upload.
    // Only this exact draft-only placeholder can be cleaned up after signature checks.
    const draftManifestStarter = draft && !requireManifest && asset.name === 'latest.json'
      && asset.state === 'starter' && asset.size === 0 && asset.digest == null
      && Number.isSafeInteger(asset.id) && asset.id > 0 && asset.url === apiAssetUrl(repo, asset.id)
      && validBrowserUrl(asset.browser_download_url, repo, tag, asset.name, true);
    if (!draftManifestStarter) validateAssetMetadata(asset, repo, tag, draft);
  }
  return expected.map((name) => assets.find((asset) => asset.name === name));
}

// Browser URLs legitimately change on publication; all immutable byte identities must not.
function assetSnapshot(assets) {
  return JSON.stringify(assets.map(({ name, id, url, size, digest }) => ({ name, id, url, size, digest }))
    .sort((a, b) => a.name.localeCompare(b.name)));
}

function assertReleaseIdentity(release, releaseId, tag, sourceSha) {
  const markers = release.body?.match(/<!-- luma-release-source:[a-f0-9]{40} -->/g) || [];
  if (release.id !== releaseId || release.tag_name !== tag || typeof release.draft !== 'boolean'
      || markers.length !== 1 || markers[0] !== sourceMarker(sourceSha)) {
    throw new Error('Release identity or source marker differs from the pinned source');
  }
}

function decodeMinisign(encoded, label) {
  const value = encoded.trim();
  const decoded = Buffer.from(value, 'base64');
  if (!value || decoded.toString('base64') !== value || !decoded.toString('utf8').startsWith('untrusted comment:')) {
    throw new Error(`Invalid base64 minisign ${label}`);
  }
  return decoded;
}

// Use the official verifier. Never implement signature crypto in release scripts.
function verifySignature({ artifactPath, signature, publicKey, directory, execute = execFileSync }) {
  const signaturePath = path.join(directory, 'verify.minisig');
  const publicKeyPath = path.join(directory, 'verify.pub');
  fs.writeFileSync(signaturePath, decodeMinisign(signature, 'signature'));
  fs.writeFileSync(publicKeyPath, decodeMinisign(publicKey, 'public key'));
  execute('minisign', ['-V', '-m', artifactPath, '-p', publicKeyPath, '-x', signaturePath], { stdio: 'pipe' });
}

async function downloadAsset(github, repo, asset, destination) {
  const { data } = await github.rest.repos.getReleaseAsset({
    ...repo, asset_id: asset.id, headers: { accept: 'application/octet-stream' },
    // Preserve exact bytes even when an asset's original Content-Type is JSON/text.
    request: { parseSuccessResponseBody: false },
  });
  if (data?.getReader || data?.[Symbol.asyncIterator]) {
    const stream = data.getReader ? Readable.fromWeb(data) : data;
    await pipeline(stream, fs.createWriteStream(destination));
    return;
  }
  if (!(data instanceof ArrayBuffer) && !ArrayBuffer.isView(data)) {
    throw new Error(`GitHub did not return binary data for ${asset.name}`);
  }
  fs.writeFileSync(destination, Buffer.from(data));
}

function verifyDownloadedAsset(asset, destination) {
  const bytes = fs.readFileSync(destination);
  if (bytes.length !== asset.size || !bytes.length) throw new Error(`Downloaded size mismatch: ${asset.name}`);
  if (!/^sha256:[a-f0-9]{64}$/.test(asset.digest || '')
      || asset.digest !== `sha256:${createHash('sha256').update(bytes).digest('hex')}`) {
    throw new Error(`Downloaded digest mismatch: ${asset.name}`);
  }
}

function createManifest({ repo, tag, version, notes, signatures, pubDate }) {
  const mac = `Luma-Subtitle_${version}_darwin_aarch64.app.tar.gz`;
  const win = `Luma-Subtitle_${version}_windows_x64-setup.exe`;
  return {
    version, notes, pub_date: pubDate,
    platforms: {
      'darwin-aarch64': { signature: signatures[mac], url: assetUrl(repo, tag, mac) },
      'windows-x86_64': { signature: signatures[win], url: assetUrl(repo, tag, win) },
    },
  };
}

function compareStableVersions(left, right) {
  const stable = /^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
  if (!stable.test(left) || !stable.test(right)) throw new Error('Cannot compare non-stable latest release versions');
  const a = left.replace(/^v/, '').split('.').map(BigInt);
  const b = right.replace(/^v/, '').split('.').map(BigInt);
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] > b[i] ? 1 : -1;
  return 0;
}

async function publishRelease({ github, context, core, sourceSha, tag, releaseId,
  config, expectedAssets, download = downloadAsset, verify = verifySignature, now = () => new Date() }) {
  if (!SHA.test(sourceSha || '') || !TAG.test(tag || '') || !/^[1-9]\d*$/.test(String(releaseId))
      || !Number.isSafeInteger(Number(releaseId))) {
    throw new Error('Invalid pinned release identity');
  }
  const repo = context.repo;
  const version = tag.slice(1);
  if (config.version !== version || config.bundle?.createUpdaterArtifacts !== true) throw new Error('Source configuration differs from release');
  const release_id = Number(releaseId);
  let releaseMetadata;
  const getRelease = async () => {
    const { data } = await github.rest.repos.getRelease({ ...repo, release_id });
    assertReleaseIdentity(data, release_id, tag, sourceSha);
    if (releaseMetadata && JSON.stringify([data.body, data.prerelease]) !== releaseMetadata) {
      throw new Error('Release notes or prerelease status changed during verification');
    }
    return data;
  };
  let release = await getRelease();
  releaseMetadata = JSON.stringify([release.body, release.prerelease]);
  const alreadyPublished = !release.draft;
  if (await tagCommit(github, repo, tag) !== sourceSha) throw new Error('Release tag moved away from pinned source');
  const listAssets = () => github.paginate(github.rest.repos.listReleaseAssets, { ...repo, release_id, per_page: 100 });
  const initialAssets = await listAssets();
  const required = validateAssets(initialAssets, repo, tag, version,
    { draft: !alreadyPublished, requireManifest: alreadyPublished });
  if (expectedAssets !== undefined) {
    const byteSnapshot = (assets) => JSON.stringify(assets.map(({ name, id, size, digest }) => ({ name, id, size, digest }))
      .sort((a, b) => a.name.localeCompare(b.name)));
    if (!Array.isArray(expectedAssets) || expectedAssets.length !== required.length
        || byteSnapshot(expectedAssets) !== byteSnapshot(required)) {
      throw new Error('Release artifacts differ from the request-pinned asset snapshot');
    }
  }
  const requiredSnapshot = assetSnapshot(required);
  const initialSnapshot = assetSnapshot(initialAssets);
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'luma-release-'));
  try {
    for (const asset of required) {
      const destination = path.join(directory, asset.name);
      await download(github, repo, asset, destination);
      verifyDownloadedAsset(asset, destination);
    }
    const signatures = {};
    for (const name of requiredAssets(version).filter((name) => name.endsWith('.sig'))) {
      const artifactName = name.slice(0, -4);
      const signature = fs.readFileSync(path.join(directory, name), 'utf8').trim();
      verify({ artifactPath: path.join(directory, artifactName), signature,
        publicKey: config.plugins?.updater?.pubkey || '', directory });
      signatures[artifactName] = signature;
    }
    const manifestPath = path.join(directory, 'latest.json');
    let pubDate = now().toISOString();
    if (alreadyPublished) {
      const existing = initialAssets.find((asset) => asset.name === 'latest.json');
      await download(github, repo, existing, manifestPath);
      verifyDownloadedAsset(existing, manifestPath);
      const saved = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
      if (typeof saved.pub_date !== 'string' || !Number.isFinite(Date.parse(saved.pub_date))
          || new Date(saved.pub_date).toISOString() !== saved.pub_date) {
        throw new Error('Published manifest has an invalid publication date');
      }
      pubDate = saved.pub_date;
    }
    const manifest = createManifest({ repo, tag, version, signatures,
      notes: release.body.replace(/\n*<!-- luma-release-source:[a-f0-9]{40} -->/g, '').trim(), pubDate });
    const bytes = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`);
    if (alreadyPublished) {
      // A retry after publication is strictly read-only, including the existing timestamp.
      if (!fs.readFileSync(manifestPath).equals(bytes)) throw new Error('Published updater manifest does not match verified source and artifacts');
      release = await getRelease();
      if (release.draft || await tagCommit(github, repo, tag) !== sourceSha) throw new Error('Published release changed during verification');
      const confirmed = await listAssets();
      validateAssets(confirmed, repo, tag, version, { draft: false, requireManifest: true });
      if (assetSnapshot(confirmed) !== initialSnapshot) throw new Error('Published assets changed during verification');
      core.setOutput('release_url', release.html_url);
      core.info(`Verified already-published ${tag}; no release or asset was modified.`);
      return manifest;
    }

    // Recheck draft state immediately before any mutation, after potentially long downloads.
    release = await getRelease();
    assertDraft(release, tag, sourceSha);
    if (await tagCommit(github, repo, tag) !== sourceSha) throw new Error('Release tag moved during verification');
    const beforeWrite = await listAssets();
    validateAssets(beforeWrite, repo, tag, version);
    if (assetSnapshot(beforeWrite) !== initialSnapshot) throw new Error('Release assets changed during verification');
    for (const old of initialAssets.filter((asset) => asset.name === 'latest.json')) {
      assertDraft(await getRelease(), tag, sourceSha);
      await github.rest.repos.deleteReleaseAsset({ ...repo, asset_id: old.id });
    }
    assertDraft(await getRelease(), tag, sourceSha);
    const { data: uploaded } = await github.rest.repos.uploadReleaseAsset({
      ...repo, release_id, name: 'latest.json', data: bytes,
      headers: { 'content-type': 'application/json', 'content-length': bytes.length },
    });
    if (uploaded.name !== 'latest.json') throw new Error('Uploaded manifest has the wrong asset name');
    validateAssetMetadata(uploaded, repo, tag, true);
    await download(github, repo, uploaded, manifestPath);
    verifyDownloadedAsset(uploaded, manifestPath);
    if (!fs.readFileSync(manifestPath).equals(bytes)) throw new Error('Uploaded updater manifest did not round-trip');
    const finalAssets = await listAssets();
    const finalRequired = validateAssets(finalAssets, repo, tag, version, { requireManifest: true });
    if (assetSnapshot(finalRequired) !== requiredSnapshot) throw new Error('Release assets changed during verification');
    const finalManifest = finalAssets.find((asset) => asset.name === 'latest.json');
    if (assetSnapshot([finalManifest]) !== assetSnapshot([uploaded]) || finalManifest.size !== bytes.length) {
      throw new Error('Uploaded updater manifest is missing or invalid');
    }
    release = await getRelease();
    assertDraft(release, tag, sourceSha);
    if (await tagCommit(github, repo, tag) !== sourceSha) throw new Error('Release tag moved during verification');
    if (!release.prerelease) {
      let latest;
      try { latest = (await github.rest.repos.getLatestRelease(repo)).data; }
      catch (error) { if (error.status !== 404) throw error; }
      if (latest && compareStableVersions(tag, latest.tag_name) <= 0) {
        throw new Error('Refusing to replace latest with an equal or older stable release');
      }
    }
    const verifiedPublishedSnapshot = assetSnapshot(finalAssets);
    const { data: published } = await github.rest.repos.updateRelease({
      ...repo, release_id, draft: false, make_latest: release.prerelease ? 'false' : 'true',
    });
    assertReleaseIdentity(published, release_id, tag, sourceSha);
    if (JSON.stringify([published.body, published.prerelease]) !== releaseMetadata) {
      throw new Error('Published release notes or prerelease status differs from the verified draft');
    }
    if (published.draft) throw new Error('GitHub did not confirm release publication');
    const publishedAssets = await listAssets();
    validateAssets(publishedAssets, repo, tag, version, { draft: false, requireManifest: true });
    if (assetSnapshot(publishedAssets) !== verifiedPublishedSnapshot) {
      throw new Error('Published asset identities or digests differ from the verified draft');
    }
    if (await tagCommit(github, repo, tag) !== sourceSha) throw new Error('Release tag moved after publication');
    core.setOutput('release_url', published.html_url);
    core.info(`Published ${tag} after verifying both platforms and all three updater signatures.`);
    return manifest;
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
}

module.exports = { requiredAssets, assetUrl, apiAssetUrl, validBrowserUrl, validateAssetMetadata, validateAssets, assetSnapshot, decodeMinisign, verifySignature, downloadAsset,
  verifyDownloadedAsset, createManifest, compareStableVersions, publishRelease };
