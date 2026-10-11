//! Feature-only native IPC regression application. This module, its fixture trust
//! roots, commands and path override are absent from ordinary release builds.
//! Never distribute a binary built with `native-install-regression`.
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{atomic::{AtomicI32, AtomicUsize, Ordering}, Mutex, OnceLock},
    time::Duration,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};
use super::trust::{Artifact, Source};

const MODEL: &[u8] = include_bytes!("../../native-install-regression/fixtures/model.bin");
const ZIP: &[u8] = include_bytes!("../../native-install-regression/fixtures/engine.zip");
const TAR: &[u8] = include_bytes!("../../native-install-regression/fixtures/engine.tar.gz");
const SERVER: &[u8] = b"LUMA NATIVE REGRESSION: non-executable server fixture\n";
const DLL: &[u8] = b"LUMA NATIVE REGRESSION: non-executable dependency fixture\n";
const OLD: &[u8] = b"Prior fixture bytes must survive a failed installation.\n";
const MODEL_NAME: &str = "Hy-MT2-1.8B-Q4_K_M.gguf";
const CASES: &[&str] = &[
    "metrics", "engine-cold", "engine-cached", "engine-stale", "engine-missing-dll",
    "engine-extraction-failure", "engine-activation-failure", "engine-rename-failure",
    "engine-wrong-hash", "engine-network-rejection", "engine-concurrent", "engine-nested-model-failure",
    "model-cold", "model-cached", "model-corrupt-cached", "model-truncated",
    "model-wrong-hash", "model-network-rejection", "model-activation-failure",
    "model-rename-failure", "model-concurrent",
];
struct Harness {
    case: String,
    root: PathBuf,
    report: PathBuf,
    log: Mutex<File>,
    catalog: Vec<Artifact>,
    engine_bytes: Vec<u8>,
    requests: AtomicUsize,
    main_thread: std::thread::ThreadId,
    // Keep OS rename-denying handles alive until the result has been checked.
    _rename_lock: Option<File>,
}
static HARNESS: OnceLock<Harness> = OnceLock::new();
// The first macOS run reported OS exit 0 even after app.exit(1). Preserve the
// diagnostic result and apply it explicitly at the normal exit lifecycle event.
static EXIT_CODE: AtomicI32 = AtomicI32::new(2);
fn harness() -> &'static Harness { HARNESS.get().expect("Native fixture runner was not initialized") }
pub(crate) fn app_data_root() -> PathBuf { harness().root.join("app-data") }
pub(crate) fn app_config_root() -> PathBuf { harness().root.join("app-config") }
fn is_engine() -> bool { harness().case.starts_with("engine-") }
fn expected_error() -> bool {
    ["-failure", "-wrong-hash", "-network-rejection", "-truncated"]
        .iter().any(|suffix| harness().case.ends_with(suffix))
}
fn engine_dir() -> PathBuf { app_data_root().join("sidecars").join("llama.cpp") }
fn server_name() -> &'static str { if cfg!(windows) { "llama-server.exe" } else { "llama-server" } }
fn dll_name() -> &'static str { if cfg!(windows) { "ggml-base.dll" } else { "libggml-base.dylib" } }
fn model_path() -> PathBuf { app_data_root().join("models").join(MODEL_NAME) }
fn record(value: Value) {
    let mut output = harness().log.lock().expect("stage log mutex");
    writeln!(output, "{value}").expect("write stage log");
    output.flush().expect("flush stage log");
}
pub(super) fn stage(name: &'static str) {
    record(json!({"event": "stage", "name": name,
        "thread": format!("{:?}", std::thread::current().id()),
        "main_thread": std::thread::current().id() == harness().main_thread}));
}
pub(super) fn observe_future<T>(name: &'static str, future: &T) {
    record(json!({"event": "future-size", "name": name, "bytes": std::mem::size_of_val(future)}));
}
pub(super) fn fixture_catalog() -> Result<Vec<Artifact>, String> { Ok(harness().catalog.clone()) }
pub(super) fn validate_fixture_artifact(item: &Artifact) -> Result<(), String> {
    // Match every field against in-process sealed fixture roots. No arbitrary URL,
    // port, file, catalog or trust override can be supplied through the environment.
    if harness().catalog.iter().any(|known| {
        item.id == known.id && item.version == known.version && item.file_name == known.file_name
            && item.url == known.url && item.bytes == known.bytes && item.sha256 == known.sha256
            && item.source == known.source && item.unpacked_bytes == known.unpacked_bytes
            && item.file_limit == known.file_limit
    }) { Ok(()) } else { Err("Descriptor is not an exact native regression fixture".into()) }
}
pub(super) fn fixture_client(item: &Artifact) -> Result<reqwest::Client, String> {
    validate_fixture_artifact(item)?;
    reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3)).timeout(Duration::from_secs(15))
        .build().map_err(|e| e.to_string())
}
fn artifact(id: &str, name: &str, port: u16, route: &str, bytes: &[u8], sha: &str, source: Source) -> Artifact {
    Artifact { id: id.into(), version: "native-fixture-v1".into(), file_name: name.into(),
        url: format!("http://127.0.0.1:{port}/{route}"), bytes: bytes.len() as u64,
        sha256: sha.into(), source, unpacked_bytes: 1024 * 1024, file_limit: 16 }
}
fn start_fixture_server(listener: TcpListener) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if let Ok(mut stream) = stream { let _ = serve(&mut stream); }
        }
    });
}
fn serve(stream: &mut TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    let mut request = Vec::new();
    let mut buffer = [0u8; 1024];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 8192 {
        let count = stream.read(&mut buffer)?;
        if count == 0 { return Ok(()); }
        request.extend_from_slice(&buffer[..count]);
    }
    let text = String::from_utf8_lossy(&request);
    let route = text.lines().next().unwrap_or_default().split_whitespace().nth(1).unwrap_or("");
    let payload = match route {
        "/model" => MODEL,
        "/engine" => harness().engine_bytes.as_slice(),
        _ => { stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?; return Ok(()); }
    };
    harness().requests.fetch_add(1, Ordering::SeqCst);
    stage("fixture-http-request");
    if harness().case.ends_with("network-rejection") {
        return stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
    let offset = text.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if !key.eq_ignore_ascii_case("range") { return None; }
        value.trim().strip_prefix("bytes=")?.strip_suffix('-')?.parse::<usize>().ok()
    }).unwrap_or(0);
    if offset >= payload.len() {
        return stream.write_all(b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
    let mut body = payload[offset..].to_vec();
    if harness().case.ends_with("wrong-hash") { body[0] ^= 0xff; }
    let status = if offset == 0 { "200 OK" } else { "206 Partial Content" };
    write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len())?;
    if offset != 0 { write!(stream, "Content-Range: bytes {offset}-{}/{}\r\n", payload.len() - 1, payload.len())?; }
    stream.write_all(b"\r\n")?;
    // Keep a real asynchronous network await pending; do not replace the downloader.
    std::thread::sleep(Duration::from_millis(30));
    if harness().case.ends_with("truncated") { body.truncate(body.len() / 2); }
    stream.write_all(&body)?;
    stream.flush()
}
fn seed_case(case: &str, root: &Path) -> Result<Option<File>, String> {
    let models = root.join("app-data/models");
    let engine = root.join("app-data/sidecars/llama.cpp");
    fs::create_dir_all(&models).map_err(|e| e.to_string())?;
    fs::create_dir_all(engine.parent().unwrap()).map_err(|e| e.to_string())?;
    if case == "model-corrupt-cached" { fs::write(models.join(MODEL_NAME), vec![b'?'; MODEL.len()]).map_err(|e| e.to_string())?; }
    if case == "model-activation-failure" {
        fs::create_dir(models.join(MODEL_NAME)).map_err(|e| e.to_string())?;
        fs::write(models.join(MODEL_NAME).join("preserve"), OLD).map_err(|e| e.to_string())?;
    }
    if case == "engine-activation-failure" { fs::write(&engine, OLD).map_err(|e| e.to_string())?; }
    if case == "engine-stale" || case == "engine-nested-model-failure" || (case.starts_with("engine-") &&
        ["-extraction-failure", "-wrong-hash", "-network-rejection", "-rename-failure"].iter().any(|s| case.ends_with(s))) {
        fs::create_dir_all(engine.join("bin")).map_err(|e| e.to_string())?;
        fs::write(engine.join("bin").join(server_name()), OLD).map_err(|e| e.to_string())?;
        fs::write(engine.join("preserve"), OLD).map_err(|e| e.to_string())?;
    }
    if case == "engine-nested-model-failure" {
        let selected_model = engine.join("selected-model.gguf");
        fs::write(&selected_model, MODEL).map_err(|e| e.to_string())?;
        fs::create_dir(root.join("app-config")).map_err(|e| e.to_string())?;
        let settings = json!({"base_url":"https://api.openai.com", "model":"fixture", "temperature":0.2,
            "whisper_model_path":"", "whisper_language":"auto", "target_language":"English",
            "translation_provider":"local", "translation_local_model_path":selected_model});
        fs::write(root.join("app-config/settings.json"), serde_json::to_vec(&settings).unwrap()).map_err(|e| e.to_string())?;
    }
    #[cfg(windows)]
    if case.ends_with("rename-failure") {
        use std::os::windows::fs::OpenOptionsExt;
        if case.starts_with("model-") {
            let path = models.join(MODEL_NAME);
            fs::write(&path, OLD).map_err(|e| e.to_string())?;
            return OpenOptions::new().read(true).share_mode(1).open(path).map(Some).map_err(|e| e.to_string());
        }
        return OpenOptions::new().read(true).share_mode(1).custom_flags(0x02000000)
            .open(engine).map(Some).map_err(|e| e.to_string());
    }
    Ok(None)
}

