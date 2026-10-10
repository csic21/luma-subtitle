use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::Duration,
};
use tauri::AppHandle;

use crate::{
    job_events::{publish_job_event, JobEventDraft},
    state::{ensure_not_cancelled, JobError, JobResult},
    subtitles::{SubtitleSegment, TranslatedSegment},
};

pub(crate) mod checkpoint;
pub(crate) mod cli;
mod client;
pub(crate) mod local;
mod parser;
mod opencode_isolation;
mod prompt;
mod runtime;

#[cfg(test)]
pub(crate) use client::chat_endpoint;
use client::translate_shard_once;
#[cfg(test)]
pub(crate) use parser::{attach_model_output, parse_translation_content};

#[derive(Clone)]
pub(crate) struct TranslationConfig {
    pub(crate) target_language: String,
    pub(crate) base_url: String,
    pub(crate) base_url_is_complete: bool,
    pub(crate) model: String,
    pub(crate) temperature: f32,
    pub(crate) shard_size: usize,
    pub(crate) provider: String,
    pub(crate) cli_tool: String,
    pub(crate) cli_command: String,
    pub(crate) cli_model: String,
    pub(crate) cli_args: String,
    pub(crate) local_model_path: String,
}

pub(crate) type TranslationProgressHook =
    Arc<dyn Fn(&[TranslatedSegment]) -> JobResult<()> + Send + Sync>;

#[derive(Clone)]
pub(crate) struct TranslationResume {
    pub(crate) completed: Vec<TranslatedSegment>,
    pub(crate) checkpoint_path: PathBuf,
    pub(crate) source_fingerprint: String,
    pub(crate) on_progress: Option<TranslationProgressHook>,
}

pub(crate) const DEFAULT_TRANSLATION_SHARD_SIZE: usize = 200;
pub(crate) const MIN_TRANSLATION_SHARD_SIZE: usize = 1;
pub(crate) const MAX_TRANSLATION_SHARD_SIZE: usize = 1_000;
const MAX_CONCURRENT_SHARDS: usize = 4;
const MAX_CONCURRENT_CLI_SHARDS: usize = 2;

pub(crate) const DEFAULT_TRANSLATION_PROVIDER: &str = "api";
pub(crate) const DEFAULT_TRANSLATION_CLI_TOOL: &str = "opencode";
pub(crate) const DEFAULT_TRANSLATION_CLI_COMMAND: &str = "opencode";
pub(crate) const DEFAULT_LOCAL_TRANSLATION_SHARD_SIZE: usize = 12;
pub(crate) const MAX_LOCAL_TRANSLATION_SHARD_SIZE: usize = 16;

pub(crate) fn normalize_translation_provider(provider: &str) -> String {
    match provider.trim().to_lowercase().as_str() {
        "cli" | "opencode" | "opencode-cli" | "command" | "custom" => "cli".to_string(),
        "local" | "llama" | "llamacpp" | "llama.cpp" | "gguf" | "hy-mt2" => "local".to_string(),
        _ => "api".to_string(),
    }
}

pub(crate) fn is_cli_provider(provider: &str) -> bool {
    normalize_translation_provider(provider) == "cli"
}

pub(crate) fn is_local_provider(provider: &str) -> bool {
    normalize_translation_provider(provider) == "local"
}

pub(crate) fn is_api_provider(provider: &str) -> bool {
    normalize_translation_provider(provider) == "api"
}

pub(crate) fn normalize_translation_local_model_path(path: &str) -> String {
    path.trim().to_string()
}

pub(crate) fn normalize_translation_cli_tool(tool: &str) -> String {
    match tool.trim().to_lowercase().as_str() {
        "custom" | "generic" | "command" => "custom".to_string(),
        _ => "opencode".to_string(),
    }
}

pub(crate) fn normalize_translation_cli_command(command: &str) -> String {
    command.trim().to_string()
}

pub(crate) fn normalize_translation_cli_model(model: &str) -> String {
    model.trim().to_string()
}

pub(crate) fn normalize_translation_cli_args(args: &str) -> String {
    args.trim().to_string()
}

pub(crate) fn normalize_translation_shard_size(size: usize) -> usize {
    size.clamp(MIN_TRANSLATION_SHARD_SIZE, MAX_TRANSLATION_SHARD_SIZE)
}

