//! One fixed Microsoft runtime package, delivered directly to the user. This is
//! not a Burn/MSI installer or resolver; no downloaded executable is ever run.
use super::{archive, catalog::{Archive, Runtime}, download, recipe::Recipe, setup_process, store};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::{self, File, OpenOptions}, io::{Read, Seek, SeekFrom, Write}, path::{Path, PathBuf}, sync::atomic::AtomicBool};

pub(super) const ID: &str = "msvc-14.44.35211-x64";
pub(super) const URL: &str = "https://download.visualstudio.microsoft.com/download/pr/73aabf2e-9532-4f68-99f7-3247081a619c/CC0FF0EB1DC3F5188AE6300FAEF32BF5BEEBA4BDD6E8E445A9184072096B713B/VC_redist.x64.exe";
pub(super) const INSTALLER_SHA: &str = "cc0ff0eb1dc3f5188ae6300faef32bf5beeba4bdd6e8e445a9184072096b713b";
pub(super) const DOWNLOAD_BYTES: u64 = 25_635_768;
pub(super) const INSTALLED_BOUND: u64 = 621_129 + 16_384;
pub(super) const CONTRACT: &str = include_str!("../../../scripts/asr-components/ct2-cpu/direct-crt/package.lock.json");
const EXE_SIGNER: &str = "e4ab39116a7dc57d073164eb1c840b1fb8334a8c920b92efafea19112dce643b";
const DLL_SIGNER: &str = "2ebcd329b745ae0efb6a72bb8a471b29c3994b074dbf94bdc2bb65936c54f4a7";
const PREFIX: &str = "LUMA_CRT_SIGNATURE ";
const HELPER: &str = "--luma-verify-crt";
const TERMS_URL: &str = "https://visualstudio.microsoft.com/license-terms/vs2022-cruntime/";
#[derive(Clone, Copy, PartialEq, Eq)]
struct Pinned { name: &'static str, member: &'static str, bytes: u64, sha: &'static str }
const INSTALLER: Pinned = Pinned { name: "installer", member: "", bytes: DOWNLOAD_BYTES, sha: INSTALLER_SHA };
const MINIMUM: Pinned = Pinned { name: "minimum-x64.cab", member: "a12", bytes: 987_836, sha: "640aa6c516c72444523b8fbe034db46ff4e118ed02705340e3ccb62d426ff040" };
const DLLS: [Pinned; 2] = [
    Pinned { name: "msvcp140.dll", member: "msvcp140.dll_amd64", bytes: 557_728, sha: "0f885b509a685d2bbfa652fed26b5fb31d88fbdab0a978c641d1c7b8aa460aa9" },
    Pinned { name: "msvcp140_1.dll", member: "msvcp140_1.dll_amd64", bytes: 35_952, sha: "bfad5aef4c63a669e3c140655cdfdf395b6c979b400a447bd5dcb65ed8826c3d" },
];
const NOTICES: [Pinned; 2] = [
    Pinned { name: "license-en.rtf", member: "u4", bytes: 9_235, sha: "8099dc3cf9502c335da829e5c755948a12e3e6de490eb492a99deb673d883d8b" },
    Pinned { name: "license-zh-CN.rtf", member: "u28", bytes: 18_214, sha: "a808b4933ce3b3e0893504dbef43ebf90b8b567f94bd6481b6315ed9141e1b11" },
];
const COMPANIONS: [Pinned; 2] = [
    Pinned { name: "vcruntime140.dll", member: "", bytes: 124_544, sha: "d5e4d9a3e835fa679450145d6a7d94e36573a509317111904d9b3712c30d9066" },
    Pinned { name: "vcruntime140_1.dll", member: "", bytes: 49_792, sha: "1f2d41c4aa5db0bc33ebf7b66d72943a817d7ce6cbe880502a9403823633093f" },
];
const SLICES: [(u64, Pinned); 2] = [
    (479_744, Pinned { name: "ux.cab", member: "", bytes: 195_988, sha: "2f57cf2cd504bd100c9222bb123b9c3a40802cca73e89ca05c97885d91be3a78" }),
    (686_152, Pinned { name: "attached.cab", member: "", bytes: 24_939_223, sha: "468f1264d50b9e3b9d309ab9e13c02ff379fd95a2837383bf65c9616583013d6" }),
];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TrustMode { Online, Cached }
impl TrustMode {
    fn name(self) -> &'static str { match self { Self::Online => "online", Self::Cached => "cache-only" } }
    fn parse(value: &str) -> Result<Self, String> { match value { "online" => Ok(Self::Online), "cache-only" => Ok(Self::Cached), _ => Err("Unknown CRT trust mode.".into()) } }
}
pub(super) fn artifact() -> Archive { Archive { url: URL.into(), bytes: DOWNLOAD_BYTES, sha256: INSTALLER_SHA.into() } }
pub(super) fn validate_recipe(recipe: &Recipe, runtime: &Runtime) -> Result<(), String> {
    let Some(id) = &recipe.windows_crt else { return Ok(()); };
    if id != ID || runtime.platform != "windows-x64" { return Err("Unsupported fixed Windows CRT contract.".into()); }
    for (id, version, notice, text_hash) in [("microsoft-vc-runtime-en", "Cpp_2015-2022_ENU.1033", NOTICES[0], "f815ace86c91d3dadc62ed85292b7c79066447f2277774803df87b1d1a9ecfc8"), ("microsoft-vc-runtime-zh-cn", "Cpp_2015-2022_CHS.2052", NOTICES[1], "bf17656105173f49b79623f1508c4c8eddf9b09e78a6ea79ee5aad99015eb38e")] {
        let matches: Vec<_> = recipe.terms.iter().filter(|t| t.id == id).collect();
        if matches.len() != 1 { return Err("Both reviewed Microsoft runtime license texts are required before setup.".into()); }
        let term = matches[0];
        if term.version != version || term.sha256 != text_hash || format!("{:x}", Sha256::digest(term.text.as_bytes())) != text_hash || term.raw_sha256.as_deref() != Some(notice.sha) || term.source_encoding.as_deref() != Some("rtf") || term.url != TERMS_URL || !term.text.contains(version) {
            return Err("Microsoft runtime terms differ from the audited locale/source contract.".into());
        }
    }
    Ok(())
}
fn check_signer(actual: &str, expected: &str) -> Result<(), String> { if actual == expected { Ok(()) } else { Err("Windows verified a signer other than the audited Microsoft certificate.".into()) } }
fn role(value: &str) -> Result<Pinned, String> { if value == "installer" { return Ok(INSTALLER); } DLLS.iter().find(|p| p.name == value).copied().ok_or("Unknown fixed CRT role.".into()) }
fn open_verified(path: &Path, pin: Pinned, cancel: &AtomicBool) -> Result<File, String> {
    if store::regular_file(path)?.len() != pin.bytes { return Err(format!("The private {} has an unexpected byte size.", pin.name)); }
    let mut options = OpenOptions::new(); options.read(true);
    // FILE_SHARE_READ only: deny concurrent write/delete through verification
    // and subsequent reads, including the child trust check on Windows.
    #[cfg(windows)] { use std::os::windows::fs::OpenOptionsExt; options.share_mode(1); }
    let mut input = options.open(path).map_err(|e| e.to_string())?;
    verify_handle(&mut input, pin, cancel)?; Ok(input)
}
fn verify_handle(input: &mut File, pin: Pinned, cancel: &AtomicBool) -> Result<(), String> {
    input.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new(); let mut size = 0u64; let mut buffer = [0u8; 64 * 1024];
    loop { archive::cancelled(cancel)?; let n = input.read(&mut buffer).map_err(|e| e.to_string())?; if n == 0 { break; } size += n as u64; if size > pin.bytes { return Err("CRT file exceeds its fixed byte limit.".into()); } digest.update(&buffer[..n]); }
    download::verify_digest(size, &format!("{:x}", digest.finalize()), pin.bytes, pin.sha)?;
    input.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?; Ok(())
}
fn write_checked(input: &mut File, target: &Path, pin: Pinned, cancel: &AtomicBool) -> Result<(), String> {
    let mut output = OpenOptions::new().create_new(true).write(true).open(target).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new(); let mut remaining = pin.bytes; let mut buffer = [0u8; 64 * 1024];
    while remaining > 0 { archive::cancelled(cancel)?; let amount = remaining.min(buffer.len() as u64) as usize; let n = input.read(&mut buffer[..amount]).map_err(|e| e.to_string())?; if n == 0 { return Err("Truncated fixed CRT data.".into()); } output.write_all(&buffer[..n]).map_err(|e| e.to_string())?; digest.update(&buffer[..n]); remaining -= n as u64; }
    download::verify_digest(pin.bytes, &format!("{:x}", digest.finalize()), pin.bytes, pin.sha)?;
    output.sync_all().map_err(|e| e.to_string())
}
fn slice(input: &mut File, total: u64, offset: u64, pin: Pinned, destination: &Path, cancel: &AtomicBool) -> Result<(), String> {
    if offset.checked_add(pin.bytes).filter(|end| *end <= total).is_none() { return Err("Fixed CRT CAB range is out of bounds.".into()); }
    input.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
    write_checked(input, destination, pin, cancel)
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Signature { schema: u32, role: String, mode: String, sha256: String, signer_sha256: String }
async fn signature(path: &Path, pin: Pinned, work: &Path, mode: TrustMode, cancel: &AtomicBool, lifetime: &setup_process::SetupLifetime) -> Result<Signature, String> {
    let mut command = tokio::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    let temporary = setup_process::PrivateTemp::create(work)?;
    setup_process::configure(&mut command, work, &temporary)?;
    command.current_dir(work);
    #[cfg(not(test))] command.arg(HELPER).arg(mode.name()).arg(pin.name).arg(path);
    #[cfg(test)] {
        command.args(["--ignored", "--exact", "asr_components::direct_crt::tests::native_signature_child", "--nocapture"])
            .env("LUMA_CRT_CHILD_MODE", mode.name()).env("LUMA_CRT_CHILD_ROLE", pin.name).env("LUMA_CRT_CHILD_PATH", path);
    }
    let (success, bytes) = setup_process::run_owned(command, "Microsoft signature verification", 120, cancel, lifetime, temporary).await?;
    signature_result(success, &bytes, pin, mode)
}
fn signature_result(success: bool, bytes: &[u8], pin: Pinned, mode: TrustMode) -> Result<Signature, String> {
    let text = String::from_utf8_lossy(bytes);
    let records: Vec<_> = text.lines().filter_map(|line| line.strip_prefix(PREFIX)).collect();
    if records.len() != 1 { return Err("Windows signature verification returned no valid result. Check Windows security policy and retry.".into()); }
    if !success { let error: serde_json::Value = serde_json::from_str(records[0]).map_err(|_| "Invalid Windows trust diagnostic")?; return Err(error["error"].as_str().unwrap_or("Windows rejected the Microsoft runtime signature.").chars().take(1024).collect()); }
    let verified: Signature = serde_json::from_str(records[0]).map_err(|_| "Invalid Windows trust result")?;
    if verified.schema != 1 || verified.role != pin.name || verified.mode != mode.name() || verified.sha256 != pin.sha || verified.signer_sha256 != if pin.name == "installer" { EXE_SIGNER } else { DLL_SIGNER } { return Err("Windows signature result differs from the fixed Microsoft contract.".into()); }
    Ok(verified)
}
fn selected_member(pin: Pinned) -> Result<(), String> { if ![MINIMUM.member, DLLS[0].member, DLLS[1].member, NOTICES[0].member, NOTICES[1].member].contains(&pin.member) { return Err("Unknown fixed Microsoft CAB member.".into()); } Ok(()) }
fn expansion_arguments(cabinet: &Path, cabinet_pin: Pinned, pin: Pinned, work: &Path) -> Result<[String; 3], String> {
    selected_member(pin)?;
    let fixed_pair = (cabinet_pin == SLICES[1].1 && pin == MINIMUM)
        || (cabinet_pin == MINIMUM && DLLS.contains(&pin))
        || (cabinet_pin == SLICES[0].1 && NOTICES.contains(&pin));
    if !fixed_pair || !work.is_absolute() || work.components().any(|part| matches!(part, std::path::Component::ParentDir | std::path::Component::CurDir)) || cabinet.parent() != Some(work)
        || cabinet.file_name() != Some(std::ffi::OsStr::new(cabinet_pin.name)) {
        return Err("Microsoft CAB extraction requires a fixed cabinet/member in its private working directory.".into());
    }
    Ok([cabinet_pin.name.into(), format!("-F:{}", pin.member), format!("extract-{}", pin.member)])
}
fn expansion_failure(status: std::process::ExitStatus, output: &[u8], cabinet: Pinned, member: Pinned) -> String {
    // Child capture is capped at 64 KiB by run_owned_status. Keep the UI error
    // short and strip terminal/control characters; do not guess that OS policy
    // caused an ordinary extraction/path failure.
    let diagnostic: String = String::from_utf8_lossy(output).chars().map(|c| if c.is_control() { ' ' } else { c }).take(2048).collect();
    format!("Windows CAB extraction failed for {} member {} ({status}). expand.exe: {}", cabinet.name, member.member,
        if diagnostic.trim().is_empty() { "No diagnostic output." } else { diagnostic.trim() })
}
async fn expand(cabinet: &Path, cabinet_pin: Pinned, pin: Pinned, work: &Path, cancel: &AtomicBool, lifetime: &setup_process::SetupLifetime) -> Result<PathBuf, String> {
    let arguments = expansion_arguments(cabinet, cabinet_pin, pin, work)?;
    let _held_cabinet = open_verified(cabinet, cabinet_pin, cancel)?;
    let directory = work.join(&arguments[2]);
    fs::create_dir(&directory).map_err(|e| e.to_string())?; store::ensure_directory(&directory)?;
    let mut command = tokio::process::Command::new(system_expand()?);
    let temporary = setup_process::PrivateTemp::create(work)?;
    setup_process::configure(&mut command, work, &temporary)?;
    // Windows canonicalization produces verbatim paths that legacy CAB tools
    // need not accept as command-line filenames. The child alone uses the
    // already owned Unicode working directory; all arguments are fixed ASCII
    // basenames, with no stripping/reinterpretation of trusted absolute paths.
    command.args(&arguments).current_dir(work);
    let (status, output) = setup_process::run_owned_status(command, "Windows CAB extraction", 120, cancel, lifetime, temporary).await?;
    if !status.success() { return Err(expansion_failure(status, &output, cabinet_pin, pin)); }
    let entries = fs::read_dir(&directory).map_err(|e| e.to_string())?.take(2).collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    if entries.len() != 1 || entries[0].file_name() != std::ffi::OsStr::new(pin.member) { return Err("Windows CAB extraction produced unexpected outputs.".into()); }
    let path = directory.join(pin.member); let _verified = open_verified(&path, pin, cancel)?; Ok(path)
}
pub(super) async fn install(package: &Path, staging: &Path, payload: &Path, cancel: &AtomicBool, mode: TrustMode, lifetime: &setup_process::SetupLifetime) -> Result<(), String> {
    archive::cancelled(cancel)?; if !cfg!(windows) { return Err("Microsoft runtime setup is available only on Windows.".into()); }
    store::ensure_directory(staging)?; store::ensure_directory(payload)?;
    for companion in COMPANIONS { let _verified = open_verified(&payload.join(companion.name), companion, cancel)?; }
    let work = staging.join("direct-crt"); fs::create_dir(&work).map_err(|e| e.to_string())?; store::ensure_directory(&work)?;
    let mut held_package = open_verified(package, INSTALLER, cancel)?;
    let mut signatures = vec![signature(package, INSTALLER, &work, mode, cancel, lifetime).await?];
    for (offset, pin) in SLICES { slice(&mut held_package, DOWNLOAD_BYTES, offset, pin, &work.join(pin.name), cancel)?; }
    let min_member = expand(&work.join("attached.cab"), SLICES[1].1, MINIMUM, &work, cancel, lifetime).await?;
    let minimum = work.join(MINIMUM.name); fs::rename(min_member, &minimum).map_err(|e| e.to_string())?;
    for pin in DLLS {
        let member = expand(&minimum, MINIMUM, pin, &work, cancel, lifetime).await?;
        let mut held = open_verified(&member, pin, cancel)?;
        signatures.push(signature(&member, pin, &work, mode, cancel, lifetime).await?);
        write_checked(&mut held, &payload.join(pin.name), pin, cancel)?;
    }
    let notices = payload.join("licenses").join(ID); store::create_private_dir(&notices)?;
    for pin in NOTICES { let member = expand(&work.join("ux.cab"), SLICES[0].1, pin, &work, cancel, lifetime).await?; let mut held = open_verified(&member, pin, cancel)?; write_checked(&mut held, &notices.join(pin.name), pin, cancel)?; }
    let receipt = serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"contract_id":ID,"contract_sha256":format!("{:x}",Sha256::digest(CONTRACT.as_bytes())),"source_url":URL,"installer_sha256":INSTALLER_SHA,"signature_policy":mode.name(),"signatures":signatures,"terms":[{"id":"Cpp_2015-2022_ENU.1033","raw_sha256":NOTICES[0].sha},{"id":"Cpp_2015-2022_CHS.2052","raw_sha256":NOTICES[1].sha}],"installer_executed":false,"global_installation":false})).map_err(|e| e.to_string())?;
    if receipt.len() > 16_384 { return Err("Oversized private CRT receipt.".into()); }
    let mut file = OpenOptions::new().create_new(true).write(true).open(notices.join("provenance.json")).map_err(|e| e.to_string())?; file.write_all(&receipt).and_then(|_|file.sync_all()).map_err(|e|e.to_string())?;
    archive::cancelled(cancel)
}
pub(super) fn verified_files(payload: &Path, cancel: &AtomicBool) -> Result<Vec<store::FileReceipt>, String> {
    let mut receipts = Vec::new();
    for pin in DLLS.into_iter().chain(COMPANIONS) {
        let path = store::checked_path(payload, pin.name)?; let _held = open_verified(&path, pin, cancel)?;
        receipts.push(store::FileReceipt { path: pin.name.into(), bytes: pin.bytes, sha256: pin.sha.into() });
    }
    for pin in NOTICES {
        let relative = format!("licenses/{ID}/{}", pin.name); let path = store::checked_path(payload, &relative)?;
        let _held = open_verified(&path, pin, cancel)?;
        receipts.push(store::FileReceipt { path: relative, bytes: pin.bytes, sha256: pin.sha.into() });
    }
    let relative = format!("licenses/{ID}/provenance.json"); let path = store::checked_path(payload, &relative)?;
    let bytes = store::regular_file(&path)?.len(); if bytes == 0 || bytes > 16_384 { return Err("Invalid private CRT provenance size.".into()); }
    let report: serde_json::Value = serde_json::from_slice(&fs::read(&path).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    if report["schema"] != 1 || report["contract_id"] != ID || report["contract_sha256"] != format!("{:x}",Sha256::digest(CONTRACT.as_bytes())) || report["installer_sha256"] != INSTALLER_SHA || report["source_url"] != URL || report["installer_executed"] != false || report["global_installation"] != false { return Err("Private CRT provenance differs from the fixed contract.".into()); }
    receipts.push(store::FileReceipt { path: relative, bytes, sha256: store::hash_file_checked(&path, cancel)? }); Ok(receipts)
}
#[cfg(not(windows))]
fn system_expand() -> Result<PathBuf, String> { Err("Windows CAB extraction is unavailable on this OS.".into()) }
#[cfg(windows)]
fn system_directory() -> Result<PathBuf, String> {
    use std::os::windows::ffi::OsStringExt;
    let mut buffer = vec![0u16; 32768];
    // The OS writes at most the supplied UTF-16 buffer length.
    let length = unsafe { windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if length == 0 || length >= buffer.len() { return Err("Cannot locate the real Windows system directory.".into()); }
    Ok(PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length])))
}
#[cfg(windows)]
fn system_expand() -> Result<PathBuf, String> { let path = system_directory()?.join("expand.exe"); store::regular_file(&path)?; Ok(path) }

