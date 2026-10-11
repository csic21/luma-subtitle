//! Do not move a selected user model/runtime into an engine-repair backup.
use std::path::{Component, Path, PathBuf};
use tauri::AppHandle;

pub(super) async fn ensure_llama_repair_preserves_selected_paths(
    app: &AppHandle,
    target: &Path,
) -> Result<(), String> {
    let app = app.clone();
    let target = target.to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        match std::fs::symlink_metadata(&target) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("无法检查现有翻译引擎目录: {error}")),
            Ok(_) => (),
        }
        let settings = crate::settings::task_settings_from_current(&app, None)?;
        let paths = [
            settings.translation_local_model_path,
            settings.whisper_model_path,
            settings.asr.model_path,
            settings.asr.aligner_path,
            settings.asr.python_path,
        ];
        reject_nested_selected_paths(&target, &paths)
    })
    .await
    .map_err(|error| format!("检查翻译引擎修复路径失败: {error}"))?
}

fn reject_nested_selected_paths(target: &Path, selected: &[String]) -> Result<(), String> {
    let lexical_root = lexical_absolute(target)?;
    let canonical_root = std::fs::canonicalize(target)
        .map_err(|error| format!("无法确认现有翻译引擎目录位置: {error}"))?;
    for value in selected.iter().filter(|value| !value.trim().is_empty()) {
        let original = Path::new(value.trim());
        let absolute = if original.is_absolute() { original.to_path_buf() }
            else { std::env::current_dir().map_err(|e| e.to_string())?.join(original) };
        let path = lexical_absolute(&absolute)?;
        // Check both spellings: a selected link inside the tree also moves, and
        // an outside alias can refer to a model physically inside the tree.
        let nested = same_or_nested(&path, &lexical_root)
            || same_or_nested(&canonical_existing_ancestor(&absolute)?, &canonical_root);
        if nested {
            return Err("所选模型或运行时位于 llama.cpp 引擎目录内。为保留现有路径，本次修复未移动该目录。请先将该模型或运行时移到单独目录并在设置中重新选择，再重试安装。".into());
        }
    }
    Ok(())
}

fn lexical_absolute(path: &Path) -> Result<PathBuf, String> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?.join(path)
    };
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => (),
            Component::ParentDir => { result.pop(); }
            other => result.push(other.as_os_str()),
        }
    }
    Ok(result)
}

fn canonical_existing_ancestor(path: &Path) -> Result<PathBuf, String> {
    let mut current = path.to_path_buf();
    let mut missing = Vec::new();
    loop {
        match std::fs::canonicalize(&current) {
            Ok(mut canonical) => {
                for part in missing.iter().rev() { canonical.push(part); }
                return Ok(canonical);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let part = current.file_name().ok_or("无法确认所选模型路径的位置")?.to_os_string();
                missing.push(part);
                if !current.pop() { return Err("无法确认所选模型路径的位置".into()); }
            }
            Err(error) => return Err(format!("无法安全检查所选模型路径: {error}")),
        }
    }
}

fn same_or_nested(path: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        let path: Vec<_> = path.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect();
        let root: Vec<_> = root.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect();
        path.starts_with(&root)
    }
    #[cfg(not(windows))]
    { path.starts_with(root) }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("luma-repair-path-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(root.join("llama.cpp")).unwrap();
            std::fs::create_dir_all(root.join("models")).unwrap();
            Self(root)
        }
    }
    impl Drop for Temp { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    #[test]
    fn separate_models_are_unchanged_and_nested_models_fail_before_move() {
        let temp = Temp::new();
        let engine = temp.0.join("llama.cpp");
        let separate = temp.0.join("models/user.gguf");
        let nested = engine.join("user.gguf");
        std::fs::write(&separate, b"separate user model").unwrap();
        std::fs::write(&nested, b"nested user model").unwrap();
        assert!(reject_nested_selected_paths(&engine, &[separate.to_string_lossy().into_owned()]).is_ok());
        assert!(reject_nested_selected_paths(&engine, &[nested.to_string_lossy().into_owned()]).is_err());
        assert!(reject_nested_selected_paths(&engine, &[engine.join("missing.gguf").to_string_lossy().into_owned()]).is_err());
        assert_eq!(std::fs::read(&separate).unwrap(), b"separate user model");
        assert_eq!(std::fs::read(&nested).unwrap(), b"nested user model");
    }
    #[cfg(unix)]
    #[test]
    fn outside_alias_and_inside_alias_both_preserve_selected_path() {
        let temp = Temp::new();
        let engine = temp.0.join("llama.cpp");
        std::fs::write(engine.join("model.gguf"), b"model").unwrap();
        std::os::unix::fs::symlink(&engine, temp.0.join("alias")).unwrap();
        std::fs::create_dir(engine.join("child")).unwrap();
        std::fs::create_dir(temp.0.join("outside")).unwrap();
        std::os::unix::fs::symlink(engine.join("child"), temp.0.join("outside/link")).unwrap();
        std::os::unix::fs::symlink(temp.0.join("models"), engine.join("alias")).unwrap();
        for path in [temp.0.join("alias/model.gguf"), engine.join("alias/missing.gguf"),
            temp.0.join("outside/link/../model.gguf")] {
            assert!(reject_nested_selected_paths(&engine, &[path.to_string_lossy().into_owned()]).is_err());
        }
    }
}
