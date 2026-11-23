use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use flate2::read::GzDecoder;
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tar::Archive;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, RwLock, broadcast};
use uuid::Uuid;
use zip::ZipArchive;

use crate::cdp::{CdpTransport, WsCdpTransport};
use crate::types::{BotBrowserError, BotBrowserEvent, NormalizedOrigin};

pub const ENV_OBSCURA_BIN: &str = "BOT_BROWSER_OBSCURA_BIN";
pub const ENV_OBSCURA_DOWNLOAD_URL: &str = "BOT_BROWSER_OBSCURA_DOWNLOAD_URL";
pub const ENV_OBSCURA_SHA256: &str = "BOT_BROWSER_OBSCURA_SHA256";

pub const DEFAULT_OBSCURA_VERSION: &str = "0.2.7";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveKind {
    RawBinary,
    TarGz,
    Zip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObscuraReleaseArtifact {
    pub target: String,
    pub url: String,
    pub sha256: String,
    pub archive_kind: ArchiveKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObscuraBinaryManifest {
    pub version: String,
    pub artifacts: Vec<ObscuraReleaseArtifact>,
}

impl Default for ObscuraBinaryManifest {
    fn default() -> Self {
        Self::default_obscura()
    }
}

impl ObscuraBinaryManifest {
    pub fn default_obscura() -> Self {
        let ver = DEFAULT_OBSCURA_VERSION;
        let base = format!("https://github.com/h4ckf0r0day/obscura/releases/download/v{ver}");
        Self {
            version: ver.to_string(),
            artifacts: vec![
                ObscuraReleaseArtifact {
                    target: "aarch64-macos".to_string(),
                    url: format!("{base}/obscura-aarch64-macos-stealth.tar.gz"),
                    sha256: "3b05ebcd4755a325be224992fad6a038d86f7e5ab483e4c781f14de06cab973f"
                        .to_string(),
                    archive_kind: ArchiveKind::TarGz,
                },
                ObscuraReleaseArtifact {
                    target: "x86_64-macos".to_string(),
                    url: format!("{base}/obscura-x86_64-macos-stealth.tar.gz"),
                    sha256: "62bd094388a1f97480fd06be349299e721a3488c216c86a2fce9367310594ec3"
                        .to_string(),
                    archive_kind: ArchiveKind::TarGz,
                },
                ObscuraReleaseArtifact {
                    target: "x86_64-linux".to_string(),
                    url: format!("{base}/obscura-x86_64-linux-stealth.tar.gz"),
                    sha256: "1d6514beacef9fe75671f4bf7a66d3a7bceef9bde39f8156d1ca4a701eb1e216"
                        .to_string(),
                    archive_kind: ArchiveKind::TarGz,
                },
                ObscuraReleaseArtifact {
                    target: "aarch64-linux".to_string(),
                    url: format!("{base}/obscura-aarch64-linux-stealth.tar.gz"),
                    sha256: "f51167d7ae98be6c6327d3e7e015e74430e190ea6846589e3c131e10641a52b7"
                        .to_string(),
                    archive_kind: ArchiveKind::TarGz,
                },
                ObscuraReleaseArtifact {
                    target: "x86_64-windows".to_string(),
                    url: format!("{base}/obscura-x86_64-windows-stealth.zip"),
                    sha256: "ffae5e71cfca0108709b57ad83ce7d6b588eff6a15a43093b0cf867dbd18c867"
                        .to_string(),
                    archive_kind: ArchiveKind::Zip,
                },
            ],
        }
    }

    pub fn current_target() -> &'static str {
        match (std::env::consts::ARCH, std::env::consts::OS) {
            ("aarch64", "macos") => "aarch64-macos",
            ("x86_64", "macos") => "x86_64-macos",
            ("x86_64", "linux") => "x86_64-linux",
            ("aarch64", "linux") => "aarch64-linux",
            ("x86_64", "windows") => "x86_64-windows",
            (_, "macos") => "aarch64-macos",
            (_, "windows") => "x86_64-windows",
            _ => "x86_64-linux",
        }
    }

    pub fn artifact_for_target(&self, target: &str) -> Option<&ObscuraReleaseArtifact> {
        self.artifacts.iter().find(|a| a.target == target)
    }

    pub fn resolve_for_current_target(
        &self,
        url_override: Option<&str>,
        sha256_override: Option<&str>,
    ) -> Result<ObscuraReleaseArtifact, BotBrowserError> {
        let target = Self::current_target();
        let mut artifact = self
            .artifact_for_target(target)
            .cloned()
            .or_else(|| {
                url_override.map(|url| ObscuraReleaseArtifact {
                    target: target.to_string(),
                    url: url.to_string(),
                    sha256: String::new(),
                    archive_kind: infer_archive_kind(url),
                })
            })
            .ok_or_else(|| {
                BotBrowserError::SidecarStartFailed(format!(
                    "no Obscura release artifact found for target '{target}'"
                ))
            })?;

        if let Some(url) = url_override.map(str::trim).filter(|s| !s.is_empty()) {
            artifact.url = url.to_string();
            artifact.archive_kind = infer_archive_kind(url);
        }
        if let Some(sha) = sha256_override.map(str::trim).filter(|s| !s.is_empty()) {
            artifact.sha256 = sha.to_ascii_lowercase();
        } else {
            artifact.sha256 = artifact.sha256.trim().to_ascii_lowercase();
        }
        Ok(artifact)
    }

    pub fn with_env_overrides(&self) -> Result<ObscuraReleaseArtifact, BotBrowserError> {
        let url_override = read_nonempty_env(ENV_OBSCURA_DOWNLOAD_URL);
        let sha_override = read_nonempty_env(ENV_OBSCURA_SHA256);
        self.resolve_for_current_target(url_override.as_deref(), sha_override.as_deref())
    }
}

fn infer_archive_kind(url: &str) -> ArchiveKind {
    let lower = url.to_ascii_lowercase();
    let without_query = lower.split(['?', '#']).next().unwrap_or(&lower);
    if without_query.ends_with(".tar.gz") || without_query.ends_with(".tgz") {
        ArchiveKind::TarGz
    } else if without_query.ends_with(".zip") {
        ArchiveKind::Zip
    } else {
        ArchiveKind::RawBinary
    }
}

fn read_nonempty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

struct TempFileCleanup {
    path: Option<PathBuf>,
}

impl TempFileCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    fn path(&self) -> &Path {
        self.path.as_deref().unwrap_or_else(|| Path::new(""))
    }

    fn disarm(&mut self) {
        self.path = None;
    }
}