pub(crate) fn run() {
    assert!(!cfg!(debug_assertions), "Native regression requires --release");
    assert!(cfg!(windows) || cfg!(all(target_os = "macos", target_arch = "aarch64")),
        "Native fixture runner supports Windows and macOS Apple Silicon");
    let args: Vec<String> = std::env::args().collect();
    assert!(args.len() == 5 && args[1] == "--case" && args[3] == "--report",
        "Diagnostic application only. Usage: --case NAME --report ABSOLUTE_JSON_PATH");
    let case = args[2].clone();
    assert!(CASES.contains(&case.as_str()), "Unknown native regression case");
    let report = PathBuf::from(&args[4]);
    assert!(report.is_absolute(), "Report path must be absolute");
    let root = std::env::temp_dir().join(format!("luma-native-install-regression-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).expect("create unique fixture root");
    let log = OpenOptions::new().write(true).create_new(true).open(report.with_extension("stages.jsonl")).expect("create stage log");
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).expect("bind loopback fixture listener");
    let port = listener.local_addr().unwrap().port();
    let manifest: Value = serde_json::from_str(include_str!("../../native-install-regression/fixtures/manifest.json")).unwrap();
    let (archive, fixture_name, name) = if cfg!(target_os = "macos") {
        (TAR, "engine.tar.gz", "llama-native-fixture-bin-macos-arm64.tar.gz")
    } else if cfg!(target_arch = "aarch64") {
        (ZIP, "engine.zip", "llama-native-fixture-bin-win-cpu-arm64.zip")
    } else { (ZIP, "engine.zip", "llama-native-fixture-bin-win-cpu-x64.zip") };
    for (data, key) in [(MODEL, "model.bin"), (archive, fixture_name)] {
        assert_eq!(data.len() as u64, manifest[key]["bytes"].as_u64().unwrap());
        assert_eq!(format!("{:x}", Sha256::digest(data)), manifest[key]["sha256"].as_str().unwrap());
    }
    let engine_bytes = if case == "engine-extraction-failure" { b"This is not a valid archive.\n".to_vec() } else { archive.to_vec() };
    let catalog = vec![
        artifact("hy-mt2-1.8b-q4", MODEL_NAME, port, "model", MODEL, manifest["model.bin"]["sha256"].as_str().unwrap(), Source::Model),
        artifact(if cfg!(target_os = "macos") { "llama-metal" } else if cfg!(target_arch = "aarch64") { "llama-cpu-arm64" } else { "llama-cpu" },
            name, port, "engine", &engine_bytes, &format!("{:x}", Sha256::digest(&engine_bytes)), Source::Github),
    ];
    let rename_lock = seed_case(&case, &root).expect("seed isolated fixture state");
    HARNESS.set(Harness { case, root, report, log: Mutex::new(log), catalog, engine_bytes,
        requests: AtomicUsize::new(0), main_thread: std::thread::current().id(), _rename_lock: rename_lock })
        .unwrap_or_else(|_| panic!("Native fixture runner initialized twice"));
    stage("native-main");
    record(json!({"event": "environment", "case": harness().case, "root": harness().root,
        "os": std::env::consts::OS, "arch": std::env::consts::ARCH, "release": !cfg!(debug_assertions),
        "notice": "LUMA_NATIVE_INSTALL_REGRESSION_NOT_FOR_DISTRIBUTION"}));
    start_fixture_server(listener);
    let generated_handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
        super::install_llama_cpp, super::download_translation_model, super::download_status,
        native_test_plan, native_test_followup, native_test_measure, native_test_finish,
    ];
    tauri::Builder::default()
        .manage(crate::state::AppState::default())
        .setup(|app| {
            tauri::WebviewWindowBuilder::new(app, "native-regression", tauri::WebviewUrl::App("index.html".into()))
                .title("Luma native regression (not for distribution)")
                .data_directory(harness().root.join("webview"))
                .visible(false)
                .build()?;
            Ok(())
        })
        .invoke_handler(move |invoke| {
            record(json!({"event":"ipc-dispatch", "command":invoke.message.command(),
                "main_thread":std::thread::current().id() == harness().main_thread,
                "thread":format!("{:?}", std::thread::current().id())}));
            generated_handler(invoke)
        })
        .build(tauri::generate_context!("native-install-regression/tauri.conf.json"))
        .expect("build native regression WebView")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                app.cleanup_before_exit();
                std::process::exit(EXIT_CODE.load(Ordering::SeqCst));
            }
        });
}

