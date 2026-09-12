use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
use tauri::AppHandle;

use crate::{
    job_events::{publish_job_event, JobEventDraft},
    state::{ensure_not_cancelled, JobError, JobResult},
    subtitles::{SubtitleSegment, TranslatedSegment},
};

pub(crate) mod cli;
mod client;
mod parser;
mod prompt;

#[cfg(test)]
pub(crate) use cli::{extract_opencode_text_output, split_cli_args};
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
}

pub(crate) const DEFAULT_TRANSLATION_SHARD_SIZE: usize = 200;
pub(crate) const MIN_TRANSLATION_SHARD_SIZE: usize = 1;
pub(crate) const MAX_TRANSLATION_SHARD_SIZE: usize = 1_000;
const MAX_CONCURRENT_SHARDS: usize = 4;
const MAX_CONCURRENT_CLI_SHARDS: usize = 2;

pub(crate) const DEFAULT_TRANSLATION_PROVIDER: &str = "api";
pub(crate) const DEFAULT_TRANSLATION_CLI_TOOL: &str = "opencode";
pub(crate) const DEFAULT_TRANSLATION_CLI_COMMAND: &str = "opencode";

pub(crate) fn normalize_translation_provider(provider: &str) -> String {
    match provider.trim().to_lowercase().as_str() {
        "cli" | "opencode" | "opencode-cli" | "command" | "custom" => "cli".to_string(),
        _ => "api".to_string(),
    }
}

pub(crate) fn is_cli_provider(provider: &str) -> bool {
    normalize_translation_provider(provider) == "cli"
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
) -> JobResult<Vec<TranslatedSegment>> {
    ensure_not_cancelled(&cancel)?;

    let shard_size = normalize_translation_shard_size(config.shard_size);
    if is_cli_provider(&config.provider) {
        cli::validate_cli_config(config)?;
        publish_job_event(
            app,
            JobEventDraft::running(
                job_id,
                "translate-shards",
                format!(
                    "正在用 CLI（{}）按每片 {shard_size} 条字幕分片翻译（并发 {MAX_CONCURRENT_CLI_SHARDS}）",
                    cli::cli_display_name(config),
                ),
                0.58,
            ),
        );
        return translate_shards_via_cli(app, job_id, config, segments, cancel).await;
    }

    let api_key = api_key
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| JobError::failed("请先保存 OpenAI 兼容接口的 API Key"))?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(240))
        .build()
        .map_err(|error| JobError::failed(format!("创建 HTTP 客户端失败: {error}")))?;

    publish_job_event(
        app,
        JobEventDraft::running(
            job_id,
            "translate-shards",
            format!("正在按每片 {shard_size} 条字幕分片翻译（并发 {MAX_CONCURRENT_SHARDS}）"),
            0.58,
        ),
    );

    translate_shards(app, job_id, &client, config, api_key, segments, cancel).await
}