impl Drop for TempFileCleanup {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

pub struct ObscuraDownloader;

impl ObscuraDownloader {
    pub async fn ensure_binary(
        data_dir: &Path,
        manifest: &ObscuraBinaryManifest,
        client: &reqwest::Client,
    ) -> Result<PathBuf, BotBrowserError> {
        let env_bin = read_nonempty_env(ENV_OBSCURA_BIN);
        let env_url = read_nonempty_env(ENV_OBSCURA_DOWNLOAD_URL);
        let env_sha = read_nonempty_env(ENV_OBSCURA_SHA256);
        Self::ensure_binary_with_overrides(
            data_dir,
            manifest,
            client,
            env_bin.as_deref(),
            env_url.as_deref(),
            env_sha.as_deref(),
        )
        .await
    }

    pub async fn ensure_binary_with_overrides(
        data_dir: &Path,
        manifest: &ObscuraBinaryManifest,
        client: &reqwest::Client,
        bin_override: Option<&str>,
        url_override: Option<&str>,
        sha256_override: Option<&str>,
    ) -> Result<PathBuf, BotBrowserError> {
        if let Some(raw_bin) = bin_override.map(str::trim).filter(|s| !s.is_empty()) {
            let override_path = PathBuf::from(raw_bin);
            if is_executable_file(&override_path) {
                return Ok(override_path);
            }
            return Err(BotBrowserError::SidecarStartFailed(format!(
                "{ENV_OBSCURA_BIN} '{}' does not exist or is not executable",
                override_path.display()
            )));
        }

        let artifact = manifest.resolve_for_current_target(url_override, sha256_override)?;
        let safe_version = sanitize_slug(&manifest.version, DEFAULT_OBSCURA_VERSION);
        let bin_dir = data_dir.join("bin").join(&safe_version);
        fs::create_dir_all(&bin_dir)?;

        let bin_name = binary_filename_for_target(&artifact.target);
        let bin_path = bin_dir.join(bin_name);
        let stamp_path = bin_dir.join(format!("{bin_name}.sha256"));

        if bin_path.is_file() {
            if artifact.sha256.is_empty() {
                ensure_executable_permissions(&bin_path)?;
                return Ok(bin_path);
            }
            if verify_cached_binary(&bin_path, &stamp_path, &artifact)? {
                ensure_executable_permissions(&bin_path)?;
                return Ok(bin_path);
            }
        }

        let download_tmp_path = bin_dir.join(format!(".obscura-download-{}.tmp", Uuid::new_v4()));
        let mut download_guard = TempFileCleanup::new(download_tmp_path.clone());

        let actual_sha256 = if let Some(sha) =
            try_parallel_range_download(client, &artifact.url, download_guard.path()).await?
        {
            sha
        } else {
            let mut resp = client
                .get(&artifact.url)
                .send()
                .await
                .map_err(|e| {
                    BotBrowserError::SidecarStartFailed(format!(
                        "failed downloading Obscura from '{}': {e}",
                        artifact.url
                    ))
                })?;

            if !resp.status().is_success() {
                return Err(BotBrowserError::SidecarStartFailed(format!(
                    "Obscura download '{}' returned HTTP {}",
                    artifact.url,
                    resp.status()
                )));
            }

            let mut tmp_file = fs::File::create(download_guard.path())?;
            let mut hasher = Sha256::new();
            while let Some(chunk) = resp.chunk().await.map_err(|e| {
                BotBrowserError::SidecarStartFailed(format!(
                    "failed reading Obscura stream from '{}': {e}",
                    artifact.url
                ))
            })? {
                hasher.update(&chunk);
                tmp_file.write_all(&chunk)?;
            }
            tmp_file.flush()?;
            drop(tmp_file);
            hex::encode(hasher.finalize())
        };
        if !artifact.sha256.is_empty() && actual_sha256 != artifact.sha256 {
            let _ = fs::remove_file(download_guard.path());
            download_guard.disarm();
            return Err(BotBrowserError::SidecarChecksumMismatch {
                expected: artifact.sha256.clone(),
                actual: actual_sha256,
            });
        }

        let staged_tmp_path = bin_dir.join(format!(".obscura-staged-{}.tmp", Uuid::new_v4()));
        let mut staged_guard = TempFileCleanup::new(staged_tmp_path.clone());

        let bin_sha256 = match artifact.archive_kind {
            ArchiveKind::RawBinary => {
                fs::rename(download_guard.path(), staged_guard.path())?;
                download_guard.disarm();
                actual_sha256.clone()
            }
            ArchiveKind::TarGz => {
                let extracted_sha = extract_obscura_from_targz(
                    download_guard.path(),
                    staged_guard.path(),
                    bin_name,
                )?;
                let _ = fs::remove_file(download_guard.path());
                download_guard.disarm();
                extracted_sha
            }
            ArchiveKind::Zip => {
                let extracted_sha = extract_obscura_from_zip(
                    download_guard.path(),
                    staged_guard.path(),
                    bin_name,
                )?;
                let _ = fs::remove_file(download_guard.path());
                download_guard.disarm();
                extracted_sha
            }
        };

        ensure_executable_permissions(staged_guard.path())?;
        fs::rename(staged_guard.path(), &bin_path)?;
        staged_guard.disarm();

        if !artifact.sha256.is_empty() {
            let _ = fs::write(&stamp_path, format!("{}:{}\n", artifact.sha256, bin_sha256));
        }

        Ok(bin_path)
    }
}

fn binary_filename_for_target(target: &str) -> &'static str {
    if target.contains("windows") || cfg!(windows) {
        "obscura.exe"
    } else {
        "obscura"
    }
}

