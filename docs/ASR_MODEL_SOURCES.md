# Managed ASR model provenance

Verified 2026-10-09. The reviewed `src-tauri/resources/asr/catalog.json` is the
download allowlist compiled into Luma. Its `version` is the complete immutable
Hugging Face Git commit for each model. Every file has an HTTPS URL pinned to that
commit, its exact byte count, SHA-256, and destination relative to the model's
private installation directory. The catalog does not bundle these model weights.

## Trust and verification boundary

- The trust anchor is the catalog shipped in the reviewed/signed app. A downloaded
  manifest, including one that supplies its own hashes, must never replace it.
  Changing model or runtime pins requires review and a new app build.
- The public Hugging Face model API was queried, then queried again at each exact
  revision with `blobs=true`. All seven sources were public and ungated. No token,
  account, model Python code, or remote-code execution was used.
- Large weight sizes and SHA-256 values come from that pinned revision's Git LFS
  metadata. **No weight files were downloaded or independently rehashed during
  catalog preparation.** The installer must hash the actual downloaded bytes and
  check both size and SHA-256 before accepting them.
- Small configs, model cards, tokenizer files, and the shard index were fetched
  from immutable URLs. Each was checked against its Git blob SHA-1 and measured
  and hashed locally with SHA-256. Identical Git blobs were reused: 11,070,997
  unique bytes were inspected; the largest file was 2,776,833 bytes. Git blob IDs
  are provenance evidence, not substitutes for the catalog's SHA-256 checks.
- Paths are explicit regular files, without traversal, absolute paths, duplicate
  case-insensitive names, or model source scripts. Only the listed files may be
  installed. `.gitattributes` is deliberately omitted; model cards are retained.
- Verified metadata and layout do not establish inference quality, speed,
  hardware compatibility, memory fit, or native runtime readiness. The worker
  still enforces local-only loading and `trust_remote_code=False` for Qwen.

## Reviewed CDN redirects