pub(crate) async fn translate_with_single_request(
    app: &AppHandle,
    job_id: &str,
    config: &TranslationConfig,
    api_key: Option<&str>,
    segments: &[SubtitleSegment],
    _source_srt: &str,
    _source_file_name: &str,
    cancel: Arc<AtomicBool>,
    resume: Option<TranslationResume>,
) -> JobResult<Vec<TranslatedSegment>> {
    ensure_not_cancelled(&cancel)?;

    let progress = TranslationProgress::new(config, segments, resume)?;
    let remaining = progress.remaining_owned();
    let kept = progress.completed_count();
    let shard_size = normalize_translation_shard_size(config.shard_size);

    if is_local_provider(&config.provider) {
        let local_shard_size = local::normalize_local_shard_size(shard_size);
        publish_job_event(
            app,
            JobEventDraft::running(
                job_id,
                "translate-shards",
                if kept > 0 {
                    format!(
                        "续翻：已保留 {kept} 条，本地 Hy-MT2 继续翻译剩余 {} 条（每片 {local_shard_size}）",
                        remaining.len()
                    )
                } else {
                    format!("正在用本地 Hy-MT2 模型按每片 {local_shard_size} 条字幕翻译")
                },
                cue_progress(kept, segments.len()),
            ),
        );
        return local::translate_shards_via_local(app, job_id, config, segments, cancel, progress)
            .await;
    }
    if is_cli_provider(&config.provider) {
        cli::validate_cli_config(config)?;
        publish_job_event(
            app,
            JobEventDraft::running(
                job_id,
                "translate-shards",
                if kept > 0 {
                    format!(
                        "续翻：已保留 {kept} 条，CLI（{}）继续翻译剩余 {} 条（每片 {shard_size}，并发 {MAX_CONCURRENT_CLI_SHARDS}）",
                        cli::cli_display_name(config),
                        remaining.len()
                    )
                } else {
                    format!(
                        "正在用 CLI（{}）按每片 {shard_size} 条字幕分片翻译（并发 {MAX_CONCURRENT_CLI_SHARDS}）",
                        cli::cli_display_name(config),
                    )
                },
                cue_progress(kept, segments.len()),
            ),
        );
        return translate_shards_via_cli(app, job_id, config, segments, cancel, progress).await;
    }

    let api_key = api_key
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| JobError::failed("请先保存 OpenAI 兼容接口的 API Key"))?;
    let client = reqwest::Client::builder()
        // Never forward subtitle text or credentials to a redirected origin.
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(240))
        .build()
        .map_err(|error| JobError::failed(format!("创建 HTTP 客户端失败: {error}")))?;

    publish_job_event(
        app,
        JobEventDraft::running(
            job_id,
            "translate-shards",
            if kept > 0 {
                format!(
                    "续翻：已保留 {kept} 条，继续翻译剩余 {} 条（每片 {shard_size}，并发 {MAX_CONCURRENT_SHARDS}）",
                    remaining.len()
                )
            } else {
                format!("正在按每片 {shard_size} 条字幕分片翻译（并发 {MAX_CONCURRENT_SHARDS}）")
            },
            cue_progress(kept, segments.len()),
        ),
    );

    translate_shards(
        app, job_id, &client, config, api_key, segments, cancel, progress,
    )
    .await
}

async fn translate_shards_via_cli(
    app: &AppHandle,
    job_id: &str,
    config: &TranslationConfig,
    all_segments: &[SubtitleSegment],
    cancel: Arc<AtomicBool>,
    progress: TranslationProgress,
) -> JobResult<Vec<TranslatedSegment>> {
    let shard_size = normalize_translation_shard_size(config.shard_size);
    let remaining = progress.remaining_owned();
    if remaining.is_empty() {
        return Ok(progress.into_completed());
    }
    let shards = remaining
        .chunks(shard_size)
        .map(|chunk| chunk.to_vec())
        .collect::<Vec<_>>();
    let total_shards = shards.len().max(1);
    let total_cues = all_segments.len();

    runtime::run_bounded(
        shards.into_iter().enumerate(),
        MAX_CONCURRENT_CLI_SHARDS,
        cancel,
        |(index, shard), stop| {
            let shard_index = index + 1;
            publish_shard_submitted(
                app,
                job_id,
                shard_index,
                total_shards,
                shard.len(),
                cue_progress(progress.completed_count(), total_cues),
                " CLI",
            );
            async move {
                cli::translate_shard_via_cli(config, &shard, shard_index, total_shards, stop)
                    .await
                    .map(|items| (shard_index, items))
                    .map_err(|error| prefix_shard_error(error, shard_index, total_shards))
            }
        },
        |(shard_index, items)| {
            progress.append_and_persist(items)?;
            publish_shard_completed(
                app,
                job_id,
                shard_index,
                total_shards,
                progress.completed_count(),
                total_cues,
            );
            Ok(())
        },
    )
    .await?;

    Ok(progress.into_completed())
}