fn verify_cached_binary(
    bin_path: &Path,
    stamp_path: &Path,
    artifact: &ObscuraReleaseArtifact,
) -> Result<bool, BotBrowserError> {
    let actual_bin_sha = sha256_file(bin_path)?;
    if actual_bin_sha == artifact.sha256 {
        return Ok(true);
    }

    if matches!(artifact.archive_kind, ArchiveKind::TarGz | ArchiveKind::Zip) {
        if let Ok(stamp) = fs::read_to_string(stamp_path) {
            if let Some((recorded_archive, recorded_bin)) = stamp.trim().split_once(':') {
                if recorded_archive.eq_ignore_ascii_case(&artifact.sha256)
                    && recorded_bin.eq_ignore_ascii_case(&actual_bin_sha)
                {
                    return Ok(true);
                }
            }
        }
    }

    Ok(false)
}

fn sha256_file(path: &Path) -> Result<String, BotBrowserError> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 32 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

const PARALLEL_DOWNLOAD_MIN_BYTES: u64 = 4 * 1024 * 1024;
const PARALLEL_DOWNLOAD_MAX_BYTES: u64 = 512 * 1024 * 1024;
const PARALLEL_DOWNLOAD_CHUNKS: u64 = 16;

async fn try_parallel_range_download(
    client: &reqwest::Client,
    url: &str,
    dest_path: &Path,
) -> Result<Option<String>, BotBrowserError> {
    let Ok(head_resp) = client
        .head(url)
        .timeout(Duration::from_secs(15))
        .send()
        .await
    else {
        return Ok(None);
    };
    if !head_resp.status().is_success() {
        return Ok(None);
    }

    let accepts_ranges = head_resp
        .headers()
        .get(reqwest::header::ACCEPT_RANGES)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().contains("bytes"));
    let total_len = head_resp
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);

    if !accepts_ranges
        || !(PARALLEL_DOWNLOAD_MIN_BYTES..=PARALLEL_DOWNLOAD_MAX_BYTES).contains(&total_len)
    {
        return Ok(None);
    }

    let resolved_url = head_resp.url().as_str().to_string();
    let file = fs::File::create(dest_path)?;
    file.set_len(total_len)?;
    let shared_file = Arc::new(Mutex::new(file));

    let chunk_size = total_len.div_ceil(PARALLEL_DOWNLOAD_CHUNKS);
    let mut ranges = Vec::new();
    let mut start = 0u64;
    while start < total_len {
        let end = (start + chunk_size - 1).min(total_len - 1);
        ranges.push((start, end));
        start = end + 1;
    }

    let results: Vec<Result<(), ()>> = stream::iter(ranges)
        .map(|(range_start, range_end)| {
            let client = client.clone();
            let target_url = resolved_url.clone();
            let file_ref = Arc::clone(&shared_file);
            async move {
                let expected_bytes = range_end - range_start + 1;
                for _attempt in 0..3 {
                    let Ok(mut resp) = client
                        .get(&target_url)
                        .header(
                            reqwest::header::RANGE,
                            format!("bytes={range_start}-{range_end}"),
                        )
                        .send()
                        .await
                    else {
                        continue;
                    };
                    if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                        continue;
                    }
                    let mut offset = range_start;
                    let mut received = 0u64;
                    let mut stream_ok = true;
                    loop {
                        match resp.chunk().await {
                            Ok(Some(chunk)) => {
                                let mut guard = file_ref.lock().await;
                                if guard.seek(SeekFrom::Start(offset)).is_err()
                                    || guard.write_all(&chunk).is_err()
                                {
                                    return Err(());
                                }
                                let n = chunk.len() as u64;
                                offset += n;
                                received += n;
                            }
                            Ok(None) => break,
                            Err(_) => {
                                stream_ok = false;
                                break;
                            }
                        }
                    }
                    if stream_ok && received == expected_bytes {
                        return Ok(());
                    }
                }
                Err(())
            }
        })
        .buffer_unordered(PARALLEL_DOWNLOAD_CHUNKS as usize)
        .collect()
        .await;

    if results.iter().any(Result::is_err) {
        drop(shared_file);
        let _ = fs::remove_file(dest_path);
        return Ok(None);
    }

    {
        let mut guard = shared_file.lock().await;
        guard.flush()?;
    }
    drop(shared_file);
    let sha = sha256_file(dest_path)?;
    Ok(Some(sha))
}