#[tauri::command]
fn native_test_plan() -> Value {
    json!({"metricsOnly":harness().case == "metrics",
        "command":if is_engine() { "install_llama_cpp" } else { "download_translation_model" },
        "args":if is_engine() { json!({}) } else { json!({"request":{"preset_id":"hy-mt2-1.8b-q4"}}) },
        "concurrent":harness().case.ends_with("concurrent"),
        "iterations":if harness().case.ends_with("cached") || harness().case == "engine-missing-dll" { 2 } else { 1 }})
}
#[tauri::command]
fn native_test_followup() -> Result<(), String> {
    if harness().case == "engine-missing-dll" {
        fs::remove_file(engine_dir().join("bin").join(dll_name())).map_err(|e| e.to_string())?;
        stage("fixture-removed-dll");
    }
    Ok(())
}
#[tauri::command]
fn native_test_measure(app: AppHandle) -> Result<(), String> {
    check_json_comparison()?;
    // A separate child: constructing futures for measurement never changes the
    // command dispatch or stack layout in the pass/fail regression cases.
    let item = &harness().catalog[0];
    let path = model_path();
    observe_future("verify_file", &super::trust::verify_file(&path, item));
    observe_future("download_file_with_resume_noop_callbacks", &super::download::download_file_with_resume(item, &path, 1.0, |_| {}, |_, _, _| {}));
    observe_future("install_llama_cpp_command", &super::install_llama_cpp(app.clone()));
    observe_future("install_llama_cpp_impl", &super::llama::install_llama_cpp(app.clone()));
    super::llama::observe_inner_future(&app);
    let request = || super::DownloadTranslationModelRequest { preset_id: "hy-mt2-1.8b-q4".into() };
    observe_future("download_translation_model_command", &super::download_translation_model(app.clone(), request()));
    observe_future("llama_ResultFutureTag", &tauri::ipc::private::ResultFutureTag.future(super::install_llama_cpp(app.clone())));
    observe_future("model_ResultFutureTag", &tauri::ipc::private::ResultFutureTag.future(super::download_translation_model(app, request())));
    Ok(())
}

