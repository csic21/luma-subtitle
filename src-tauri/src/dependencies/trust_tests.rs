//! Network-free tests for the production legacy trust boundary.
use super::{archive, download, install, trust};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("luma-legacy-trust-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn sample() -> trust::Artifact {
    let mut item = trust::artifact("tiny").unwrap();
    item.bytes = 3;
    item.sha256 = format!("{:x}", Sha256::digest(b"abc"));
    item.unpacked_bytes = 1024;
    item.file_limit = 10;
    item
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn embedded_legacy_artifacts_are_complete_and_immutable() {
    let artifacts = trust::catalog().unwrap();
    let mut ids = std::collections::HashSet::new();
    assert_eq!(artifacts.len(), 19);
    for item in &artifacts {
        trust::validate_artifact(item).unwrap();
        assert!(ids.insert(&item.id));
        assert!(!item.url.contains("/latest"));
        assert!(!item.url.contains("/main/"));
        if item.source != trust::Source::Model {
            assert!(item.unpacked_bytes > 0 && item.file_limit > 0);
        }
    }
    for preset in super::WHISPER_MODEL_PRESETS {
        assert_eq!(
            trust::artifact(preset.id).unwrap().file_name,
            preset.file_name
        );
    }
    for preset in super::TRANSLATION_MODEL_PRESETS {
        assert_eq!(
            trust::artifact(preset.id).unwrap().file_name,
            preset.file_name
        );
    }
    assert_eq!(
        trust::artifact("vad").unwrap().file_name,
        super::WHISPER_VAD_MODEL_FILE_NAME
    );
}
#[test]
fn trust_rejects_unpinned_origins_versions_and_sizes() {
    for url in [
        "http://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
        "https://huggingface.co.evil.test/model",
        "https://user:pass@huggingface.co/model",
        "https://huggingface.co:444/model",
    ] {
        let mut item = sample();
        item.url = url.into();
        assert!(trust::validate_artifact(&item).is_err());
    }
    let mut item = sample();
    item.sha256 = "0".repeat(63);
    assert!(trust::validate_artifact(&item).is_err());
    item = sample();
    item.bytes = 0;
    assert!(trust::validate_artifact(&item).is_err());
    item = trust::artifact("whisper-cpu").unwrap();
    item.version = "latest".into();
    assert!(trust::validate_artifact(&item).is_err());
}
#[test]
fn redirects_require_bounded_https_origin_allowlist() {
    for raw in [
        "http://github.com/a",
        "https://github.com.evil.test/a",
        "https://github.com:8443/a",
        "https://u:p@github.com/a",
        "https://github.com/a#fragment",
        "https://127.0.0.1/a",
    ] {
        assert!(!trust::allowed_redirect(
            &url::Url::parse(raw).unwrap(),
            trust::Source::Github,
            1
        ));
    }
    assert!(trust::allowed_redirect(
        &url::Url::parse("https://release-assets.githubusercontent.com/a?signature=public-fixture")
            .unwrap(),
        trust::Source::Github,
        1
    ));
    assert!(!trust::allowed_redirect(
        &url::Url::parse("https://github.com/a").unwrap(),
        trust::Source::Github,
        8
    ));
    for host in [
        "huggingface.co",
        "us.aws.cdn.hf.co",
        "cas-bridge.xethub.hf.co",
    ] {
        assert!(trust::allowed_redirect(
            &url::Url::parse(&format!("https://{host}/a")).unwrap(),
            trust::Source::Model,
            1
        ));
    }
    assert!(!trust::allowed_redirect(
        &url::Url::parse("https://cas-bridge.xethub.hf.co.evil.test/a").unwrap(),
        trust::Source::Model,
        1
    ));
    assert!(!trust::allowed_redirect(
        &url::Url::parse("https://github.com/a").unwrap(),
        trust::Source::Model,
        1
    ));
}
#[test]
fn full_and_resumed_response_contract_is_exact() {
    assert_eq!(
        download::validate_response(200, 0, 10, None, Some(10), None),
        Ok(false)
    );
    assert_eq!(
        download::validate_response(200, 4, 10, None, Some(10), None),
        Ok(false)
    );
    assert_eq!(
        download::validate_response(206, 4, 10, Some("bytes 4-9/10"), Some(6), Some("identity")),
        Ok(true)
    );
    for range in [
        "bytes 0-5/10",
        "bytes 4-8/10",
        "bytes 4-9/11",
        "bytes 4-9/*",
        "garbage",
    ] {
        assert!(download::validate_response(206, 4, 10, Some(range), Some(6), None).is_err());
    }
    for status in [201, 204, 301, 404, 416] {
        assert!(download::validate_response(status, 10, 10, None, Some(0), None).is_err());
    }
    assert!(download::validate_response(206, 0, 10, Some("bytes 0-9/10"), Some(10), None).is_err());
    assert!(download::validate_response(206, 4, 10, None, Some(6), None).is_err());
    assert!(download::validate_response(206, 4, 10, Some("bytes 4-9/10"), Some(7), None).is_err());
    assert!(download::validate_response(200, 0, 10, None, Some(9), None).is_err());
    assert!(download::validate_response(200, 0, 10, None, Some(10), Some("gzip")).is_err());
}
#[test]
fn integrity_rejects_tampering_truncation_and_overflow() {
    let item = sample();
    assert!(trust::verify_digest(3, &item.sha256, &item).is_ok());
    for n in [0, 2, 4, u64::MAX] {
        assert!(trust::verify_digest(n, &item.sha256, &item).is_err());
    }
    assert!(trust::verify_digest(3, &"0".repeat(64), &item).is_err());
}
#[test]
fn existing_model_is_rehashed_including_resumed_prefix() {
    let temp = Temp::new();
    let path = temp.0.join("model.part");
    let item = sample();
    fs::write(&path, b"abc").unwrap();
    assert!(runtime().block_on(trust::verify_file(&path, &item)).is_ok());
    fs::write(&path, b"abd").unwrap();
    assert!(runtime()
        .block_on(trust::verify_file(&path, &item))
        .is_err());
    fs::write(&path, b"ab").unwrap();
    assert!(runtime()
        .block_on(trust::verify_file(&path, &item))
        .is_err());
    fs::write(&path, b"abcd").unwrap();
    assert!(runtime()
        .block_on(trust::verify_file(&path, &item))
        .is_err());
}
#[test]
fn complete_verified_cache_returns_without_network() {
    let temp = Temp::new();
    let path = temp.0.join("model.part");
    fs::write(&path, b"abc").unwrap();
    runtime()
        .block_on(download::download_file_with_resume(
            &sample(),
            &path,
            1.0,
            |_| (),
            |_, _, _| panic!("must not retry"),
        ))
        .unwrap();
}
#[cfg(unix)]
#[test]
fn cached_symlink_is_not_a_trusted_file() {
    let temp = Temp::new();
    fs::write(temp.0.join("original"), b"abc").unwrap();
    std::os::unix::fs::symlink(temp.0.join("original"), temp.0.join("link")).unwrap();
    assert!(runtime()
        .block_on(trust::verify_file(&temp.0.join("link"), &sample()))
        .is_err());
}
fn zip(path: &Path, names: &[&str]) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for name in names {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"abc").unwrap();
    }
    zip.finish().unwrap();
}
#[test]
fn legacy_zip_rejects_traversal_aliases_and_oversize_before_activation() {
    for names in [
        vec!["../escape"],
        vec!["/absolute"],
        vec!["C:/escape"],
        vec!["A.dll", "a.dll"],
        vec!["CON.txt"],
    ] {
        let temp = Temp::new();
        let archive = temp.0.join("input.zip");
        let out = temp.0.join("out");
        fs::create_dir(&out).unwrap();
        zip(&archive, &names);
        assert!(install::extract_zip_into_dir(&archive, &out, &sample()).is_err());
        assert_eq!(fs::read_dir(&out).unwrap().count(), 0);
    }
    let temp = Temp::new();
    let path = temp.0.join("input.zip");
    let out = temp.0.join("out");
    fs::create_dir(&out).unwrap();
    zip(&path, &["file"]);
    let mut item = sample();
    item.unpacked_bytes = 2;
    assert!(install::extract_zip_into_dir(&path, &out, &item).is_err());
}
fn tar(path: &Path, members: &[(&str, u8, &str, &[u8])]) {
    let encoder =
        flate2::write::GzEncoder::new(fs::File::create(path).unwrap(), flate2::Compression::fast());
    let mut archive = tar::Builder::new(encoder);
    for (name, kind, link, bytes) in members {
        let mut header = tar::Header::new_ustar();
        header.set_mode(0o755);
        header.set_entry_type(tar::EntryType::new(*kind));
        header.set_size(bytes.len() as u64);
        // Raw path bytes permit deliberate malformed fixtures that set_path rejects.
        header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
        if !link.is_empty() {
            header.set_link_name(link).unwrap();
        }
        header.set_cksum();
        archive.append(&header, Cursor::new(bytes)).unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap();
}
#[test]
fn macos_tar_materializes_internal_dylib_links_without_filesystem_links() {
    let temp = Temp::new();
    let path = temp.0.join("input.tar.gz");
    let out = temp.0.join("out");
    fs::create_dir(&out).unwrap();
    tar(
        &path,
        &[
            ("lib/real.dylib", b'0', "", b"abc"),
            ("lib/v1.dylib", b'2', "real.dylib", b""),
            ("lib/lib.dylib", b'2', "v1.dylib", b""),
        ],
    );
    archive::extract_tar(&path, &out, &sample()).unwrap();
    assert_eq!(fs::read(out.join("lib/lib.dylib")).unwrap(), b"abc");
    assert!(!fs::symlink_metadata(out.join("lib/lib.dylib"))
        .unwrap()
        .file_type()
        .is_symlink());
}
#[test]
fn tar_rejects_paths_links_cycles_extensions_and_bombs() {
    let fixtures: Vec<Vec<(&str, u8, &str, &[u8])>> = vec![
        vec![("../escape", b'0', "", b"abc")],
        vec![("x", b'2', "../../escape", b"")],
        vec![("x", b'2', "/absolute", b"")],
        vec![("x", b'2', "y", b""), ("y", b'2', "x", b"")],
        vec![("x", b'2', "missing", b"")],
        vec![("x", b'3', "", b"")],
        vec![("x", b'L', "", b"abc")],
        vec![("x", b'g', "", b"abc")],
        vec![("A", b'0', "", b"abc"), ("a", b'0', "", b"abc")],
    ];
    for members in fixtures {
        let temp = Temp::new();
        let path = temp.0.join("input.tar.gz");
        let out = temp.0.join("out");
        fs::create_dir(&out).unwrap();
        tar(&path, &members);
        assert!(
            archive::extract_tar(&path, &out, &sample()).is_err(),
            "accepted {:?}",
            members
        );
    }
    let temp = Temp::new();
    let path = temp.0.join("input.tar.gz");
    let out = temp.0.join("out");
    fs::create_dir(&out).unwrap();
    tar(&path, &[("file", b'0', "", b"abc")]);
    let mut item = sample();
    item.unpacked_bytes = 2;
    assert!(archive::extract_tar(&path, &out, &item).is_err());
}
#[test]
fn source_tar_accepts_only_its_exact_git_commit_comment() {
    let temp = Temp::new();
    let path = temp.0.join("input.tar.gz");
    let out = temp.0.join("out");
    fs::create_dir(&out).unwrap();
    let item = trust::artifact("whisper-source").unwrap();
    let comment = format!("52 comment={}\n", item.version);
    tar(
        &path,
        &[
            ("pax_global_header", b'g', "", comment.as_bytes()),
            ("source/file", b'0', "", b"abc"),
        ],
    );
    archive::extract_tar(&path, &out, &item).unwrap();
}
#[test]
fn failed_activation_restores_previous_install() {
    let temp = Temp::new();
    let target = temp.0.join("runtime");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("engine"), b"working").unwrap();
    assert!(install::activate_directory(&temp.0.join("missing"), &target).is_err());
    assert_eq!(fs::read(target.join("engine")).unwrap(), b"working");
    let stage = install::StagedDirectory::new(&target).unwrap();
    fs::write(stage.0.join("engine"), b"verified").unwrap();
    install::activate_directory(&stage.0, &target).unwrap();
    assert_eq!(fs::read(target.join("engine")).unwrap(), b"verified");
}