async fn translate_shards_via_cli(
    app: &AppHandle,
    job_id: &str,
    config: &TranslationConfig,
    segments: &[SubtitleSegment],
    cancel: Arc<AtomicBool>,
) -> JobResult<Vec<TranslatedSegment>> {
    let shard_size = normalize_translation_shard_size(config.shard_size);
    let shards = segments
        .chunks(shard_size)
        .map(|chunk| chunk.to_vec())
        .collect::<Vec<_>>();
    let total_shards = shards.len().max(1);
    let mut translated = Vec::with_capacity(segments.len());

    for (group_index, group) in shards.chunks(MAX_CONCURRENT_CLI_SHARDS).enumerate() {
        ensure_not_cancelled(&cancel)?;
        let mut handles = Vec::with_capacity(group.len());
        for (offset, shard) in group.iter().cloned().enumerate() {
            let shard_index = group_index * MAX_CONCURRENT_CLI_SHARDS + offset + 1;
            let progress = shard_progress(shard_index.saturating_sub(1), total_shards);
            publish_job_event(
                app,
                JobEventDraft::running(
                    job_id,
                    "translate-shard",
                    format!(
                        "分片 {shard_index}/{total_shards} 已提交 CLI（{} 条字幕）",
                        shard.len()
                    ),
                    progress,
                ),
            );

            let config = (*config).clone();
            let cancel = cancel.clone();
            handles.push(tauri::async_runtime::spawn(async move {
                cli::translate_shard_via_cli(&config, &shard, shard_index, total_shards, cancel)
                    .await
                    .map(|items| (shard_index, items))
                    .map_err(|error| prefix_shard_error(error, shard_index, total_shards))
            }));
        }

        for handle in handles {
            ensure_not_cancelled(&cancel)?;
            let (shard_index, mut items) = handle
                .await
                .map_err(|error| JobError::failed(format!("翻译分片任务失败: {error}")))??;
            translated.append(&mut items);
            publish_job_event(
                app,
                JobEventDraft::running(
                    job_id,
                    "translate-shard",
                    format!("分片 {shard_index}/{total_shards} 已完成"),
                    shard_progress(shard_index, total_shards),
                ),
            );
        }
    }

    translated.sort_by_key(|item| item.id);
    Ok(translated)
}

async fn translate_shards(
    app: &AppHandle,
    job_id: &str,
    client: &reqwest::Client,
    config: &TranslationConfig,
    api_key: &str,
    segments: &[SubtitleSegment],
    cancel: Arc<AtomicBool>,
) -> JobResult<Vec<TranslatedSegment>> {
    let shard_size = normalize_translation_shard_size(config.shard_size);
    let shards = segments
        .chunks(shard_size)
        .map(|chunk| chunk.to_vec())
        .collect::<Vec<_>>();
    let total_shards = shards.len().max(1);
    let mut translated = Vec::with_capacity(segments.len());

    for (group_index, group) in shards.chunks(MAX_CONCURRENT_SHARDS).enumerate() {
        ensure_not_cancelled(&cancel)?;
        let mut handles = Vec::with_capacity(group.len());
        for (offset, shard) in group.iter().cloned().enumerate() {
            let shard_index = group_index * MAX_CONCURRENT_SHARDS + offset + 1;
            let progress = shard_progress(shard_index.saturating_sub(1), total_shards);
            publish_job_event(
                app,
                JobEventDraft::running(
                    job_id,
                    "translate-shard",
                    format!(
                        "分片 {shard_index}/{total_shards} 已提交（{} 条字幕）",
                        shard.len()
                    ),
                    progress,
                ),
            );

            let client = client.clone();
            let config = (*config).clone();
            let api_key = api_key.to_string();
            handles.push(tauri::async_runtime::spawn(async move {
                translate_shard_once(
                    &client,
                    &config,
                    &api_key,
                    &shard,
                    shard_index,
                    total_shards,
                )
                .await
                .map(|items| (shard_index, items))
                .map_err(|error| prefix_shard_error(error, shard_index, total_shards))
            }));
        }

        for handle in handles {
            ensure_not_cancelled(&cancel)?;
            let (shard_index, mut items) = handle
                .await
                .map_err(|error| JobError::failed(format!("翻译分片任务失败: {error}")))??;
            translated.append(&mut items);
            publish_job_event(
                app,
                JobEventDraft::running(
                    job_id,
                    "translate-shard",
                    format!("分片 {shard_index}/{total_shards} 已完成"),
                    shard_progress(shard_index, total_shards),
                ),
            );
        }
    }

    translated.sort_by_key(|item| item.id);
    Ok(translated)
}

fn prefix_shard_error(error: JobError, shard_index: usize, total_shards: usize) -> JobError {
    match error {
        JobError::Cancelled => JobError::Cancelled,
        JobError::Failed(message) => {
            JobError::failed(format!("分片 {shard_index}/{total_shards} 失败: {message}"))
        }
    }
}

fn shard_progress(completed_shards: usize, total_shards: usize) -> f32 {
    0.58 + (completed_shards as f32 / total_shards.max(1) as f32) * 0.36
}