fn score_obscura_archive_entry(
    file_name: &str,
    expected_bin_name: &str,
    unix_mode: Option<u32>,
) -> Option<u8> {
    if file_name.is_empty() {
        return None;
    }
    let lower = file_name.to_ascii_lowercase();
    for ignored_ext in [
        ".txt", ".md", ".json", ".sha256", ".sig", ".asc", ".toml", ".yaml", ".yml", ".plist",
    ] {
        if lower.ends_with(ignored_ext) {
            return None;
        }
    }

    if file_name == expected_bin_name {
        return Some(100);
    }
    if file_name == "obscura" || file_name == "obscura.exe" {
        return Some(90);
    }
    if lower.starts_with("obscura") {
        let expects_exe = expected_bin_name.ends_with(".exe");
        if expects_exe && !lower.ends_with(".exe") {
            return None;
        }
        let mut score = 70u8;
        if unix_mode.is_some_and(|m| (m & 0o111) != 0) {
            score = score.saturating_add(10);
        }
        if !lower.contains('.') || lower.ends_with(".exe") {
            score = score.saturating_add(5);
        }
        return Some(score);
    }
    None
}

fn copy_and_hash_reader<R: Read>(
    mut reader: R,
    dest_path: &Path,
) -> Result<String, BotBrowserError> {
    let mut out = fs::File::create(dest_path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 32 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n])?;
    }
    out.flush()?;
    Ok(hex::encode(hasher.finalize()))
}

