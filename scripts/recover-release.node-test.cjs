'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { recoverRelease, parseRecoveryRequest, publicApi } = require('./recover-release.cjs');
const { sourceMarker } = require('./prepare-release.cjs');
const { requiredAssets } = require('./publish-release.cjs');
const source = 'a'.repeat(40), build = 'b'.repeat(40), controller = 'c'.repeat(40), head = 'd'.repeat(40);
const tag = 'v1.2.3';
const entry = (path, sha, type = 'blob') => ({ path, sha, type, mode: type === 'tree' ? '040000' : '100644' });
function fixture() {
  const request = { schema_version: 1, controller_sha: controller, source_sha: source, tag,
    release_id: 42, build_run_id: 1001, build_run_attempt: 1,
    assets: requiredAssets('1.2.3').map((name, index) => ({ name, id: index + 1, size: 100, digest: 'sha256:'+'e'.repeat(64) })) };
  const config = { version:'1.2.3', bundle:{createUpdaterArtifacts:true}, plugins:{updater:{pubkey:'original-source-public-key'}} };
  const state = { request, main: head, tag: source, parent: controller,
    original: { schema_version:1, tag, source_sha: source }, originalParent:source,
    run: { id:1001, run_attempt:1, head_sha:build, repository:{full_name:'owner/repo'}, head_repository:{full_name:'owner/repo'}, event:'push', head_branch:'main', path:'.github/workflows/release.yml', status:'completed' },
    release: { id:42, tag_name:tag, body:sourceMarker(source), draft:true, prerelease:false },
    trees: {
      [source]:[entry('.github','gs','tree'),entry('app','same')],
      [build]:[entry('.github','gb','tree'),entry('app','same')],
      [controller]:[entry('.github','gc','tree'),entry('app','controller')],
      [head]:[entry('.github','gh','tree'),entry('app','controller')],
      gs:[entry('workflows','original','tree')],
      gb:[entry('workflows','original','tree'),entry('release-request.json','request')],
      gc:[entry('workflows','recovery','tree'),entry('release-request.json','request')],
      gh:[entry('workflows','recovery','tree'),entry('release-request.json','request'),entry('release-recovery.json','recovery')],
    },
  };
  state.jobs = { total_count:3, jobs:['Validate pinned source and prepare draft','Build Windows x64','Build macOS Apple Silicon'].map((name) => ({name,run_id:1001,head_sha:build,status:'completed',conclusion:'success'})) };
  const files = { 'package.json':JSON.stringify({version:'1.2.3'}), 'src-tauri/tauri.conf.json':JSON.stringify(config), 'src-tauri/Cargo.toml':'[package]\nname = "luma-subtitle"\nversion = "1.2.3"\n', 'src-tauri/Cargo.lock':'[[package]]\nname = "luma-subtitle"\nversion = "1.2.3"\n' };
  const published = [], fetched = [];
  const github = { rest:{
    git:{
      getRef: async ({ref}) => ({data:{object:{type:'commit',sha:ref==='heads/main'?state.main:state.tag}}}),
      getTree: async ({tree_sha}) => ({data:{tree:state.trees[tree_sha],truncated:false}}),
    },
    repos:{
      getCommit: async ({ref}) => ({data:{parents:[{sha:ref===head?state.parent:state.originalParent}]}}),
      getContent: async ({path,ref}) => {
        let value;
        if(path==='.github/release-recovery.json') { assert.equal(ref,head); value=JSON.stringify(state.request); }
        else if(path==='.github/release-request.json') { assert.equal(ref,build); value=JSON.stringify(state.original); }
        else { assert.equal(ref,source); value=files[path]; }
        return {data:{type:'file',encoding:'base64',content:Buffer.from(value).toString('base64')}};
      },
      getRelease: async ({release_id}) => { assert.equal(release_id,state.request.release_id); return {data:state.release}; },
    },
  }};
  const args = { github,context:{repo:{owner:'owner',repo:'repo'},eventName:'push',ref:'refs/heads/main',sha:head,payload:{}},core:{},
    fetchImpl:async (url,options) => { fetched.push({url,options}); return {ok:true,json:async()=>url.includes('/jobs?')?state.jobs:state.run}; },
    publish:async (input)=>{published.push(input);return {verified:true};},
  };
  return {state,files,published,fetched,args};
}

test('request-only recovery pins original request source and successful platform attempt', async()=>{
  const f=fixture(); await recoverRelease(f.args);
  assert.equal(f.published.length,1); assert.equal(f.published[0].sourceSha,source);
  assert.equal(f.published[0].releaseId,'42'); assert.deepEqual(f.published[0].expectedAssets,f.state.request.assets);
  assert.equal(f.published[0].config.plugins.updater.pubkey,'original-source-public-key');
  assert.equal(f.fetched.length,2);
  for(const call of f.fetched) { assert.ok(call.url.startsWith('https://api.github.com/repos/owner/repo/actions/runs/1001')); assert.equal(call.options.redirect,'error'); assert.equal(call.options.headers.authorization,undefined); }
});

