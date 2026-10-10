'use strict';
const {test}=require('node:test');const assert=require('node:assert/strict');
const c=require('./validate_request.cjs');
const fs=require('node:fs'),path=require('node:path');
function fixture(){const source='a'.repeat(40),head='b'.repeat(40);return{
 request:{selector:'full-inference',schema:1,purpose:'one-time-feature-branch-qwen-inference-proof',repository:c.REPOSITORY,feature_branch:c.BRANCH,source_sha:source,pack_id:'qwen3-asr-cpu-macos-arm64',prior_native_evidence:{run_id:1,run_attempt:2,job_id:2,proof_json_sha256:'c'.repeat(64)},model_revisions:{...c.MODEL_REVISIONS},model_download_bytes:3720689099,audio_sha256:'59dfb9a4acb36fe2a2affc14bacbee2920ff435cb13cc314a08c13f66ba7860e',minimum_available_ram_bytes:13*1024**3,minimum_free_disk_bytes:12*1024**3,inference_timeout_seconds:900,download_deadline_seconds:600,metadata_only:true},
 event:{repository:{full_name:c.REPOSITORY},ref:'refs/heads/'+c.BRANCH,after:head,before:source,deleted:false,forced:false},commit:{sha:head,parents:[source],request_existed:false,request_mode:'100644'},changed:[c.REQUEST],
 run:{id:1,run_attempt:2,repository:{full_name:c.REPOSITORY},head_repository:{full_name:c.REPOSITORY},head_sha:source,head_branch:c.BRANCH,event:'pull_request',path:'.github/workflows/asr-components.yml',status:'completed',pull_requests:[{number:4,head:{sha:'d'.repeat(40)}}]},job:{id:2,run_id:1,run_attempt:2,head_sha:source,name:'Build component qwen3-asr-cpu-macos-arm64',status:'completed',conclusion:'success',steps:[{name:'Prove private offline-pip assembly without system Python packages',conclusion:'success'}]}};}
test('exact request and immutable native job are accepted despite mutable historical PR head',()=>{const f=fixture();assert.equal(c.validRequest(f.request,f.event,f.commit,f.changed).runner,'macos-15');c.validEvidence(f.request,f.run,f.job);});
for(const [name,mutate]of Object.entries({fork:f=>f.event.repository.full_name='other/repo',branch:f=>f.event.ref='refs/heads/main',symlink:f=>f.commit.request_mode='120000',executableRequest:f=>f.commit.request_mode='100755',oldRequest:f=>f.commit.request_existed=true,forcePush:f=>f.event.forced=true,bundledPush:f=>f.event.before='e'.repeat(40),merge:f=>f.commit.parents.push('e'.repeat(40)),wrongParent:f=>f.commit.parents[0]='e'.repeat(40),extraFile:f=>f.changed.push('src/main.ts'),replay:f=>f.commit.sha='f'.repeat(40),smallRam:f=>f.request.minimum_available_ram_bytes=1,largeModel:f=>f.request.model_revisions['qwen3-asr-0-6b']='f'.repeat(40),publication:f=>f.request.metadata_only=false,extraField:f=>f.request.install_anything=true})){test('reject '+name,()=>{const f=fixture();mutate(f);assert.throws(()=>c.validRequest(f.request,f.event,f.commit,f.changed));});}
for(const [name,mutate]of Object.entries({forkEvidence:f=>f.run.head_repository.full_name='other/repo',wrongPR:f=>f.run.pull_requests[0].number=3,wrongAttempt:f=>f.run.run_attempt=1,wrongJobAttempt:f=>f.job.run_attempt=1,jobSource:f=>f.job.head_sha='e'.repeat(40),runSource:f=>f.run.head_sha='e'.repeat(40),wrongWorkflow:f=>f.run.path='other.yml',failedJob:f=>f.job.conclusion='failure',skippedProof:f=>f.job.steps[0].conclusion='skipped',wrongJob:f=>f.job.name='other',wrongRun:f=>f.job.run_id=999})){test('reject native '+name,()=>{const f=fixture();mutate(f);assert.throws(()=>c.validEvidence(f.request,f.run,f.job));});}
for (const retiredPath of ['.github/workflows/asr-qwen-inference-proof.yml',c.REQUEST]) {
 test('retired one-time Qwen control is absent: '+retiredPath,()=>{
  // lstat also rejects a dangling symlink at a retired control path.
  assert.throws(()=>fs.lstatSync(path.join(__dirname,'../../..',retiredPath)),{code:'ENOENT'});
 });
}

 test('exact README-only request retains all immutable source/provenance guards',()=>{
  const f=fixture();f.request.selector='readme-diagnostic';f.request.diagnostic={...c.README_REQUEST};
  assert.equal(c.validRequest(f.request,f.event,f.commit,f.changed).selector,'readme-diagnostic');
  c.validEvidence(f.request,f.run,f.job);
 });
 for(const [name,mutate] of Object.entries({missing:f=>delete f.request.selector,unknown:f=>f.request.selector='any-file',empty:f=>f.request.selector='',wrongType:f=>f.request.selector=true,implicitDiagnostic:f=>f.request.diagnostic={...c.README_REQUEST}})) {
  test('reject selector '+name,()=>{const f=fixture();mutate(f);assert.throws(()=>c.validRequest(f.request,f.event,f.commit,f.changed));});
 }
 for(const key of Object.keys(c.README_REQUEST)) {
  test('reject README override '+key,()=>{const f=fixture();f.request.selector='readme-diagnostic';f.request.diagnostic={...c.README_REQUEST,[key]:typeof c.README_REQUEST[key]==='number'?c.README_REQUEST[key]+1:'changed'};assert.throws(()=>c.validRequest(f.request,f.event,f.commit,f.changed));});
 }
 test('README selection cannot omit pins, add overrides or loosen full-attempt bounds',()=>{
  for(const mutate of [f=>delete f.request.diagnostic,f=>f.request.diagnostic.url='https://example.test',f=>f.request.minimum_available_ram_bytes=1,f=>f.request.download_deadline_seconds=601,f=>f.request.inference_timeout_seconds=901]){
   const f=fixture();f.request.selector='readme-diagnostic';f.request.diagnostic={...c.README_REQUEST};mutate(f);assert.throws(()=>c.validRequest(f.request,f.event,f.commit,f.changed));
  }
 });