fn extract_obscura_from_targz(
    archive_path: &Path,
    dest_path: &Path,
    expected_bin_name: &str,
) -> Result<String, BotBrowserError> {
    let best_idx: Option<usize> = {
        let file = fs::File::open(archive_path)?;
        let gz = GzDecoder::new(file);
        let mut archive = Archive::new(gz);
        let mut best: Option<(u8, usize)> = None;

        for (idx, entry_res) in archive
            .entries()
            .map_err(|e| {
                BotBrowserError::SidecarStartFailed(format!(
                    "failed reading Obscura tar.gz entries: {e}"
                ))
            })?
            .enumerate()
        {
            let entry = entry_res.map_err(|e| {
                BotBrowserError::SidecarStartFailed(format!("corrupt Obscura tar.gz entry: {e}"))
            })?;
            if !entry.header().entry_type().is_file() {
                continue;
            }
            let path = entry.path().map_err(|e| {
                BotBrowserError::SidecarStartFailed(format!("invalid tar entry path: {e}"))
            })?;
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let mode = entry.header().mode().ok();
            if let Some(score) = score_obscura_archive_entry(file_name, expected_bin_name, mode) {
                if best.is_none_or(|(best_score, _)| score > best_score) {
                    best = Some((score, idx));
                }
            }
        }
        best.map(|(_, idx)| idx)
    };

    let target_idx = best_idx.ok_or_else(|| {
        BotBrowserError::SidecarStartFailed(format!(
            "tar.gz archive did not contain '{expected_bin_name}' binary"
        ))
    })?;

    let file = fs::File::open(archive_path)?;
    let gz = GzDecoder::new(file);
    let mut archive = Archive::new(gz);
    for (idx, entry_res) in archive
        .entries()
        .map_err(|e| {
            BotBrowserError::SidecarStartFailed(format!(
                "failed reading Obscura tar.gz entries: {e}"
            ))
        })?
        .enumerate()
    {
        let entry = entry_res.map_err(|e| {
            BotBrowserError::SidecarStartFailed(format!("corrupt Obscura tar.gz entry: {e}"))
        })?;
        if idx == target_idx {
            return copy_and_hash_reader(entry, dest_path);
        }
    }

    Err(BotBrowserError::SidecarStartFailed(format!(
        "tar.gz archive entry {target_idx} vanished during extraction"
    )))
}

fn extract_obscura_from_zip(
    archive_path: &Path,
    dest_path: &Path,
    expected_bin_name: &str,
) -> Result<String, BotBrowserError> {
    let file = fs::File::open(archive_path)?;
    let mut archive = ZipArchive::new(file).map_err(|e| {
        BotBrowserError::SidecarStartFailed(format!("failed reading Obscura zip archive: {e}"))
    })?;

    let mut best: Option<(u8, usize)> = None;
    for idx in 0..archive.len() {
        let entry = archive.by_index(idx).map_err(|e| {
            BotBrowserError::SidecarStartFailed(format!("corrupt Obscura zip entry: {e}"))
        })?;
        if !entry.is_file() {
            continue;
        }
        let Some(enclosed) = entry.enclosed_name() else {
            continue;
        };
        let file_name = enclosed
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let mode = entry.unix_mode();
        if let Some(score) = score_obscura_archive_entry(file_name, expected_bin_name, mode) {
            if best.is_none_or(|(best_score, _)| score > best_score) {
                best = Some((score, idx));
            }
        }
    }

    let (_, target_idx) = best.ok_or_else(|| {
        BotBrowserError::SidecarStartFailed(format!(
            "zip archive did not contain '{expected_bin_name}' binary"
        ))
    })?;

    let entry = archive.by_index(target_idx).map_err(|e| {
        BotBrowserError::SidecarStartFailed(format!("failed opening Obscura zip entry: {e}"))
    })?;
    copy_and_hash_reader(entry, dest_path)
}

fn ensure_executable_permissions(path: &Path) -> Result<(), BotBrowserError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(path, perms)?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(path) {
            return (meta.permissions().mode() & 0o111) != 0;
        }
        false
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub struct ProfileNamespace;

impl ProfileNamespace {
    pub fn storage_dir_for(
        base_dir: &Path,
        namespace: &str,
        origin: Option<&NormalizedOrigin>,
    ) -> PathBuf {
        let ns_slug = sanitize_slug(namespace, "default");
        let dir_name = match origin {
            Some(orig) => {
                let host_slug = sanitize_slug(orig.host(), "origin");
                let mut hasher = Sha256::new();
                hasher.update(orig.as_str().as_bytes());
                let digest = hex::encode(hasher.finalize());
                let short_sha = &digest[..12];
                format!("{ns_slug}__{host_slug}_{short_sha}")
            }
            None => format!("{ns_slug}__global"),
        };
        base_dir.join("profiles").join(dir_name)
    }
}

fn sanitize_slug(raw: &str, fallback: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_underscore = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '.' {
            out.push(ch.to_ascii_lowercase());
            prev_underscore = false;
        } else if !prev_underscore && !out.is_empty() {
            out.push('_');
            prev_underscore = true;
        }
    }
    let trimmed = out.trim_matches(['_', '.']);
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarConfig {
    pub binary_path: PathBuf,
    pub storage_dir: PathBuf,
    pub extra_args: Vec<String>,
    pub command_timeout: Duration,
    pub startup_timeout: Duration,
    pub heartbeat_interval: Duration,
    pub max_auto_restarts: u32,
}

impl SidecarConfig {
    pub fn new(binary_path: impl Into<PathBuf>, storage_dir: impl Into<PathBuf>) -> Self {
        Self {
            binary_path: binary_path.into(),
            storage_dir: storage_dir.into(),
            extra_args: Vec::new(),
            command_timeout: Duration::from_secs(15),
            startup_timeout: Duration::from_secs(10),
            heartbeat_interval: Duration::from_secs(5),
            max_auto_restarts: 5,
        }
    }