Reviewed 2026-10-10 against Hugging Face's official
[download/firewall documentation](https://huggingface.co/docs/hub/models-downloading#downloading-behind-a-proxy-or-firewall)
and [published host metadata](https://huggingface.co/.well-known/meta.json).
Both identify `us.aws.cdn.hf.co` and `us.gcp.cdn.hf.co` as Hugging Face CDN hosts.
The app's model downloader and the bounded Qwen fixture add only those two exact
HTTPS redirect hosts; this does not trust all `hf.co` subdomains, arbitrary CDN
regions, or cloud-provider domains. Original catalog URLs must still be pinned
Hugging Face repository URLs, and exact byte/hash verification remains mandatory.
No model file, model revision, hash, byte count, or resource budget changes.

The previous Qwen attempt did not record its rejected hostname. This correction
addresses an independently documented compatibility gap, not a claim that either
new hostname was observed in that historical failure. Future fixture rejection
evidence may include only a validated lowercase ASCII hostname, capped at 253
characters, with no URL, path, query, userinfo, port, or response-header content.
An unparseable hostname is omitted. These diagnostics never authorize a host.

Both policies reject effective nonempty credentials, HTTP, custom ports and
lookalike hosts. Their URL parsers are not lexically identical: Rust's parsed URL
normalizes empty userinfo away and permits fragments that HTTP does not send;
Python conservatively rejects raw empty userinfo and fragments. Existing redirect
loop/depth limits are retained, including reqwest's cutoff at eight previous URLs
(the initial URL is included). No raw-Location driver is introduced.

## Pinned models

All byte totals include the downloaded model card and metadata. They are decimal
bytes on disk, not RAM/VRAM requirements, runtime sizes, or free-space estimates.
The catalog contains all per-file hashes and URLs; this table avoids duplicating
those security-sensitive values.

| Catalog ID | Source and immutable revision | Installed bytes | Source/license |
| --- | --- | ---: | --- |
| `faster-whisper-turbo` | [dropbox-dash/faster-whisper-large-v3-turbo](https://huggingface.co/dropbox-dash/faster-whisper-large-v3-turbo/tree/0a363e9161cbc7ed1431c9597a8ceaf0c4f78fcf) · `0a363e9161cbc7ed1431c9597a8ceaf0c4f78fcf` | 1,621,667,428 | Dropbox community conversion; card declares MIT |
| `faster-whisper-small` | [Systran/faster-whisper-small](https://huggingface.co/Systran/faster-whisper-small/tree/536b0662742c02347bc0e980a01041f333bce120) · `536b0662742c02347bc0e980a01041f333bce120` | 486,214,370 | SYSTRAN conversion; card declares MIT |
| `faster-whisper-tiny` | [Systran/faster-whisper-tiny](https://huggingface.co/Systran/faster-whisper-tiny/tree/d90ca5fe260221311c53c58e660288d3deb8d356) · `d90ca5fe260221311c53c58e660288d3deb8d356` | 78,205,610 | SYSTRAN conversion; card declares MIT |
| `mlx-whisper-turbo` | [mlx-community/whisper-turbo](https://huggingface.co/mlx-community/whisper-turbo/tree/ec8e501925a1e2e85b082740a208c37ff6c4f59a) · `ec8e501925a1e2e85b082740a208c37ff6c4f59a` | 1,614,089,389 | Community conversion; upstream Whisper MIT, conversion license not separately declared |
| `qwen3-asr-0-6b` | [Qwen/Qwen3-ASR-0.6B](https://huggingface.co/Qwen/Qwen3-ASR-0.6B/tree/5eb144179a02acc5e5ba31e748d22b0cf3e303b0) · `5eb144179a02acc5e5ba31e748d22b0cf3e303b0` | 1,880,618,159 | Original Qwen publisher; card declares Apache-2.0 |
| `qwen3-asr-1-7b` | [Qwen/Qwen3-ASR-1.7B](https://huggingface.co/Qwen/Qwen3-ASR-1.7B/tree/7278e1e70fe206f11671096ffdd38061171dd6e5) · `7278e1e70fe206f11671096ffdd38061171dd6e5` | 4,703,112,789 | Original Qwen publisher; card declares Apache-2.0 |
| `qwen3-forced-aligner-0-6b` | [Qwen/Qwen3-ForcedAligner-0.6B](https://huggingface.co/Qwen/Qwen3-ForcedAligner-0.6B/tree/c7cbfc2048c462b0d63a45797104fc9db3ad62b7) · `c7cbfc2048c462b0d63a45797104fc9db3ad62b7` | 1,840,070,940 | Original Qwen publisher; card declares Apache-2.0 |

### CTranslate2 Whisper

These are converted models, **not official OpenAI exports**. The pinned
[faster-whisper 1.2.1 model aliases](https://github.com/SYSTRAN/faster-whisper/blob/65882eee9f5cdbeeb2d877f1131d48cf241b327d/faster_whisper/utils.py)
name `mobiuslabsgmbh/faster-whisper-large-v3-turbo` for both `turbo` and
`large-v3-turbo`; that repository now redirects to the catalog's `dropbox-dash`
owner. Luma pins the resolved owner and immutable snapshot directly.

All three snapshots include `config.json`, `model.bin`, `tokenizer.json`, and a
vocabulary. Turbo also includes `preprocessor_config.json` and uses
`vocabulary.json`; Small/Tiny use `vocabulary.txt` and the runtime's default
feature-extraction configuration. Keeping the local tokenizer prevents an
implicit Hub fallback. These `.bin` files are not whisper.cpp GGML models.

### MLX Whisper

The community snapshot contains `config.json` and **`weights.npz`**, which match
the pinned `mlx-whisper==0.4.3` loader. `model.safetensors`-only conversions do not
match that release and are excluded. The config declares `n_mels=128`,
`n_audio_ctx=1500`, and `n_vocab=51866`.

The conversion card says it was converted from Whisper Turbo but does not declare
a license or include a license file. The original
[OpenAI Turbo model card](https://huggingface.co/openai/whisper-large-v3-turbo/blob/41f01f3fe87f28c78e2fbf8b568835947dd65ed9/README.md)
declares MIT, and [OpenAI's Whisper license](https://github.com/openai/whisper/blob/31243bad24cc746f07d4c8bfdd2d974872cb1803/LICENSE)
provides the copyright and license text. The catalog explicitly distinguishes
this upstream license evidence from the conversion publisher's missing
declaration. No independent conversion audit or OpenAI endorsement is claimed.

### Original Qwen3-ASR and forced alignment

These are the original `qwen-asr` models, not native Transformers `-hf` exports.
All three configs declare `model_type=qwen3_asr`; only the aligner has
`timestamp_token_id` and `timestamp_segment_time`. ASR and aligner directories
remain separate and are not interchangeable.

Each includes the original `config.json`, `preprocessor_config.json`,
`tokenizer_config.json`, `vocab.json`, `merges.txt`, `chat_template.json`,
`generation_config.json`, and model card. The 0.6B ASR and aligner each have one
`model.safetensors`. The 1.7B index was inspected: both
`model-00001-of-00002.safetensors` and `model-00002-of-00002.safetensors` are listed
in the catalog, along with `model.safetensors.index.json`. There are no omitted
weight shards or required external tokenizer files.

Both ASR choices require the same separate ForcedAligner-0.6B for timed subtitles.
Their combined installed model sizes are 3,720,689,099 bytes (0.6B pair) and
6,543,183,729 bytes (1.7B pair). CPU loading uses float32 for both models, so these
storage sizes substantially understate memory demand. Refer to
[optional engine limitations](OPTIONAL_ASR.md) before selecting Qwen, especially
on an 8 GB computer. The publisher's cards declare
[Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0).

## Native runtime publication

The initial catalog intentionally has four unavailable runtime entries, with no
archive URL or fabricated hash/size. A model pin does not make those runtimes
ready. Only after a native Windows x64 or macOS arm64 build passes its checks may
the release process add that archive's measured SHA-256, byte count, extraction
bounds, and entrypoint, and remove its unavailable reason. Runtime version
`1.2.0-r1` targets the separate `asr-components-1.2.0-r1` prerelease; it must not
replace a previously published asset in place. Until publication, the planned
runtime license/provenance link may not resolve.

Runtime dependencies carry multiple licenses; the exact package inventory and
preserved notices must accompany each verified archive. Model and runtime
installation remains explicit and optional. The ordinary offline worker never
refreshes this catalog, downloads a model, installs packages, or sends user audio
to Hugging Face or another service.