#[cfg(windows)]
fn helper_path(path: &Path, pin: Pinned) -> Result<(), String> {
    use std::path::{Component, Prefix};
    let components: Vec<_> = path.components().collect();
    if !matches!(components.first(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))) || !path.is_absolute() || components.iter().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return Err("CRT trust helper requires an absolute local disk path.".into());
    }
    let name = path.file_name().ok_or("CRT trust helper input has no filename")?;
    let allowed = if pin.name == "installer" { name == "VC_redist.x64.exe" || name == INSTALLER_SHA } else { name == pin.name || name == pin.member };
    if !allowed { return Err("CRT trust helper rejects an unknown input filename.".into()); }
    let mut ancestor = path.parent();
    while let Some(directory) = ancestor { store::ensure_directory(directory)?; ancestor = directory.parent(); }
    store::regular_file(path)?; Ok(())
}

#[cfg(windows)]
fn native_signature(path: &Path, pin: Pinned, mode: TrustMode) -> Result<Signature, String> {
    use std::{os::windows::{ffi::OsStrExt, io::AsRawHandle}, ptr};
    use windows_sys::Win32::{Foundation::{HANDLE, INVALID_HANDLE_VALUE, FreeLibrary}, Security::WinTrust::*, System::LibraryLoader::{LoadLibraryExW, GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32}};
    helper_path(path, pin)?;
    let cancelled = AtomicBool::new(false);
    // Hash and signature verification share this held read-only/non-replaceable
    // handle; Authenticode alone intentionally does not cover every PE byte.
    let held = open_verified(path, pin, &cancelled)?;
    let mut path_wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if path_wide.contains(&0) { return Err("Invalid CRT input path.".into()); }
    path_wide.push(0);
    let mut library_path: Vec<u16> = system_directory()?.join("wintrust.dll").as_os_str().encode_wide().collect(); library_path.push(0);
    // WTHelper APIs are resolved from the absolute OS DLL, as recommended by
    // Microsoft. Missing exports fail closed; no alternate library is searched.
    let library = unsafe { LoadLibraryExW(library_path.as_ptr(), ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) };
    if library.is_null() { return Err("Windows trust services are unavailable under current OS policy.".into()); }
    struct Library(windows_sys::Win32::Foundation::HMODULE);
    impl Drop for Library { fn drop(&mut self) { unsafe { FreeLibrary(self.0); } } }
    let _library = Library(library);
    type GetData = unsafe extern "system" fn(HANDLE) -> *mut CRYPT_PROVIDER_DATA;
    type GetSigner = unsafe extern "system" fn(*mut CRYPT_PROVIDER_DATA, u32, i32, u32) -> *mut CRYPT_PROVIDER_SGNR;
    type GetCert = unsafe extern "system" fn(*mut CRYPT_PROVIDER_SGNR, u32) -> *mut CRYPT_PROVIDER_CERT;
    // Signatures are the exact official windows-sys bindings for these exports.
    let get_data: GetData = unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, GetData>(GetProcAddress(library, b"WTHelperProvDataFromStateData\0".as_ptr()).ok_or("Windows trust state API is unavailable")?) };
    let get_signer: GetSigner = unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, GetSigner>(GetProcAddress(library, b"WTHelperGetProvSignerFromChain\0".as_ptr()).ok_or("Windows signer API is unavailable")?) };
    let get_cert: GetCert = unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, GetCert>(GetProcAddress(library, b"WTHelperGetProvCertFromChain\0".as_ptr()).ok_or("Windows signer certificate API is unavailable")?) };
    // Official repr(C) SDK bindings are zero-initialized; all required sizes,
    // pointer lifetimes, file choice, and state actions are set below.
    let mut info: WINTRUST_FILE_INFO = unsafe { std::mem::zeroed() };
    info.cbStruct = std::mem::size_of::<WINTRUST_FILE_INFO>() as u32; info.pcwszFilePath = path_wide.as_ptr(); info.hFile = held.as_raw_handle();
    let mut data: WINTRUST_DATA = unsafe { std::mem::zeroed() };
    data.cbStruct = std::mem::size_of::<WINTRUST_DATA>() as u32; data.dwUIChoice = WTD_UI_NONE;
    data.fdwRevocationChecks = WTD_REVOKE_NONE; // Provider flag below checks the chain except the root.
    data.dwUnionChoice = WTD_CHOICE_FILE; data.Anonymous.pFile = &mut info;
    data.dwStateAction = WTD_STATEACTION_VERIFY;
    data.dwProvFlags = WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT | WTD_DISABLE_MD2_MD4 | WTD_MOTW;
    if mode == TrustMode::Cached { data.dwProvFlags |= WTD_CACHE_ONLY_URL_RETRIEVAL; }
    struct State { data: WINTRUST_DATA, action: windows_sys::core::GUID }
    impl Drop for State {
        fn drop(&mut self) { self.data.dwStateAction = WTD_STATEACTION_CLOSE; unsafe { WinVerifyTrust(INVALID_HANDLE_VALUE, &mut self.action, (&mut self.data as *mut WINTRUST_DATA).cast()); } }
    }
    let mut state = State { data, action: WINTRUST_ACTION_GENERIC_VERIFY_V2 };
    let status = unsafe { WinVerifyTrust(INVALID_HANDLE_VALUE, &mut state.action, (&mut state.data as *mut WINTRUST_DATA).cast()) };
    if status != 0 {
        return Err(format!("Windows rejected Microsoft runtime trust (0x{:08X}, {}). Check the system clock, Windows certificate/revocation availability and application-control policy, then retry. No security checks were relaxed.", status as u32, mode.name()));
    }
    // These pointers are owned by successful WinVerifyTrust state. Read only the
    // first actual signer and its leaf, before State's unconditional close.
    let provider = unsafe { get_data(state.data.hWVTStateData) };
    if provider.is_null() { return Err("Windows returned no verified signer state.".into()); }
    let signer = unsafe { get_signer(provider, 0, 0, 0) };
    if signer.is_null() || unsafe { (*signer).csCertChain } == 0 { return Err("Windows returned no verified certificate chain.".into()); }
    let cert = unsafe { get_cert(signer, 0) };
    if cert.is_null() || unsafe { (*cert).pCert }.is_null() { return Err("Windows returned no verified leaf certificate.".into()); }
    let context = unsafe { &*(*cert).pCert };
    if context.pbCertEncoded.is_null() || context.cbCertEncoded == 0 || context.cbCertEncoded > 64 * 1024 { return Err("Windows returned an invalid leaf certificate.".into()); }
    let der = unsafe { std::slice::from_raw_parts(context.pbCertEncoded, context.cbCertEncoded as usize) };
    let signer_sha256 = format!("{:x}", Sha256::digest(der));
    let expected = if pin.name == "installer" { EXE_SIGNER } else { DLL_SIGNER };
    check_signer(&signer_sha256, expected)?;
    Ok(Signature { schema: 1, role: pin.name.into(), mode: mode.name().into(), sha256: pin.sha.into(), signer_sha256 })
}