    pub fn with_command_timeout(mut self, timeout: Duration) -> Self {
        self.command_timeout = timeout;
        self
    }

    pub fn with_startup_timeout(mut self, timeout: Duration) -> Self {
        self.startup_timeout = timeout;
        self
    }

    pub fn with_heartbeat_interval(mut self, interval: Duration) -> Self {
        self.heartbeat_interval = interval;
        self
    }

    pub fn with_max_auto_restarts(mut self, max_restarts: u32) -> Self {
        self.max_auto_restarts = max_restarts;
        self
    }

    pub fn with_extra_args(mut self, extra_args: Vec<String>) -> Self {
        self.extra_args = extra_args;
        self
    }
}

struct SidecarProcessState {
    child: Child,
    port: u16,
    endpoint: String,
    transport: Arc<WsCdpTransport>,
    generation: u64,
}

pub struct ObscuraSidecar {
    config: SidecarConfig,
    session_id: String,
    event_tx: Option<broadcast::Sender<BotBrowserEvent>>,
    last_url: RwLock<Option<String>>,
    restart_count: AtomicU64,
    generation: AtomicU64,
    shutting_down: AtomicBool,
    http_client: reqwest::Client,
    runtime: Mutex<Option<SidecarProcessState>>,
    lifecycle_lock: Mutex<()>,
}

impl ObscuraSidecar {
    pub async fn start(
        config: SidecarConfig,
        session_id: impl Into<String>,
        event_tx: Option<broadcast::Sender<BotBrowserEvent>>,
    ) -> Result<Self, BotBrowserError> {
        let http_client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .map_err(|e| BotBrowserError::SidecarStartFailed(format!("http client init failed: {e}")))?;

        let sidecar = Self {
            config,
            session_id: session_id.into(),
            event_tx,
            last_url: RwLock::new(None),
            restart_count: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            shutting_down: AtomicBool::new(false),
            http_client,
            runtime: Mutex::new(None),
            lifecycle_lock: Mutex::new(()),
        };

        let initial_state = sidecar.spawn_process(1).await?;
        sidecar.generation.store(1, Ordering::Release);
        *sidecar.runtime.lock().await = Some(initial_state);
        Ok(sidecar)
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn config(&self) -> &SidecarConfig {
        &self.config
    }

    pub fn restart_count(&self) -> u64 {
        self.restart_count.load(Ordering::Acquire)
    }

    pub async fn last_url(&self) -> Option<String> {
        self.last_url.read().await.clone()
    }

    pub async fn current_port(&self) -> Option<u16> {
        self.runtime.lock().await.as_ref().map(|s| s.port)
    }

    pub async fn current_endpoint(&self) -> Result<String, BotBrowserError> {
        self.runtime
            .lock()
            .await
            .as_ref()
            .map(|s| s.endpoint.clone())
            .ok_or_else(|| BotBrowserError::SidecarStartFailed("Obscura sidecar is not running".to_string()))
    }

    pub async fn is_running(&self) -> bool {
        if self.shutting_down.load(Ordering::Acquire) {
            return false;
        }
        let mut guard = self.runtime.lock().await;
        match guard.as_mut() {
            Some(state) => matches!(state.child.try_wait(), Ok(None)),
            None => false,
        }
    }

    pub async fn send_cdp_with_watchdog(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Value, BotBrowserError> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(BotBrowserError::SidecarStartFailed(
                "Obscura sidecar has been shut down".to_string(),
            ));
        }

        let timeout_ms = self.config.command_timeout.as_millis() as u64;

        for attempt in 0..2 {
            let (generation, transport) = self.ensure_live_transport().await?;

            let fut = transport.send_command(method, params.clone());
            match tokio::time::timeout(self.config.command_timeout, fut).await {
                Ok(Ok(val)) => {
                    if method == "Page.navigate" {
                        if let Some(url) = params.get("url").and_then(|v| v.as_str()) {
                            *self.last_url.write().await = Some(url.to_string());
                        }
                    }
                    return Ok(val);
                }
                Ok(Err(err @ BotBrowserError::Cdp(_))) => {
                    // Application/CDP error from live child: propagate without restarting.
                    return Err(err);
                }
                Ok(Err(BotBrowserError::SidecarTimeout { .. })) | Err(_) => {
                    self.restart_and_recover_for_generation(Some(generation))
                        .await?;
                    return Err(BotBrowserError::SidecarTimeout {
                        method: method.to_string(),
                        timeout_ms,
                    });
                }
                Ok(Err(err)) => {
                    // Transport disconnect / dead child: recover and retry once on fresh process.
                    self.restart_and_recover_for_generation(Some(generation))
                        .await?;
                    if attempt == 1 {
                        return Err(err);
                    }
                }
            }
        }

        Err(BotBrowserError::SidecarStartFailed(
            "Obscura watchdog failed after recovery retry".to_string(),
        ))
    }