/// Preserve the production IPC serializer, including its f32 representation.
fn backend_snapshot(app: &AppHandle) -> Result<Value, String> {
    let state = app.state::<crate::state::AppState>();
    let status = super::DownloadStatus {
        model: state.model_download.lock().clone(),
        dependency: state.dependency_install.lock().clone(),
    };
    let wire = serde_json::to_string(&status).map_err(|e| e.to_string())?;
    serde_json::from_str(&wire).map_err(|e| e.to_string())
}

/// JavaScript JSON.stringify turns 1.0/0.0 into 1/0. Require the same complete
/// object shape and exact values while allowing that harmless numeric spelling
/// difference. Do not coerce strings, omit nulls, ignore fields or use tolerance.
fn same_ipc_json(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::Number(a), Value::Number(b)) => {
            a == b || (a.is_f64() != b.is_f64() &&
                matches!((a.as_f64(), b.as_f64()), (Some(a), Some(b))
                    if a == b && a.abs() <= 9_007_199_254_740_991.0))
        }
        (Value::Array(a), Value::Array(b)) => a.len() == b.len() &&
            a.iter().zip(b).all(|(a, b)| same_ipc_json(a, b)),
        (Value::Object(a), Value::Object(b)) => a.len() == b.len() &&
            a.iter().all(|(key, value)| b.get(key).is_some_and(|other| same_ipc_json(value, other))),
        _ => expected == actual,
    }
}