async fn translate_shards(
    app: &AppHandle,
    job_id: &str,
    client: &reqwest::Client,
    config: &TranslationConfig,
    api_key: &str,
    all_segments: &[SubtitleSegment],
    cancel: Arc<AtomicBool>,
    progress: TranslationProgress,
) -> JobResult<Vec<TranslatedSegment>> {
    let shard_size = normalize_translation_shard_size(config.shard_size);
    let remaining = progress.remaining_owned();
    if remaining.is_empty() {
        return Ok(progress.into_completed());
    }
    let shards = remaining
        .chunks(shard_size)
        .map(|chunk| chunk.to_vec())
        .collect::<Vec<_>>();
    let total_shards = shards.len().max(1);
    let total_cues = all_segments.len();

    runtime::run_bounded(
        shards.into_iter().enumerate(),
        MAX_CONCURRENT_SHARDS,
        cancel,
        |(index, shard), stop| {
            let shard_index = index + 1;
            publish_shard_submitted(
                app,
                job_id,
                shard_index,
                total_shards,
                shard.len(),
                cue_progress(progress.completed_count(), total_cues),
                "",
            );
            async move {
                translate_shard_once(
                    client,
                    config,
                    api_key,
                    &shard,
                    shard_index,
                    total_shards,
                    &stop,
                )
                .await
                .map(|items| (shard_index, items))
                .map_err(|error| prefix_shard_error(error, shard_index, total_shards))
            }
        },
        |(shard_index, items)| {
            progress.append_and_persist(items)?;
            publish_shard_completed(
                app,
                job_id,
                shard_index,
                total_shards,
                progress.completed_count(),
                total_cues,
            );
            Ok(())
        },
    )
    .await?;

    Ok(progress.into_completed())
}

fn publish_shard_submitted(
    app: &AppHandle,
    job_id: &str,
    index: usize,
    total: usize,
    cues: usize,
    progress: f32,
    provider: &str,
) {
    publish_job_event(
        app,
        JobEventDraft::running(
            job_id,
            "translate-shard",
            format!("分片 {index}/{total} 已提交{provider}（{cues} 条字幕）"),
            progress,
        ),
    );
}

fn publish_shard_completed(
    app: &AppHandle,
    job_id: &str,
    index: usize,
    total: usize,
    completed: usize,
    total_cues: usize,
) {
    publish_job_event(
        app,
        JobEventDraft::running(
            job_id,
            "translate-shard",
            format!("分片 {index}/{total} 已完成（累计 {completed}/{total_cues}）"),
            cue_progress(completed, total_cues),
        ),
    );
}

pub(crate) struct TranslationProgress {
    target_language: String,
    source_fingerprint: String,
    checkpoint_path: PathBuf,
    all_segments: Vec<SubtitleSegment>,
    completed: Mutex<Vec<TranslatedSegment>>,
    on_progress: Option<TranslationProgressHook>,
}

impl TranslationProgress {
    pub(crate) fn new(
        config: &TranslationConfig,
        segments: &[SubtitleSegment],
        resume: Option<TranslationResume>,
    ) -> JobResult<Self> {
        parser::validate_source_ids(segments)?;
        let (checkpoint_path, source_fingerprint, completed, on_progress) =
            if let Some(resume) = resume {
                (
                    resume.checkpoint_path,
                    resume.source_fingerprint,
                    resume.completed,
                    resume.on_progress,
                )
            } else {
                (
                    PathBuf::new(),
                    checkpoint::source_fingerprint(segments),
                    Vec::new(),
                    None,
                )
            };
        Ok(Self {
            target_language: config.target_language.clone(),
            source_fingerprint,
            checkpoint_path,
            all_segments: segments.to_vec(),
            completed: Mutex::new(completed),
            on_progress,
        })
    }

    pub(crate) fn completed_count(&self) -> usize {
        self.completed.lock().map(|items| items.len()).unwrap_or(0)
    }

    pub(crate) fn remaining_owned(&self) -> Vec<SubtitleSegment> {
        let completed = self
            .completed
            .lock()
            .map(|items| items.clone())
            .unwrap_or_default();
        checkpoint::remaining_segments(&self.all_segments, &completed)
            .into_iter()
            .cloned()
            .collect()
    }

    pub(crate) fn append_and_persist(&self, newly: Vec<TranslatedSegment>) -> JobResult<()> {
        let snapshot = {
            let mut completed = self
                .completed
                .lock()
                .map_err(|_| JobError::failed("翻译进度锁损坏"))?;
            let merged = checkpoint::merge_translations(&completed, &newly);
            *completed = merged.clone();
            merged
        };
        if !self.checkpoint_path.as_os_str().is_empty() {
            checkpoint::save_checkpoint(
                &self.checkpoint_path,
                &self.target_language,
                &self.source_fingerprint,
                &snapshot,
            )?;
        }
        if let Some(on_progress) = &self.on_progress {
            on_progress(&snapshot)?;
        }
        Ok(())
    }

    pub(crate) fn into_completed(self) -> Vec<TranslatedSegment> {
        self.completed.into_inner().unwrap_or_default()
    }
}

fn prefix_shard_error(error: JobError, shard_index: usize, total_shards: usize) -> JobError {
    match error {
        JobError::Cancelled => JobError::Cancelled,
        JobError::Failed(message) => {
            JobError::failed(format!("分片 {shard_index}/{total_shards} 失败: {message}"))
        }
    }
}

pub(crate) fn cue_progress(completed_cues: usize, total_cues: usize) -> f32 {
    0.58 + (completed_cues as f32 / total_cues.max(1) as f32) * 0.36
}