    pub async fn restart_and_recover(&self) -> Result<(), BotBrowserError> {
        self.restart_and_recover_for_generation(None).await
    }

    async fn ensure_live_transport(&self) -> Result<(u64, Arc<WsCdpTransport>), BotBrowserError> {
        let needs_restart_gen = {
            let mut guard = self.runtime.lock().await;
            match guard.as_mut() {
                Some(state) => match state.child.try_wait() {
                    Ok(None) => {
                        return Ok((state.generation, Arc::clone(&state.transport)));
                    }
                    Ok(Some(_)) | Err(_) => Some(state.generation),
                },
                None => Some(self.generation.load(Ordering::Acquire)),
            }
        };

        self.restart_and_recover_for_generation(needs_restart_gen)
            .await?;

        let guard = self.runtime.lock().await;
        let state = guard.as_ref().ok_or_else(|| {
            BotBrowserError::SidecarStartFailed("Obscura sidecar unavailable after restart".to_string())
        })?;
        Ok((state.generation, Arc::clone(&state.transport)))
    }

    async fn restart_and_recover_for_generation(
        &self,
        expected_generation: Option<u64>,
    ) -> Result<(), BotBrowserError> {
        let _lifecycle = self.lifecycle_lock.lock().await;

        if self.shutting_down.load(Ordering::Acquire) {
            return Err(BotBrowserError::SidecarStartFailed(
                "Obscura sidecar is shutting down".to_string(),
            ));
        }

        if let Some(expected_gen) = expected_generation {
            let current_gen = self.generation.load(Ordering::Acquire);
            if current_gen != expected_gen {
                let mut guard = self.runtime.lock().await;
                if let Some(state) = guard.as_mut() {
                    if matches!(state.child.try_wait(), Ok(None)) {
                        return Ok(());
                    }
                }
            }
        }

        if let Some(mut old_state) = self.runtime.lock().await.take() {
            let _ = old_state.child.kill().await;
            let _ = old_state.child.wait().await;
        }

        let current_restarts = self.restart_count.load(Ordering::Acquire);
        if current_restarts >= self.config.max_auto_restarts as u64 {
            return Err(BotBrowserError::SidecarStartFailed(format!(
                "exceeded max_auto_restarts ({})",
                self.config.max_auto_restarts
            )));
        }

        let new_restart_count = self.restart_count.fetch_add(1, Ordering::AcqRel) + 1;
        let next_gen = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let new_state = self.spawn_process(next_gen).await?;
        let new_transport = Arc::clone(&new_state.transport);
        *self.runtime.lock().await = Some(new_state);

        let saved_url = self.last_url.read().await.clone();
        let mut restored_url = None;
        if let Some(ref url) = saved_url {
            match tokio::time::timeout(
                self.config.command_timeout,
                new_transport.send_command("Page.navigate", json!({ "url": url })),
            )
            .await
            {
                Ok(Ok(_)) => {
                    restored_url = Some(url.clone());
                }
                Ok(Err(e)) => {
                    tracing::warn!(
                        session_id = %self.session_id,
                        url = %url,
                        error = %e,
                        "failed to restore last_url after Obscura restart"
                    );
                }
                Err(_) => {
                    tracing::warn!(
                        session_id = %self.session_id,
                        url = %url,
                        "timed out restoring last_url after Obscura restart"
                    );
                }
            }
        }

        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(BotBrowserEvent::SidecarRestarted {
                session_id: self.session_id.clone(),
                restart_count: new_restart_count,
                restored_url,
            });
        }