/// Execute comparison regression assertions in the native metrics child too.
fn check_json_comparison() -> Result<(), String> {
    let expected = json!({"status":"completed", "progress":1.0, "path":"fixture/server",
        "error":null, "nested":[0.0, 0.5, 32]});
    let roundtrip = json!({"status":"completed", "progress":1, "path":"fixture/server",
        "error":null, "nested":[0, 0.5, 32.0]});
    if !same_ipc_json(&expected, &roundtrip) { return Err("JSON comparator rejected equivalent integral floats".into()); }
    for (field, value) in [("status", json!("failed")), ("path", json!("other/server")),
        ("error", json!("unexpected")), ("progress", json!(0)), ("progress", json!("1")),
        ("progress", json!(true)), ("nested", json!([0, 0.5]))] {
        let mut changed = roundtrip.clone();
        changed[field] = value;
        if same_ipc_json(&expected, &changed) { return Err(format!("JSON comparator ignored changed {field}")); }
    }
    let mut missing = roundtrip.clone();
    missing.as_object_mut().unwrap().remove("error");
    let mut extra = roundtrip.clone();
    extra["extra"] = Value::Null;
    if same_ipc_json(&expected, &missing) || same_ipc_json(&expected, &extra) ||
        same_ipc_json(&json!(9_007_199_254_740_992u64), &json!(9_007_199_254_740_993u64)) ||
        same_ipc_json(&json!(9_007_199_254_740_993u64), &json!(9_007_199_254_740_992.0)) {
        return Err("JSON comparator ignored shape or large-integer precision mismatch".into());
    }
    stage("json-comparison-regressions-passed");
    Ok(())
}
fn validate_report(report: &Value, app: &AppHandle) -> Result<(), String> {
    if !report["fatal"].is_null() { return Err(format!("WebView fixture failed: {}", report["fatal"])); }
    if harness().case == "metrics" { return Ok(()); }
    let expected = if expected_error() { "failed" } else { "completed" };
    let key = if is_engine() { "dependency" } else { "model" };
    let results = report["results"].as_array().ok_or("No IPC results")?;
    let plan = native_test_plan();
    let count = if plan["concurrent"] == true { 2 } else { plan["iterations"].as_u64().unwrap() as usize };
    if results.len() != count { return Err("Unexpected number of IPC results".into()); }
    for result in results {
        if result["ok"] != !expected_error() { return Err(format!("Unexpected IPC result: {result}")); }
        if expected_error() && result["error"].as_str().unwrap_or("").is_empty() { return Err("IPC rejection has no error".into()); }
    }
    for status in report["snapshots"].as_array().ok_or("No snapshots")? {
        if status[key]["status"] != expected { return Err(format!("IPC snapshot has non-terminal/wrong status: {status}")); }
    }
    let backend = backend_snapshot(app)?;
    if !same_ipc_json(&backend, &report["finalSnapshot"]) || backend[key]["status"] != expected { return Err("WebView and backend terminal status differ".into()); }
    let events = report["events"].as_array().ok_or("No WebView events")?;
    let event = events.last().ok_or("No terminal event reached the WebView")?;
    if event["status"] != expected { return Err("Last queued WebView event disagrees with terminal state".into()); }
    if !expected_error() {
        if is_engine() {
            if fs::read(engine_dir().join("bin").join(server_name())).map_err(|e| e.to_string())? != SERVER ||
                fs::read(engine_dir().join("bin").join(dll_name())).map_err(|e| e.to_string())? != DLL {
                return Err("Activated fixture bundle differs from trusted bytes".into());
            }
            if !engine_dir().join(".luma-llama-receipt.json").is_file() { return Err("No installed bundle receipt".into()); }
        } else if fs::read(model_path()).map_err(|e| e.to_string())? != MODEL { return Err("Activated model bytes differ".into()); }
    }
    let requests = harness().requests.load(Ordering::SeqCst);
    let expected_requests = if harness().case == "engine-nested-model-failure" { 0 }
        else if harness().case == "engine-missing-dll" { 2 } else if harness().case == "model-truncated" { 4 } else { 1 };
    if requests != expected_requests { return Err(format!("Expected {expected_requests} fixture requests, got {requests}")); }
    if expected_error() {
        let preserved = if harness().case == "model-activation-failure" { Some(model_path().join("preserve")) }
            else if harness().case == "model-rename-failure" { Some(model_path()) }
            else if harness().case == "engine-activation-failure" { Some(engine_dir()) }
            else if is_engine() { Some(engine_dir().join("preserve")) } else { None };
        if let Some(path) = preserved {
            if fs::read(path).map_err(|e| e.to_string())? != OLD { return Err("Previous fixture bytes were not preserved".into()); }
        }
        if harness().case == "engine-nested-model-failure" &&
            fs::read(engine_dir().join("selected-model.gguf")).map_err(|e| e.to_string())? != MODEL {
            return Err("Selected model nested in engine directory was moved or changed".into());
        }
    }
    let expected_backup = matches!(harness().case.as_str(), "engine-stale" | "engine-missing-dll");
    let mut backups = Vec::new();
    assert_no_staging(&app_data_root(), &mut backups)?;
    if backups.len() != usize::from(expected_backup) { return Err("Unexpected retained backup count".into()); }
    if let Some(backup) = backups.first() {
        if harness().case == "engine-stale" {
            if fs::read(backup.join("preserve")).map_err(|e| e.to_string())? != OLD ||
                fs::read(backup.join("bin").join(server_name())).map_err(|e| e.to_string())? != OLD {
                return Err("Repair did not preserve old fixture bytes in its backup".into());
            }
        } else if fs::read(backup.join("bin").join(server_name())).map_err(|e| e.to_string())? != SERVER ||
            backup.join("bin").join(dll_name()).exists() {
            return Err("Missing-DLL repair backup does not match the prior tree".into());
        }
    }
    Ok(())
}
fn assert_no_staging(root: &Path, backups: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        if [".part", ".installing"].iter().any(|s| name.ends_with(s)) { return Err(format!("Leaked staging entry: {name}")); }
        if name.ends_with(".previous") { backups.push(entry.path()); }
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() { assert_no_staging(&entry.path(), backups)?; }
    }
    Ok(())
}
#[tauri::command]
fn native_test_finish(app: AppHandle, report: Value) {
    let validation = validate_report(&report, &app);
    let exit_code = if validation.is_ok() { 0 } else { 1 };
    let backend = backend_snapshot(&app);
    let output = json!({"case":harness().case, "passed":validation.is_ok(), "error":validation.err(),
        "fixture_requests":harness().requests.load(Ordering::SeqCst), "root":harness().root,
        "scope":"feature-built release-mode Wry IPC, not the distributable binary", "webview":report,
        "backend_snapshot":backend.as_ref().ok(), "backend_snapshot_error":backend.as_ref().err(),
        "requested_exit_code":exit_code});
    fs::write(&harness().report, serde_json::to_vec_pretty(&output).unwrap()).expect("write native regression report");
    record(json!({"event":"finished", "passed":output["passed"]}));
    EXIT_CODE.store(exit_code, Ordering::SeqCst);
    app.exit(exit_code);
}
