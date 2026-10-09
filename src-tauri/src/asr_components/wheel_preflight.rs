//! Strict destination planning before pip runs. Every destination is confined to
//! the fresh private prefix; collisions cannot overwrite the interpreter/tooling.
use super::{archive, recipe::{Recipe, Wheel}, store::FileReceipt};
use sha2::{Digest, Sha256};
use std::{collections::{HashMap, HashSet}, fs::File, io::Read, path::Path, sync::atomic::AtomicBool};

#[derive(Default)]
struct Destinations { files: HashMap<String, Destination>, directories: HashSet<String> }
struct Destination { owner: String, shared: Option<SharedMlxFile> }
#[derive(PartialEq, Eq)]
struct SharedMlxFile { path: String, bytes: u64, sha256: String, mode: Option<u32> }
impl Destinations {
    fn add(&mut self, path: &str, owner: &str, replace_python: bool) -> Result<(), String> {
        self.add_file(path, owner, replace_python, None)
    }
    fn add_file(&mut self, path: &str, owner: &str, replace_python: bool, shared: Option<SharedMlxFile>) -> Result<(), String> {
        archive::relative_path(path)?;
        // The exact supported wheel locks use ASCII names. Reject Unicode
        // normalization aliases instead of relying on filesystem-specific rules.
        if !path.is_ascii() { return Err("Pinned wheel contains an unreviewed non-ASCII destination.".into()); }
        let key = path.to_ascii_lowercase();
        if self.directories.contains(&key) { return Err("A pinned wheel file would replace a destination directory.".into()); }
        if let Some(previous) = self.files.get(&key) {
            let identical_mlx = matches!((previous.owner.as_str(), owner), ("mlx", "mlx-metal") | ("mlx-metal", "mlx")) && shared.is_some() && previous.shared.as_ref() == shared.as_ref();
            if !(replace_python && previous.owner == "private-python") && !identical_mlx { return Err(format!("Pinned wheel destination collision between {} and {owner}: {path}",previous.owner)); }
        }
        let mut parent = String::new(); let parts: Vec<_> = key.split('/').collect();
        for part in &parts[..parts.len()-1] {
            if !parent.is_empty() { parent.push('/'); } parent.push_str(part);
            if self.files.contains_key(&parent) { return Err("Pinned wheel destination crosses an existing file.".into()); }
            self.directories.insert(parent.clone());
        }
        self.files.insert(key, Destination { owner: owner.into(), shared }); Ok(())
    }
}
pub(super) fn validate(wheelhouse: &Path, recipe: &Recipe, python_files: &[FileReceipt], cancel: &AtomicBool) -> Result<(), String> {
    let mut destinations = Destinations::default();
    for file in python_files { destinations.add(&file.path, "private-python", false)?; }
    for wheel in &recipe.wheels {
        archive::cancelled(cancel)?;
        let path = wheelhouse.join(&wheel.filename);
        archive::inspect(&path, wheel.installed_bytes, wheel.max_files, cancel)?;
        let mut zip = zip::ZipArchive::new(File::open(&path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let mut info_dirs = HashSet::new();
        for index in 0..zip.len() {
            let file = zip.by_index(index).map_err(|e| e.to_string())?;
            if let Some(first) = file.name().split('/').next() { if first.ends_with(".dist-info") { info_dirs.insert(first.to_owned()); } }
        }
        if info_dirs.len() != 1 { return Err("Pinned wheel must contain exactly one distribution metadata directory.".into()); }
        let info = info_dirs.into_iter().next().ok_or("Missing wheel metadata")?;
        let prefix = info.strip_suffix(".dist-info").ok_or("Invalid wheel metadata directory")?;
        let (name, version) = prefix.rsplit_once('-').ok_or("Invalid wheel distribution identity")?;
        if normalize(name) != normalize(&wheel.name) || version != wheel.version { return Err("Wheel metadata directory differs from its pinned identity.".into()); }
        let metadata = small_text(&mut zip, &format!("{info}/METADATA"), 1024 * 1024)?;
        if header(&metadata,"Name").map(normalize) != Some(normalize(&wheel.name)) || header(&metadata,"Version") != Some(wheel.version.as_str()) { return Err("Wheel METADATA differs from the pinned package name or version.".into()); }
        let format = small_text(&mut zip, &format!("{info}/WHEEL"), 64 * 1024)?;
        if !header(&format,"Wheel-Version").is_some_and(|v| v == "1.0" || v == "1.1") || !matches!(header(&format,"Root-Is-Purelib"),Some("true"|"false")) { return Err("Unsupported pinned wheel format.".into()); }
        if zip.by_name(&format!("{info}/RECORD")).is_err() { return Err("Pinned wheel is missing its file RECORD.".into()); }
        let site = &recipe.python.site_packages;
        let data_root = format!("{prefix}.data");
        for index in 0..zip.len() {
            archive::cancelled(cancel)?;
            let mut file = zip.by_index(index).map_err(|e| e.to_string())?;
            if file.is_dir() { continue; }
            let name = file.name().to_owned();
            let destination = destination(&name, &data_root, site)?;
            let normalized_destination = destination.to_ascii_lowercase();
            let normalized_site = format!("{site}/").to_ascii_lowercase();
            let relative_site = normalized_destination.strip_prefix(&normalized_site);
            if let Some(relative) = relative_site {
                let first = relative.split('/').next().unwrap_or("");
                if (first == "pip" || first == "pip.py") || first.starts_with("pip-") || matches!(relative,"sitecustomize.py"|"usercustomize.py") { return Err("Wheel attempts to replace private installer/startup code.".into()); }
                if relative.ends_with(".pth") {
                    if file.size() > 64*1024 { return Err("Oversized wheel startup-path file.".into()); }
                    let mut text = String::new(); file.read_to_string(&mut text).map_err(|e| e.to_string())?;
                    if text.lines().any(|line| { let line = line.trim(); !line.is_empty() && !line.starts_with('#') && (line.starts_with("import ") || line.starts_with("import\t") || archive::relative_path(line).is_err()) }) && !(normalize(&wheel.name) == "setuptools" && relative == "distutils-precedence.pth") {
                        return Err("Wheel contains an unreviewed executable or escaping startup hook.".into());
                    }
                }
            }
            let replace = normalize(&wheel.name) == "setuptools" && relative_site.is_some_and(|p| p.starts_with("setuptools/") || p.starts_with("setuptools-") || p.starts_with("_distutils_hack/") || p.starts_with("pkg_resources/") || p == "distutils-precedence.pth");
            let shared = if matches!(wheel.name.as_str(), "mlx" | "mlx-metal") && wheel.version == "0.29.3" && reviewed_mlx_path(destination.strip_prefix(&format!("{site}/"))) {
                let expected = file.size(); let mode = file.unix_mode(); let mut bytes = 0u64; let mut digest = Sha256::new(); let mut buffer = [0u8; 64 * 1024];
                loop {
                    archive::cancelled(cancel)?;
                    let n = file.read(&mut buffer).map_err(|e| e.to_string())?; if n == 0 { break; }
                    bytes = bytes.checked_add(n as u64).ok_or("MLX shared file size overflow")?;
                    if bytes > expected { return Err("MLX shared file exceeds its archive size.".into()); }
                    digest.update(&buffer[..n]);
                }
                if bytes != expected { return Err("Truncated shared MLX wheel file.".into()); }
                Some(SharedMlxFile { path: destination.clone(), bytes, sha256: format!("{:x}",digest.finalize()), mode })
            } else { None };
            destinations.add_file(&destination, &wheel.name, replace, shared)?;
        }
        // Generated console launchers are also writes, even though assembly
        // removes them. Reserve their destinations before pip can start.
        if let Ok(text) = optional_small_text(&mut zip,&format!("{info}/entry_points.txt"),1024*1024) {
            if let Some(text) = text { reserve_launchers(&text,wheel,&recipe.python.site_packages,&mut destinations)?; }
        } else { return Err("Invalid wheel entry-point metadata.".into()); }
    }
    Ok(())
}
fn reviewed_mlx_path(path: Option<&str>) -> bool {
    matches!(path, Some("mlx/__main__.py" | "mlx/_os_warning.py" | "mlx/_reprlib_fix.py" | "mlx/distributed_run.py" | "mlx/extension.py" | "mlx/py.typed" | "mlx/utils.py"))
}
fn normalize(value: &str) -> String { value.to_ascii_lowercase().replace('_',"-").replace('.',"-") }
fn header<'a>(text: &'a str, name: &str) -> Option<&'a str> { text.lines().find_map(|line| line.split_once(':').filter(|(key,_)| *key == name).map(|(_,value)| value.trim())) }
fn small_text(zip: &mut zip::ZipArchive<File>, name: &str, cap: u64) -> Result<String,String> {
    optional_small_text(zip,name,cap)?.ok_or_else(|| format!("Pinned wheel is missing {name}"))
}
fn optional_small_text(zip: &mut zip::ZipArchive<File>,name:&str,cap:u64)->Result<Option<String>,String>{
    let mut file=match zip.by_name(name){Ok(file)=>file,Err(zip::result::ZipError::FileNotFound)=>return Ok(None),Err(e)=>return Err(e.to_string())};
    if file.size()>cap{return Err("Oversized wheel metadata".into());}
    let mut text=String::new(); file.read_to_string(&mut text).map_err(|e|e.to_string())?; Ok(Some(text))
}
fn destination(name:&str,data_root:&str,site:&str)->Result<String,String>{
    archive::relative_path(name)?;
    let parts:Vec<_>=name.split('/').collect();
    let destination=if parts[0].ends_with(".data"){
        if parts[0]!=data_root || parts.len()<3{return Err("Wheel .data directory identity or layout is invalid.".into());}
        let rest=parts[2..].join("/");
        match parts[1]{
            "purelib"|"platlib"=>format!("{site}/{rest}"),
            "data"=>rest,
            "scripts"=>format!("{}/{rest}",if site.starts_with("Lib/"){"Scripts"}else{"bin"}),
            _=>return Err("This exact recipe does not support wheel headers or an unknown .data scheme.".into()),
        }
    }else{format!("{site}/{name}")};
    archive::relative_path(&destination)?;
    if destination.to_ascii_lowercase().split('/').any(|p| p.starts_with(".assembly-") || p.starts_with(".luma-") || p.starts_with(".owned-")) { return Err("Wheel collides with owned setup metadata.".into()); }
    Ok(destination)
}
fn reserve_launchers(text:&str,wheel:&Wheel,site:&str,destinations:&mut Destinations)->Result<(),String>{
    let mut enabled=false;
    for raw in text.lines(){
        let line=raw.trim(); if line.starts_with('['){enabled=matches!(line,"[console_scripts]"|"[gui_scripts]");continue;}
        if !enabled || line.is_empty() || line.starts_with('#') {continue;}
        let (name,_)=line.split_once('=').ok_or("Invalid pinned wheel entry point")?;let name=name.trim();
        if archive::relative_path(name)?.components().count()!=1{return Err("Unsafe wheel launcher name".into());}
        if site.starts_with("Lib/"){destinations.add(&format!("Scripts/{name}.exe"),&wheel.name,false)?;}
        else{destinations.add(&format!("bin/{name}"),&wheel.name,false)?;}
    }
    Ok(())
}
#[cfg(test)]
pub(super) fn destination_for_test(name:&str,data_root:&str,site:&str)->Result<String,String>{destination(name,data_root,site)}