#[test]
fn zip_ownership_metadata_is_inert_and_strictly_bounded() {
    let valid = [0x75, 0x78, 11, 0, 1, 4, 0, 0, 0, 0, 4, 0, 0, 0, 0];
    assert!(crate::asr_components::legacy_validate_zip_extra(&valid).is_ok());
    for bytes in [
        &[0x75, 0x78, 1, 0, 1][..],
        &[0x75, 0x78, 3, 0, 1, 255, 1],
        &[0x75, 0x78, 3, 0, 1, 0, 0],
        &[0x6e, 0x75, 0, 0],
    ] {
        assert!(crate::asr_components::legacy_validate_zip_extra(bytes).is_err());
    }
}

#[test]
fn failed_model_activation_preserves_previous_bytes() {
    let temp = Temp::new();
    let path = temp.0.join("model");
    fs::write(&path, b"working").unwrap();
    assert!(install::activate_model(&temp.0.join("missing"), &path).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"working");
}

#[test]
fn pinned_catalog_preserves_native_backend_selection() {
    let items = trust::catalog().unwrap();
    let names = items
        .iter()
        .filter(|item| item.id.starts_with("llama-"))
        .map(|item| item.file_name.as_str())
        .collect::<Vec<_>>();
    for (os, arch, nvidia, backend) in [
        ("windows", "x86_64", true, "CUDA"),
        ("windows", "x86_64", false, "Vulkan"),
        ("macos", "aarch64", false, "Metal"),
    ] {
        let selected = super::llama::select_llama_cpp_assets(&names, os, arch, nvidia).unwrap();
        assert_eq!(selected.backend, backend);
        if nvidia {
            assert_eq!(
                selected.cudart_name.as_deref(),
                Some("cudart-llama-bin-win-cuda-12.4-x64.zip")
            );
        }
    }
}
