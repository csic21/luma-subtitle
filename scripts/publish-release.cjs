'use strict';

const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const { createHash } = require('node:crypto');
const { Readable } = require('node:stream');
const { pipeline } = require('node:stream/promises');
const { TAG, SHA, assertDraft, tagCommit } = require('./prepare-release.cjs');

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

function validateAssets(assets, repo, tag, version) {
  const expected = requiredAssets(version);
  if (assets.some((asset) => !expected.includes(asset.name) && asset.name !== 'latest.json')) {
    throw new Error('Draft contains unexpected assets; refusing to publish stale or unverified files');
  }
  for (const name of expected) {
    const matches = assets.filter((asset) => asset.name === name);
    if (matches.length !== 1 || matches[0].state !== 'uploaded' || matches[0].size <= 0
        || matches[0].browser_download_url !== assetUrl(repo, tag, name)) {
      throw new Error(`Missing, incomplete, duplicate or incorrect asset URL: ${name}`);
    }
  }
  if (assets.filter((asset) => asset.name === 'latest.json').length > 1) throw new Error('Duplicate updater manifests');
  return expected.map((name) => assets.find((asset) => asset.name === name));
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
  if (asset.digest && asset.digest !== `sha256:${createHash('sha256').update(bytes).digest('hex')}`) {
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
  config, download = downloadAsset, verify = verifySignature, now = () => new Date() }) {
  if (!SHA.test(sourceSha || '') || !TAG.test(tag || '') || !/^\d+$/.test(String(releaseId))) {
    throw new Error('Invalid pinned release identity');
  }
  const repo = context.repo;
  const version = tag.slice(1);
  if (config.version !== version || config.bundle?.createUpdaterArtifacts !== true) throw new Error('Source configuration differs from release');
  const release_id = Number(releaseId);
  let release = (await github.rest.repos.getRelease({ ...repo, release_id })).data;
  assertDraft(release, tag, sourceSha);
  if (await tagCommit(github, repo, tag) !== sourceSha) throw new Error('Release tag moved away from pinned source');
  const listAssets = () => github.paginate(github.rest.repos.listReleaseAssets, { ...repo, release_id, per_page: 100 });
  const initialAssets = await listAssets();
  const required = validateAssets(initialAssets, repo, tag, version);
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
    // Only successful verification of both platforms can reach the manifest writer.
    const manifest = createManifest({ repo, tag, version, signatures,
      notes: release.body.replace(/\n*<!-- luma-release-source:[a-f0-9]{40} -->/g, '').trim(),
      pubDate: now().toISOString() });
    const bytes = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`);
    for (const old of initialAssets.filter((asset) => asset.name === 'latest.json')) {
      await github.rest.repos.deleteReleaseAsset({ ...repo, asset_id: old.id });
    }
    const { data: uploaded } = await github.rest.repos.uploadReleaseAsset({
      ...repo, release_id, name: 'latest.json', data: bytes,
      headers: { 'content-type': 'application/json', 'content-length': bytes.length },
    });
    const manifestPath = path.join(directory, 'latest.json');
    await download(github, repo, uploaded, manifestPath);
    if (!fs.readFileSync(manifestPath).equals(bytes)) throw new Error('Uploaded updater manifest did not round-trip');
    const finalAssets = await listAssets();
    const finalRequired = validateAssets(finalAssets, repo, tag, version);
    if (JSON.stringify(finalRequired.map((a) => [a.id, a.size, a.digest]))
        !== JSON.stringify(required.map((a) => [a.id, a.size, a.digest]))) {
      throw new Error('Release assets changed during verification');
    }
    const finalManifest = finalAssets.find((asset) => asset.name === 'latest.json');
    if (finalManifest?.id !== uploaded.id || finalManifest.state !== 'uploaded'
        || finalManifest.size !== bytes.length || finalManifest.browser_download_url !== assetUrl(repo, tag, 'latest.json')) {
      throw new Error('Uploaded updater manifest is missing or invalid');
    }
    release = (await github.rest.repos.getRelease({ ...repo, release_id })).data;
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
    const { data: published } = await github.rest.repos.updateRelease({
      ...repo, release_id, draft: false, make_latest: release.prerelease ? 'false' : 'true',
    });
    if (published.draft || published.tag_name !== tag) throw new Error('GitHub did not confirm release publication');
    core.setOutput('release_url', published.html_url);
    core.info(`Published ${tag} after verifying both platforms and all three updater signatures.`);
    return manifest;
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
}

module.exports = { requiredAssets, assetUrl, validateAssets, decodeMinisign, verifySignature, downloadAsset,
  verifyDownloadedAsset, createManifest, compareStableVersions, publishRelease };