        Ok(())
    }

    pub fn spawn_heartbeat(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let weak = Arc::downgrade(self);
        let interval = self.config.heartbeat_interval.max(Duration::from_millis(50));
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let Some(sidecar) = weak.upgrade() else {
                    break;
                };
                if sidecar.shutting_down.load(Ordering::Acquire) {
                    break;
                }
                let dead_gen = {
                    let mut guard = sidecar.runtime.lock().await;
                    match guard.as_mut() {
                        Some(state) => match state.child.try_wait() {
                            Ok(None) => None,
                            _ => Some(state.generation),
                        },
                        None => Some(sidecar.generation.load(Ordering::Acquire)),
                    }
                };
                if let Some(gen_id) = dead_gen {
                    let _ = sidecar
                        .restart_and_recover_for_generation(Some(gen_id))
                        .await;
                }
            }
        })
    }

    pub async fn shutdown(&self) -> Result<(), BotBrowserError> {
        self.shutting_down.store(true, Ordering::Release);
        let _lifecycle = self.lifecycle_lock.lock().await;
        if let Some(mut state) = self.runtime.lock().await.take() {
            // Ask Obscura to flush session cookies/storage to `--storage-dir` before exit.
            let _ = tokio::time::timeout(
                Duration::from_millis(250),
                state.transport.send_command("Browser.close", json!({})),
            )
            .await;

            #[cfg(unix)]
            {
                if let Some(pid) = state.child.id() {
                    let _ = Command::new("kill")
                        .args(["-TERM", &pid.to_string()])
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status()
                        .await;
                    if tokio::time::timeout(Duration::from_secs(2), state.child.wait())
                        .await
                        .is_ok()
                    {
                        return Ok(());
                    }
                }
            }

            let _ = state.child.kill().await;
            let _ = state.child.wait().await;
        }
        Ok(())
    }

    async fn spawn_process(&self, generation: u64) -> Result<SidecarProcessState, BotBrowserError> {
        fs::create_dir_all(&self.config.storage_dir)?;
        let port = pick_free_loopback_port()?;
        let endpoint = format!("http://127.0.0.1:{port}");

        let mut cmd = Command::new(&self.config.binary_path);
        cmd.arg("serve")
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(port.to_string())
            .arg("--stealth")
            .arg("--storage-dir")
            .arg(&self.config.storage_dir)
            .args(&self.config.extra_args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| {
            BotBrowserError::SidecarStartFailed(format!(
                "failed spawning Obscura binary '{}': {e}",
                self.config.binary_path.display()
            ))
        })?;

        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(drain_stream_to_null(stdout));
        }
        let stderr_tail = Arc::new(parking_lot::Mutex::new(String::new()));
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(drain_stream_tail(stderr, Arc::clone(&stderr_tail)));
        }

        let deadline = tokio::time::Instant::now() + self.config.startup_timeout;
        let list_url = format!("{endpoint}/json/list");
        let version_url = format!("{endpoint}/json/version");

        loop {
            if let Ok(Some(status)) = child.try_wait() {
                let err_tail = stderr_tail.lock().clone();
                return Err(BotBrowserError::SidecarStartFailed(format!(
                    "Obscura exited prematurely with status {status}: {}",
                    err_tail.trim()
                )));
            }

            if probe_obscura_ready(&self.http_client, &list_url, &version_url).await {
                break;
            }

            if tokio::time::Instant::now() >= deadline {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let err_tail = stderr_tail.lock().clone();
                return Err(BotBrowserError::SidecarStartFailed(format!(
                    "Obscura did not become ready at {endpoint} within {:?}: {}",
                    self.config.startup_timeout,
                    err_tail.trim()
                )));
            }

            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        let transport = Arc::new(WsCdpTransport::with_client(
            endpoint.clone(),
            self.http_client.clone(),
            self.config.command_timeout,
        ));

        Ok(SidecarProcessState {
            child,
            port,
            endpoint,
            transport,
            generation,
        })
    }
}

fn pick_free_loopback_port() -> Result<u16, BotBrowserError> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

async fn probe_obscura_ready(
    client: &reqwest::Client,
    list_url: &str,
    version_url: &str,
) -> bool {
    if let Ok(resp) = client
        .get(list_url)
        .timeout(Duration::from_millis(500))
        .send()
        .await
    {
        if resp.status().is_success() {
            if let Ok(val) = resp.json::<Value>().await {
                if val
                    .as_array()
                    .is_some_and(|arr| arr.iter().any(|t| t.get("webSocketDebuggerUrl").is_some()))
                {
                    return true;
                }
            }
        }
    }

    if let Ok(resp) = client
        .get(version_url)
        .timeout(Duration::from_millis(500))
        .send()
        .await
    {
        if resp.status().is_success() {
            return true;
        }
    }

    false
}

async fn drain_stream_to_null<R: AsyncReadExt + Unpin>(mut reader: R) {
    let mut buf = [0u8; 2048];
    while let Ok(n) = reader.read(&mut buf).await {
        if n == 0 {
            break;
        }
    }
}

async fn drain_stream_tail<R: AsyncReadExt + Unpin>(
    mut reader: R,
    tail: Arc<parking_lot::Mutex<String>>,
) {
    let mut buf = [0u8; 2048];
    while let Ok(n) = reader.read(&mut buf).await {
        if n == 0 {
            break;
        }
        let chunk = String::from_utf8_lossy(&buf[..n]);
        let mut guard = tail.lock();
        guard.push_str(&chunk);
        if guard.len() > 4096 {
            let cut = guard.len() - 4096;
            let safe_cut = guard.ceil_char_boundary(cut);
            guard.drain(..safe_cut);
        }
    }
}
