'use strict';
// Temporary feature-branch control. Read-only and never a normal PR weight trigger.
const fs = require('node:fs');
const {execFileSync} = require('node:child_process');
const REQUEST = '.github/requests/qwen-inference-proof.json';
const REPOSITORY = 'csic21/luma-subtitle';
const BRANCH = 'feat/optional-asr-engines';
const MODEL_REVISIONS = Object.freeze({
  'qwen3-asr-0-6b':'5eb144179a02acc5e5ba31e748d22b0cf3e303b0',
  'qwen3-forced-aligner-0-6b':'c7cbfc2048c462b0d63a45797104fc9db3ad62b7'
});
const README_REQUEST = Object.freeze({component:'qwen3-asr-0-6b',revision:MODEL_REVISIONS['qwen3-asr-0-6b'],path:'README.md',expected_bytes:57456,expected_sha256:'5058416891bc47a2051557765997e8c42f8eb78a0e33c3e775bd17d4b0ba4d50',max_bytes:57456,deadline_seconds:30});
function check(ok, message) { if (!ok) throw new Error(message); }
function keys(value, expected) {
  check(value && typeof value === 'object' && !Array.isArray(value), 'Expected an object');
  check(JSON.stringify(Object.keys(value).sort()) === JSON.stringify([...expected].sort()), 'Unexpected request fields');
}
function validRequest(request, event, commit, changed) {
  check(request && ['full-inference','readme-diagnostic'].includes(request.selector), 'An exact supported proof selector is required');
  const diagnostic=request.selector==='readme-diagnostic';
  keys(request,['selector',...(diagnostic?['diagnostic']:[]),'schema','purpose','repository','feature_branch','source_sha','pack_id','prior_native_evidence','model_revisions','model_download_bytes','audio_sha256','minimum_available_ram_bytes','minimum_free_disk_bytes','inference_timeout_seconds','download_deadline_seconds','metadata_only']);
  if(diagnostic) {
    keys(request.diagnostic,Object.keys(README_REQUEST));
    for(const [key,value] of Object.entries(README_REQUEST)) check(request.diagnostic[key]===value,'README diagnostic bounds or identity changed');
  }
  check(request.schema===1 && request.purpose==='one-time-feature-branch-qwen-inference-proof', 'Wrong proof purpose');
  check(request.repository===REPOSITORY && request.feature_branch===BRANCH, 'Wrong request repository/branch');
  check(event.repository?.full_name===REPOSITORY && event.ref==='refs/heads/'+BRANCH && event.deleted===false && event.forced===false, 'Only the reviewed feature-branch push may request this proof');
  check(/^[0-9a-f]{40}$/.test(request.source_sha), 'Source must be immutable');
  check(commit.sha===event.after && event.before===request.source_sha && commit.parents.length===1 && commit.parents[0]===request.source_sha, 'Request commit must have tested source as sole parent');
  check(changed.length===1 && changed[0]===REQUEST && commit.request_existed===false && commit.request_mode==='100644', 'Request commit may add only the new bounded request file');
  check(['qwen3-asr-cpu-windows-x64','qwen3-asr-cpu-macos-arm64'].includes(request.pack_id), 'Only one reviewed native Qwen CPU pack is allowed');
  keys(request.prior_native_evidence,['run_id','run_attempt','job_id','proof_json_sha256']);
  check(Number.isSafeInteger(request.prior_native_evidence.run_id) && request.prior_native_evidence.run_id>0 && Number.isSafeInteger(request.prior_native_evidence.job_id) && request.prior_native_evidence.job_id>0, 'Invalid native evidence IDs');
  check(Number.isSafeInteger(request.prior_native_evidence.run_attempt) && request.prior_native_evidence.run_attempt>0, 'Exact successful native attempt is required');
  check(/^[0-9a-f]{64}$/.test(request.prior_native_evidence.proof_json_sha256), 'Missing reviewed native report digest');
  keys(request.model_revisions,Object.keys(MODEL_REVISIONS));
  for (const [name, revision] of Object.entries(MODEL_REVISIONS)) check(request.model_revisions[name]===revision, 'Model revision changed');
  check(request.model_download_bytes===3720689099 && request.audio_sha256==='59dfb9a4acb36fe2a2affc14bacbee2920ff435cb13cc314a08c13f66ba7860e', 'Fixture identity changed');
  check(request.minimum_available_ram_bytes===13*1024**3 && request.minimum_free_disk_bytes===12*1024**3, 'Resource gates changed');
  check(request.inference_timeout_seconds===900 && request.download_deadline_seconds===600 && request.metadata_only===true, 'Proof bounds changed');
  return {source_sha:request.source_sha,pack_id:request.pack_id,selector:request.selector,runner:request.pack_id.endsWith('windows-x64')?'windows-2022':'macos-15'};
}
function validEvidence(request, run, job) {
  check(run.id===request.prior_native_evidence.run_id && run.run_attempt===request.prior_native_evidence.run_attempt && run.repository?.full_name===REPOSITORY && run.head_repository?.full_name===REPOSITORY, 'Wrong native run');
  check(run.pull_requests?.some(pr=>pr.number===4), 'Native evidence must belong to the reviewed PR');
  check(run.head_sha===request.source_sha && run.head_branch===BRANCH && run.event==='pull_request' && run.path==='.github/workflows/asr-components.yml' && run.status==='completed', 'Native run source/workflow differs');
  // A historical nested PR head is mutable. Only these immutable run/job SHAs
  // establish source identity; never use run.pull_requests[].head.sha.
  check(job.id===request.prior_native_evidence.job_id && job.run_id===run.id && job.run_attempt===request.prior_native_evidence.run_attempt && job.head_sha===request.source_sha, 'Native job source differs');
  check(job.name==='Build component '+request.pack_id && job.status==='completed' && job.conclusion==='success', 'Selected runtime has no successful native proof');
  const step=job.steps?.filter(s=>s.name==='Prove private offline-pip assembly without system Python packages');
  check(step?.length===1 && step[0].conclusion==='success', 'Selected native proof step did not pass');
}
async function publicJson(path) {
  const response=await fetch('https://api.github.com/repos/'+REPOSITORY+path, {redirect:'error',signal:AbortSignal.timeout(15000),headers:{Accept:'application/vnd.github+json','User-Agent':'luma-bounded-qwen-proof'}});
  check(response.status===200, 'Public native-provenance read failed: '+response.status);
  const chunks=[]; let size=0;
  for await (const chunk of response.body) { size+=chunk.byteLength; check(size<=1024*1024,'Oversized public native metadata'); chunks.push(Buffer.from(chunk)); }
  return JSON.parse(Buffer.concat(chunks).toString('utf8'));
}
async function main() {
  check(process.env.GITHUB_EVENT_NAME==='push','Only the request-only feature-branch push trigger is supported');
  // This one-time request is not automatically replayed. A separately reviewed
  // fresh request is needed after any failed model attempt. Native provenance
  // itself may reference any exact successful attempt, including a retry.
  check(process.env.GITHUB_RUN_ATTEMPT==='1','This one-time model request cannot be rerun');
  const event=JSON.parse(fs.readFileSync(process.env.GITHUB_EVENT_PATH,'utf8'));
  check(fs.lstatSync(REQUEST).isFile() && fs.statSync(REQUEST).size<=16384,'Oversized request');
  const bytes=fs.readFileSync(REQUEST);
  const request=JSON.parse(bytes.toString('utf8'));
  const git=(...args)=>execFileSync('git',args,{encoding:'utf8',maxBuffer:65536}).trim();
  const sha=git('rev-parse','HEAD');const parents=git('show','-s','--format=%P',sha).split(' ').filter(Boolean);
  const changed=git('diff-tree','--no-commit-id','--name-only','-r',sha).split('\n').filter(Boolean);
  check(/^[0-9a-f]{40}$/.test(request.source_sha), 'Source must be immutable');
  let requestExisted=false;
  try { git('cat-file','-e',request.source_sha+':'+REQUEST); requestExisted=true; } catch(error) { if(error.status!==128)throw error; }
  const entry=git('ls-tree',sha,'--',REQUEST);
  check(/^100644 blob [0-9a-f]{40}\t/.test(entry), 'Request must be a regular non-executable Git blob');
  const output=validRequest(request,event,{sha,parents,request_existed:requestExisted,request_mode:entry.slice(0,6)},changed);
  const run=await publicJson('/actions/runs/'+request.prior_native_evidence.run_id+'/attempts/'+request.prior_native_evidence.run_attempt);
  const job=await publicJson('/actions/jobs/'+request.prior_native_evidence.job_id);
  validEvidence(request,run,job);
  // The report digest is independently reviewed before creating the request.
  // This control verifies immutable public job metadata. Full inference reruns
  // native proof before weights; README-only mode cannot enter that path. Neither
  // selector claims to download the old report.
  fs.appendFileSync(process.env.GITHUB_OUTPUT,Object.entries(output).map(([k,v])=>k+'='+v+'\n').join(''));
  console.log(JSON.stringify({request_sha:sha,...output,prior_native_job_verified:true,prior_report_digest_reviewed:request.prior_native_evidence.proof_json_sha256,metadata_only:true}));
}
module.exports={REQUEST,REPOSITORY,BRANCH,MODEL_REVISIONS,README_REQUEST,validRequest,validEvidence};
if(require.main===module)main().catch(error=>{console.error(error.message);process.exitCode=1;});