#[cfg(windows)]
fn print_signature_result(mode: &str, role_name: &str, path: &Path) -> i32 {
    let result = TrustMode::parse(mode).and_then(|mode| role(role_name).and_then(|pin| native_signature(path, pin, mode)));
    match result {
        Ok(result) => { println!("\n{PREFIX}{}", serde_json::to_string(&result).expect("fixed signature record")); 0 },
        Err(error) => { println!("\n{PREFIX}{}", serde_json::json!({"error":error})); 1 },
    }
}
/// Checked before Tauri initialization. This internal mode cannot install,
/// execute downloaded files, select arbitrary hashes, URLs, or trust policies.
#[cfg(windows)]
pub(crate) fn signature_helper_from_args() -> Option<i32> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).map_or(true, |v| v != HELPER) { return None; }
    if args.len() != 5 { return Some(2); }
    let (Some(mode), Some(role)) = (args[2].to_str(), args[3].to_str()) else { return Some(2); };
    Some(print_signature_result(mode, role, Path::new(&args[4])))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    fn temporary() -> PathBuf { let p = std::env::temp_dir().join(format!("luma-crt-{}",uuid::Uuid::new_v4())); fs::create_dir(&p).unwrap(); p }
    #[test]
    fn fixed_contract_matches_audited_lock() {
        let lock: serde_json::Value = serde_json::from_str(CONTRACT).unwrap();
        assert_eq!(lock["source_url"],URL); assert_eq!(lock["installer"]["sha256"],INSTALLER_SHA); assert_eq!(lock["installer"]["bytes"],DOWNLOAD_BYTES);
        for (offset,pin) in SLICES { assert_eq!(lock["containers"][pin.name]["offset"],offset); assert_eq!(lock["containers"][pin.name]["bytes"],pin.bytes); assert_eq!(lock["containers"][pin.name]["sha256"],pin.sha); }
        for pin in DLLS { assert_eq!(lock["dlls"][pin.name]["bytes"],pin.bytes); assert_eq!(lock["dlls"][pin.name]["sha256"],pin.sha); assert_eq!(lock["dlls"][pin.name]["member"],pin.member); }
        for pin in NOTICES { assert_eq!(lock["notices"][pin.name]["bytes"],pin.bytes); assert_eq!(lock["notices"][pin.name]["sha256"],pin.sha); }
        assert!(lock["source_manifest"].as_str().unwrap().contains("3956ea1dc1086c62fa6d2d05484952caee010646"));
        assert_eq!(lock["certificate_pins"]["installer"]["leaf_der_sha256"], EXE_SIGNER);
        assert_eq!(lock["certificate_pins"]["dlls"]["leaf_der_sha256"], DLL_SIGNER);
        assert!(check_signer(DLL_SIGNER, EXE_SIGNER).is_err());
        assert!(check_signer(DLL_SIGNER, DLL_SIGNER).is_ok());
    }
    #[test]
    fn only_fixed_roles_modes_members_and_microsoft_url_are_accepted() {
        assert!(role("msvcp140.dll").is_ok()); assert!(role("evil.dll").is_err()); assert!(TrustMode::parse("skip-revocation").is_err());
        for member in ["*", "../a12", "a?", "a13"] { assert!(selected_member(Pinned { member, ..MINIMUM }).is_err()); }
        assert!(super::super::catalog::validate_download(URL,DOWNLOAD_BYTES,INSTALLER_SHA,super::super::catalog::Source::MicrosoftCrt).is_ok());
        assert!(super::super::catalog::validate_download(&URL.replace("73aabf2e", "73aabf2f"),DOWNLOAD_BYTES,INSTALLER_SHA,super::super::catalog::Source::MicrosoftCrt).is_err());
    }
    #[test]
    fn cab_arguments_are_fixed_relative_names_inside_the_owned_unicode_directory() {
        let root = temporary(); let work = root.join("Managed runtime é 测试"); fs::create_dir(&work).unwrap();
        let work = fs::canonicalize(work).unwrap();
        #[cfg(windows)] assert!(matches!(work.components().next(), Some(std::path::Component::Prefix(p)) if matches!(p.kind(), std::path::Prefix::VerbatimDisk(_))));
        for (cabinet, members) in [(SLICES[1].1, vec![MINIMUM]), (MINIMUM, DLLS.to_vec()), (SLICES[0].1, NOTICES.to_vec())] {
            for member in members {
                let args = expansion_arguments(&work.join(cabinet.name), cabinet, member, &work).unwrap();
                assert_eq!(args, [cabinet.name.to_owned(), format!("-F:{}", member.member), format!("extract-{}", member.member)]);
                assert!(args.iter().all(|a| a.is_ascii() && !a.contains('/') && !a.contains('\\')));
                assert_eq!(work.join(&args[0]), work.join(cabinet.name));
                assert!(expansion_arguments(&root.join(cabinet.name), cabinet, member, &work).is_err());
                assert!(expansion_arguments(&work.join("other.cab"), cabinet, member, &work).is_err());
                assert!(expansion_arguments(&work.join(cabinet.name), Pinned { sha: "changed", ..cabinet }, member, &work).is_err());
                assert!(expansion_arguments(&work.join(cabinet.name), cabinet, Pinned { bytes: 1, ..member }, &work).is_err());
            }
        }
        assert!(expansion_arguments(&work.join("attached.cab"), SLICES[1].1, DLLS[0], &work).is_err());
        assert!(expansion_arguments(Path::new("relative/attached.cab"), SLICES[1].1, MINIMUM, Path::new("relative")).is_err());
        let traversal = work.join("..").join("direct-crt");
        assert!(expansion_arguments(&traversal.join("attached.cab"), SLICES[1].1, MINIMUM, &traversal).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cab_failure_keeps_native_exit_and_bounded_output_without_policy_assumptions() {
        #[cfg(windows)] use std::os::windows::process::ExitStatusExt;
        #[cfg(unix)] use std::os::unix::process::ExitStatusExt;
        let status = std::process::ExitStatus::from_raw(1);
        let message = expansion_failure(status, b"Cannot open input file.\r\n\x1b\x00", SLICES[1].1, MINIMUM);
        assert!(message.contains(&status.to_string())); assert!(message.contains("attached.cab member a12"));
        assert!(message.contains("Cannot open input file.")); assert!(!message.contains("policy")); assert!(!message.chars().any(char::is_control));
        assert!(expansion_failure(status, &vec![b'x'; 65536], SLICES[1].1, MINIMUM).len() < 2300);
        assert!(expansion_failure(status, &[], SLICES[1].1, MINIMUM).contains("No diagnostic output"));
    }
    #[test]
    fn fixed_copy_rejects_bounds_digest_and_cancellation() {
        let root = temporary(); let input = root.join("input"); fs::write(&input,b"four").unwrap();
        let pin = Pinned { name:"fixture",member:"",bytes:4,sha:"04efaf080f5a3e74e1c29d1ca6a48569382cbbcd324e8d59d2b83ef21c039f00" };
        let mut f = File::open(input).unwrap();
        assert!(slice(&mut f,4,1,pin,&root.join("bounds"),&AtomicBool::new(false)).is_err());
        assert!(slice(&mut f,4,u64::MAX,pin,&root.join("overflow"),&AtomicBool::new(false)).is_err());
        assert!(slice(&mut f,4,0,Pinned { sha: "0000000000000000000000000000000000000000000000000000000000000000", ..pin },&root.join("hash"),&AtomicBool::new(false)).is_err());
        assert!(slice(&mut f,4,0,pin,&root.join("valid"),&AtomicBool::new(false)).is_ok());
        assert_eq!(fs::read(root.join("valid")).unwrap(),b"four");
        assert!(slice(&mut f,4,0,pin,&root.join("valid"),&AtomicBool::new(false)).is_err());
        assert!(slice(&mut f,4,0,pin,&root.join("cancel"),&AtomicBool::new(true)).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    fn recipe_fixture() -> (Recipe, Runtime) {
        let terms = [
            ("microsoft-vc-runtime-en", "Cpp_2015-2022_ENU.1033", include_str!("../../resources/asr/terms/microsoft-vc-runtime-en.txt"), NOTICES[0]),
            ("microsoft-vc-runtime-zh-cn", "Cpp_2015-2022_CHS.2052", include_str!("../../resources/asr/terms/microsoft-vc-runtime-zh-cn.txt"), NOTICES[1]),
        ].into_iter().map(|(id,version,text,pin)| serde_json::json!({"id":id,"version":version,"text":text,"sha256":format!("{:x}",Sha256::digest(text.as_bytes())),"url":TERMS_URL,"raw_sha256":pin.sha,"source_encoding":"rtf"})).collect::<Vec<_>>();
        let runtime: Runtime = serde_json::from_value(serde_json::json!({
            "id":"fixture-runtime","version":"1","label":"Fixture runtime","platform":"windows-x64","engine":"whisper-accelerated","backend":"faster-whisper","device":"cpu","license":"Fixture","license_url":TERMS_URL,"installed_bytes":1024*1024,"max_files":100,"entrypoint":"python.exe",
            "recipe":{"schema":1,"windows_crt":ID,
                "python":{"url":"https://github.com/astral-sh/python-build-standalone/releases/download/20261003/cpython-3.12.15%2B20261003-x86_64-pc-windows-msvc-install_only_stripped.tar.gz","bytes":4,"sha256":"a".repeat(64),"installed_bytes":100000,"max_files":100,"entrypoint":"python.exe","site_packages":"Lib/site-packages","pip_version":"26.2.1"},
                "wheels":[{"name":"example","version":"1.0","filename":"example-1.0-py3-none-any.whl","url":format!("https://files.pythonhosted.org/packages/aa/bb/{}/example-1.0-py3-none-any.whl","a".repeat(64)),"bytes":4,"sha256":"a".repeat(64),"installed_bytes":1000,"max_files":20}],"terms":terms}
        })).unwrap();
        (runtime.recipe.clone().unwrap(),runtime)
    }
    #[test]
    fn fixed_terms_require_complete_exact_text_raw_provenance_and_platform() {
        let (recipe,runtime)=recipe_fixture(); recipe.validate(&runtime).unwrap();
        assert_eq!(recipe.download_bytes(),8+DOWNLOAD_BYTES);
        for field in ["id","version","sha256","raw_sha256","source_encoding","url","text"] {
            let mut value=serde_json::to_value(&recipe).unwrap(); value["terms"][0][field]=serde_json::json!("changed");
            let changed: Recipe=serde_json::from_value(value).unwrap(); assert!(validate_recipe(&changed,&runtime).is_err(),"accepted changed {field}");
        }
        let mut changed=recipe.clone(); changed.terms.remove(1); assert!(validate_recipe(&changed,&runtime).is_err());
        let mut changed=recipe.clone(); changed.terms.push(changed.terms[0].clone()); assert!(validate_recipe(&changed,&runtime).is_err());
        let mut changed=recipe.clone(); changed.windows_crt=Some("latest".into()); assert!(validate_recipe(&changed,&runtime).is_err());
        let mut changed_runtime=runtime; changed_runtime.platform="macos-arm64".into(); assert!(validate_recipe(&recipe,&changed_runtime).is_err());
    }
    #[test]
    fn fixed_contract_is_in_plan_and_full_terms_are_required_for_assent() {
        use super::super::{recipe,ComponentRequest};
        let (recipe,mut runtime)=recipe_fixture();
        let expected=format!("{:x}",Sha256::new().chain_update(serde_json::to_vec(&runtime).unwrap()).chain_update(CONTRACT.as_bytes()).finalize());
        let plan=recipe::plan_hash(&runtime).unwrap(); assert_eq!(plan,expected);
        let mut request=ComponentRequest{component_id:runtime.id.clone(),request_id:uuid::Uuid::new_v4().to_string(),plan_sha256:Some(plan.clone()),acknowledged_terms:recipe.acknowledgements()};
        assert!(recipe::validate_acknowledgement(&runtime,&request).is_ok());
        request.acknowledged_terms.pop(); assert!(recipe::validate_acknowledgement(&runtime,&request).is_err());
        request.acknowledged_terms=recipe.acknowledgements(); request.acknowledged_terms[0].sha256="0".repeat(64); assert!(recipe::validate_acknowledgement(&runtime,&request).is_err());
        runtime.recipe.as_mut().unwrap().windows_crt=None; assert_ne!(plan,recipe::plan_hash(&runtime).unwrap());
    }
    #[test]
    fn trust_result_rejects_unknown_fields_duplicate_records_and_wrong_pins() {
        let value=serde_json::json!({"schema":1,"role":"installer","mode":"online","sha256":INSTALLER_SHA,"signer_sha256":EXE_SIGNER});
        let record=format!("{PREFIX}{value}\n");
        assert!(signature_result(true,record.as_bytes(),INSTALLER,TrustMode::Online).is_ok());
        assert!(signature_result(true,format!("{record}{record}").as_bytes(),INSTALLER,TrustMode::Online).is_err());
        assert!(signature_result(true,record.as_bytes(),INSTALLER,TrustMode::Cached).is_err());
        for (field,changed) in [("schema",serde_json::json!(2)),("role",serde_json::json!("msvcp140.dll")),("sha256",serde_json::json!(DLLS[0].sha)),("signer_sha256",serde_json::json!(DLL_SIGNER)),("skip_revocation",serde_json::json!(true))] {
            let mut value=value.clone(); value[field]=changed;
            assert!(signature_result(true,format!("{PREFIX}{value}\n").as_bytes(),INSTALLER,TrustMode::Online).is_err(),"accepted {field}");
        }
        assert!(signature_result(false,format!("{PREFIX}{{\"error\":\"trust policy rejected\"}}\n").as_bytes(),INSTALLER,TrustMode::Online).unwrap_err().contains("trust policy rejected"));
    }
    #[test]
    fn wheel_data_cannot_replace_fixed_crt_files_or_notices() {
        let root=temporary(); let (mut recipe,_)=recipe_fixture();
        for relative in ["msvcp140.dll","MSVCP140_1.DLL","vcruntime140.dll","licenses/msvc-14.44.35211-x64/license-en.rtf","licenses/msvc-14.44.35211-x64/provenance.json"] {
            let file=File::create(root.join(&recipe.wheels[0].filename)).unwrap(); let mut zip=zip::ZipWriter::new(file);
            let options=zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).unix_permissions(0o644);
            for (name,text) in [("example-1.0.dist-info/METADATA".into(),"Name: example\nVersion: 1.0\n"),("example-1.0.dist-info/WHEEL".into(),"Wheel-Version: 1.0\nRoot-Is-Purelib: true\n"),("example-1.0.dist-info/RECORD".into(),""),(format!("example-1.0.data/data/{relative}"),"changed")] {
                zip.start_file(name,options).unwrap(); zip.write_all(text.as_bytes()).unwrap();
            }
            zip.finish().unwrap(); recipe.wheels[0].max_files=20; recipe.wheels[0].installed_bytes=10000;
            let reserved=vec![store::FileReceipt{path:relative.to_ascii_lowercase(),bytes:1,sha256:"a".repeat(64)}];
            let error=super::super::wheel_preflight::validate(&root,&recipe,&reserved,&AtomicBool::new(false)).unwrap_err(); assert!(error.contains("collision"),"{error}");
        }
        fs::remove_dir_all(root).unwrap();
    }
    // A test-harness child exercises the same native verifier. Production never
    // accepts these environment overrides and uses the signed app's real mode.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn native_signature_child() {
        let mode=std::env::var("LUMA_CRT_CHILD_MODE").unwrap_or_default(); let role=std::env::var("LUMA_CRT_CHILD_ROLE").unwrap_or_default(); let path=std::env::var_os("LUMA_CRT_CHILD_PATH").unwrap_or_default();
        std::process::exit(print_signature_result(&mode,&role,Path::new(&path)));
    }
    #[cfg(windows)]
    #[test]
    fn native_helper_rejects_unc_relative_and_unknown_filenames() {
        for path in [r"\\server\share\VC_redist.x64.exe",r"relative\VC_redist.x64.exe",r"C:\Windows\evil.exe"] { assert!(helper_path(Path::new(path),INSTALLER).is_err()); }
    }
}