test('request rejects schema expansion, unsafe IDs, invalid digests and incomplete asset pins',()=>{
  for(const change of [r=>r.extra=true,r=>r.build_run_id=-1,r=>r.build_run_attempt=0,r=>r.release_id=Number.MAX_SAFE_INTEGER+1,r=>r.assets.pop(),r=>r.assets[0].digest='sha256:no',r=>r.assets[0].id=r.assets[1].id,r=>r.assets[0].name='different.exe']) {
    const f=fixture();change(f.state.request);assert.throws(()=>parseRecoveryRequest(JSON.stringify(f.state.request),{parents:[{sha:controller}]}));
  }
});

test('request rejects wrong parent, concurrent main change, tag move and non-main events',async()=>{
  for(const change of [f=>f.state.parent=source,f=>f.state.main=source,f=>f.state.tag=head,f=>f.args.context.eventName='pull_request',f=>f.args.context.ref='refs/heads/feature',f=>f.args.context.payload.deleted=true]) {
    const f=fixture();change(f);await assert.rejects(recoverRelease(f.args));assert.equal(f.published.length,0);
  }
});

test('recovery commit cannot carry other source or workflow changes',async()=>{
  for(const change of [f=>f.state.trees[head][1].sha='changed',f=>f.state.trees.gh[0].sha='changed']) {
    const f=fixture();change(f);await assert.rejects(recoverRelease(f.args),/change only/);assert.equal(f.published.length,0);
  }
});

test('run rejects repository, fork, workflow, attempt, branch and unfinished mismatches',async()=>{
  for(const change of [r=>r.repository.full_name='other/repo',r=>r.head_repository.full_name='fork/repo',r=>r.path='.github/workflows/evil.yml',r=>r.event='pull_request',r=>r.head_branch='feature',r=>r.run_attempt=2,r=>r.id=1002,r=>r.status='in_progress']) {
    const f=fixture();change(f.state.run);await assert.rejects(recoverRelease(f.args),/build run/);assert.equal(f.published.length,0);
  }
});

test('both successful platform jobs and prepare job must match run and source request',async()=>{
  for(const change of [j=>j.jobs[0].conclusion='failure',j=>j.jobs[1].conclusion='failure',j=>j.jobs[2].conclusion='cancelled',j=>j.jobs[1].head_sha=source,j=>j.jobs[1].run_id=1002,j=>j.jobs[1].status='in_progress',j=>{j.jobs.push({...j.jobs[1]});j.total_count++;},j=>j.total_count++]) {
    const f=fixture();change(f.state.jobs);await assert.rejects(recoverRelease(f.args));assert.equal(f.published.length,0);
  }
});

test('original build request must exclusively pin the immutable app source',async()=>{
  for(const change of [f=>f.state.original.source_sha=head,f=>f.state.originalParent=head,f=>f.state.original.tag='v1.2.4',f=>f.state.trees[build][1].sha='different',f=>f.state.trees.gb[0].sha='changed']) {
    const f=fixture();change(f);await assert.rejects(recoverRelease(f.args));assert.equal(f.published.length,0);
  }
});

test('source versions and release identity cannot be changed by the controller',async()=>{
  for(const change of [f=>f.files['package.json']='{"version":"1.2.4"}',f=>f.state.release.id=43,f=>f.state.release.tag_name='v1.2.4',f=>f.state.release.body=sourceMarker(head),f=>f.state.release.prerelease=true]) {
    const f=fixture();change(f);await assert.rejects(recoverRelease(f.args));assert.equal(f.published.length,0);
  }
});

test('published state is only delegated to the publisher for exact read-only verification',async()=>{
  const f=fixture();f.state.release.draft=false;await recoverRelease(f.args);
  assert.equal(f.published.length,1);assert.deepEqual(f.published[0].expectedAssets,f.state.request.assets);
});

test('public provenance fetch fails closed and does not follow attacker paths',async()=>{
  let calls=0;
  const fetchImpl=async()=>{calls++;return {ok:false,status:403};};
  await assert.rejects(publicApi({owner:'owner',repo:'repo'},'actions/runs/1001',fetchImpl),/HTTP 403/);
  await assert.rejects(publicApi({owner:'owner/evil',repo:'repo'},'actions/runs/1001',fetchImpl),/Invalid/);
  await assert.rejects(publicApi({owner:'owner',repo:'repo'},'https://evil.invalid',fetchImpl),/Invalid/);
  assert.equal(calls,1);
});
