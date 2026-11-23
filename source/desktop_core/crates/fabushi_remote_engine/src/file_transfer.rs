use crate::error::{EngineResult, RemoteEngineError};
use gpui_telegram_proto::remote_control_contracts::{
    FileConflictPolicy, FileTransferDirection, FileTransferStatus, FileTransferTaskDto,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub const DEFAULT_CHUNK_SIZE: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChunkFrame {
    pub transfer_id: String,
    pub offset: u64,
    pub data: Vec<u8>,
    pub sha256_hex: String,
    pub is_last: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedTransferState {
    dto: FileTransferTaskDto,
    source_size: u64,
    source_mtime_ms: i64,
    chunk_bitmap: Vec<bool>,
    resolved_dest_path: Option<String>,
}

fn sha256_hex_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub fn sha256_file_hex(path: &Path) -> EngineResult<String> {
    let mut f = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    Ok(s)
}

fn file_mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn chunk_count_for(total_bytes: u64, chunk_size: usize) -> usize {
    if total_bytes == 0 {
        return 1;
    }
    total_bytes.div_ceil(chunk_size as u64) as usize
}

/// Recursively collects `(relative_path, full_path, size)` for all regular files in a directory.
pub fn enumerate_directory_files(root: &Path) -> EngineResult<Vec<(String, PathBuf, u64)>> {
    let mut out = Vec::new();
    collect_dir_recursive(root, root, &mut out)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn collect_dir_recursive(
    base: &Path,
    current: &Path,
    out: &mut Vec<(String, PathBuf, u64)>,
) -> EngineResult<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            collect_dir_recursive(base, &path, out)?;
        } else if ft.is_file() {
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let size = entry.metadata()?.len();
            out.push((rel, path, size));
        }
    }
    Ok(())
}

pub struct FileTransferManager {
    state_dir: PathBuf,
    tasks: RwLock<HashMap<String, PersistedTransferState>>,
}

impl FileTransferManager {
    pub fn new(state_dir: PathBuf) -> EngineResult<Self> {
        std::fs::create_dir_all(&state_dir)?;
        let mut loaded = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(&state_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Ok(raw) = std::fs::read(&p) {
                        if let Ok(st) = serde_json::from_slice::<PersistedTransferState>(&raw) {
                            loaded.insert(st.dto.transfer_id.clone(), st);
                        }
                    }
                }
            }
        }
        Ok(Self {
            state_dir,
            tasks: RwLock::new(loaded),
        })
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    fn persist_state(&self, st: &PersistedTransferState) -> EngineResult<()> {
        let final_path = self.state_dir.join(format!("{}.json", st.dto.transfer_id));
        let tmp_path = self
            .state_dir
            .join(format!(".{}.{}.tmp", st.dto.transfer_id, uuid::Uuid::new_v4()));
        let bytes = serde_json::to_vec_pretty(st)
            .map_err(|e| RemoteEngineError::Other(format!("failed to serialize transfer state: {e}")))?;
        std::fs::write(&tmp_path, bytes)?;
        std::fs::rename(&tmp_path, &final_path)?;
        Ok(())
    }

    pub fn create_upload(
        &self,
        local_path: &Path,
        remote_path: &str,
        policy: FileConflictPolicy,
    ) -> EngineResult<FileTransferTaskDto> {
        let meta = std::fs::metadata(local_path)?;
        let total_bytes = meta.len();
        let mtime_ms = file_mtime_ms(&meta);
        let file_sha = sha256_file_hex(local_path)?;
        let now = chrono::Utc::now().timestamp_millis();
        let chunks = chunk_count_for(total_bytes, DEFAULT_CHUNK_SIZE);
        let dto = FileTransferTaskDto {
            transfer_id: uuid::Uuid::new_v4().to_string(),
            direction: FileTransferDirection::Upload,
            local_path: local_path.to_string_lossy().to_string(),
            remote_path: remote_path.to_string(),
            total_bytes,
            transferred_bytes: 0,
            chunk_size: DEFAULT_CHUNK_SIZE as u32,
            sha256: Some(file_sha),
            status: FileTransferStatus::Queued,
            conflict_policy: policy,
            error: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let state = PersistedTransferState {
            dto: dto.clone(),
            source_size: total_bytes,
            source_mtime_ms: mtime_ms,
            chunk_bitmap: vec![false; chunks],
            resolved_dest_path: None,
        };
        self.persist_state(&state)?;
        self.tasks.write().insert(dto.transfer_id.clone(), state);
        Ok(dto)
    }

    pub fn create_download(
        &self,
        remote_path: &str,
        local_path: &Path,
        total_bytes: u64,
        expected_sha256: Option<String>,
        policy: FileConflictPolicy,
    ) -> EngineResult<FileTransferTaskDto> {
        let resolved_dest = Self::resolve_conflict_path(local_path, policy)?;
        let now = chrono::Utc::now().timestamp_millis();
        let chunks = chunk_count_for(total_bytes, DEFAULT_CHUNK_SIZE);
        let status = if resolved_dest.is_none() {
            FileTransferStatus::Completed
        } else {
            FileTransferStatus::Queued
        };
        let dto = FileTransferTaskDto {
            transfer_id: uuid::Uuid::new_v4().to_string(),
            direction: FileTransferDirection::Download,
            local_path: resolved_dest
                .as_ref()
                .unwrap_or(&local_path.to_path_buf())
                .to_string_lossy()
                .to_string(),
            remote_path: remote_path.to_string(),
            total_bytes,
            transferred_bytes: if status == FileTransferStatus::Completed {
                total_bytes
            } else {
                0
            },
            chunk_size: DEFAULT_CHUNK_SIZE as u32,
            sha256: expected_sha256,
            status,
            conflict_policy: policy,
            error: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let state = PersistedTransferState {
            dto: dto.clone(),
            source_size: total_bytes,
            source_mtime_ms: 0,
            chunk_bitmap: vec![status == FileTransferStatus::Completed; chunks],
            resolved_dest_path: resolved_dest.map(|p| p.to_string_lossy().to_string()),
        };
        self.persist_state(&state)?;
        self.tasks.write().insert(dto.transfer_id.clone(), state);
        Ok(dto)
    }

    pub fn check_source_unchanged(&self, transfer_id: &str) -> EngineResult<bool> {
        let guard = self.tasks.read();
        let st = guard
            .get(transfer_id)
            .ok_or_else(|| RemoteEngineError::NotFound(format!("transfer '{transfer_id}' not found")))?;
        if st.dto.direction != FileTransferDirection::Upload {
            return Ok(true);
        }
        let meta = std::fs::metadata(&st.dto.local_path)?;
        Ok(meta.len() == st.source_size && file_mtime_ms(&meta) == st.source_mtime_ms)
    }

    pub fn read_chunk_at(&self, transfer_id: &str, chunk_index: usize) -> EngineResult<FileChunkFrame> {
        if !self.check_source_unchanged(transfer_id)? {
            let mut guard = self.tasks.write();
            if let Some(st) = guard.get_mut(transfer_id) {
                st.dto.status = FileTransferStatus::Failed;
                st.dto.error = Some("source file modified during transfer; resume invalidated".into());
                st.dto.updated_at_ms = chrono::Utc::now().timestamp_millis();
                let _ = self.persist_state(st);
            }
            return Err(RemoteEngineError::InvalidArgument(
                "source file changed since transfer started".into(),
            ));
        }

        let (local_path, total_bytes, chunk_size, total_chunks) = {
            let guard = self.tasks.read();
            let st = guard.get(transfer_id).ok_or_else(|| {
                RemoteEngineError::NotFound(format!("transfer '{transfer_id}' not found"))
            })?;
            (
                PathBuf::from(&st.dto.local_path),
                st.dto.total_bytes,
                st.dto.chunk_size as usize,
                st.chunk_bitmap.len(),
            )
        };
        if chunk_index >= total_chunks {
            return Err(RemoteEngineError::InvalidArgument(format!(
                "chunk index {chunk_index} out of bounds ({total_chunks})"
            )));
        }
        let offset = (chunk_index as u64) * (chunk_size as u64);
        let mut f = File::open(&local_path)?;
        f.seek(SeekFrom::Start(offset))?;
        let to_read = if total_bytes == 0 {
            0
        } else {
            (total_bytes.saturating_sub(offset) as usize).min(chunk_size)
        };
        let mut buf = vec![0u8; to_read];
        f.read_exact(&mut buf)?;
        let sha = sha256_hex_bytes(&buf);

        {
            let mut guard = self.tasks.write();
            if let Some(st) = guard.get_mut(transfer_id) {
                st.chunk_bitmap[chunk_index] = true;
                let completed_bytes: u64 = st
                    .chunk_bitmap
                    .iter()
                    .enumerate()
                    .filter(|(_, done)| **done)
                    .map(|(idx, _)| {
                        let start = (idx as u64) * (chunk_size as u64);
                        total_bytes.saturating_sub(start).min(chunk_size as u64)
                    })
                    .sum();
                st.dto.transferred_bytes = completed_bytes;
                st.dto.status = if st.chunk_bitmap.iter().all(|b| *b) {
                    FileTransferStatus::Completed
                } else {
                    FileTransferStatus::Running
                };
                st.dto.updated_at_ms = chrono::Utc::now().timestamp_millis();
                self.persist_state(st)?;
            }
        }

        Ok(FileChunkFrame {
            transfer_id: transfer_id.to_string(),
            offset,
            data: buf,
            sha256_hex: sha,
            is_last: chunk_index + 1 == total_chunks,
        })
    }

    pub fn read_next_chunk(&self, transfer_id: &str) -> EngineResult<Option<FileChunkFrame>> {
        let next_idx = {
            let guard = self.tasks.read();
            let st = guard.get(transfer_id).ok_or_else(|| {
                RemoteEngineError::NotFound(format!("transfer '{transfer_id}' not found"))
            })?;
            if matches!(
                st.dto.status,
                FileTransferStatus::Paused | FileTransferStatus::Cancelled
            ) {
                return Ok(None);
            }
            st.chunk_bitmap.iter().position(|done| !*done)
        };
        match next_idx {
            Some(idx) => self.read_chunk_at(transfer_id, idx).map(Some),
            None => Ok(None),
        }
    }

    pub fn write_chunk(&self, chunk: &FileChunkFrame) -> EngineResult<FileTransferTaskDto> {
        let actual_sha = sha256_hex_bytes(&chunk.data);
        if !actual_sha.eq_ignore_ascii_case(&chunk.sha256_hex) {
            return Err(RemoteEngineError::ChecksumMismatch {
                expected: chunk.sha256_hex.clone(),
                actual: actual_sha,
            });
        }

        let mut guard = self.tasks.write();
        let st = guard.get_mut(&chunk.transfer_id).ok_or_else(|| {
            RemoteEngineError::NotFound(format!("transfer '{}' not found", chunk.transfer_id))
        })?;
        if matches!(st.dto.status, FileTransferStatus::Cancelled) {
            return Err(RemoteEngineError::InvalidArgument(
                "transfer was cancelled".into(),
            ));
        }
        let dest_path = PathBuf::from(
            st.resolved_dest_path
                .as_deref()
                .unwrap_or(&st.dto.local_path),
        );
        if let Some(parent) = dest_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let chunk_size = st.dto.chunk_size as u64;
        if chunk_size == 0 || chunk.offset % chunk_size != 0 {
            return Err(RemoteEngineError::InvalidArgument(format!(
                "unaligned chunk offset {} for chunk_size {chunk_size}",
                chunk.offset
            )));
        }
        let chunk_idx = (chunk.offset / chunk_size) as usize;
        if chunk_idx >= st.chunk_bitmap.len() {
            return Err(RemoteEngineError::InvalidArgument(format!(
                "chunk index {chunk_idx} exceeds bitmap length {}",
                st.chunk_bitmap.len()
            )));
        }

        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&dest_path)?;
        file.set_len(st.dto.total_bytes)?;
        file.seek(SeekFrom::Start(chunk.offset))?;
        file.write_all(&chunk.data)?;
        file.flush()?;

        st.chunk_bitmap[chunk_idx] = true;
        let completed_bytes: u64 = st
            .chunk_bitmap
            .iter()
            .enumerate()
            .filter(|(_, done)| **done)
            .map(|(idx, _)| {
                let start = (idx as u64) * chunk_size;
                st.dto.total_bytes.saturating_sub(start).min(chunk_size)
            })
            .sum();
        st.dto.transferred_bytes = completed_bytes;
        st.dto.updated_at_ms = chrono::Utc::now().timestamp_millis();

        if st.chunk_bitmap.iter().all(|b| *b) {
            if let Some(expected_full) = st.dto.sha256.as_deref() {
                let actual_full = sha256_file_hex(&dest_path)?;
                if !actual_full.eq_ignore_ascii_case(expected_full) {
                    st.dto.status = FileTransferStatus::Failed;
                    st.dto.error = Some(format!(
                        "final file sha256 mismatch: expected {expected_full}, got {actual_full}"
                    ));
                    self.persist_state(st)?;
                    return Err(RemoteEngineError::ChecksumMismatch {
                        expected: expected_full.to_string(),
                        actual: actual_full,
                    });
                }
            }
            st.dto.status = FileTransferStatus::Completed;
        } else {
            st.dto.status = FileTransferStatus::Running;
        }
        self.persist_state(st)?;
        Ok(st.dto.clone())
    }

    pub fn missing_offsets(&self, transfer_id: &str) -> EngineResult<Vec<u64>> {
        let guard = self.tasks.read();
        let st = guard
            .get(transfer_id)
            .ok_or_else(|| RemoteEngineError::NotFound(format!("transfer '{transfer_id}' not found")))?;
        let chunk_size = st.dto.chunk_size as u64;
        Ok(st
            .chunk_bitmap
            .iter()
            .enumerate()
            .filter(|(_, done)| !**done)
            .map(|(idx, _)| (idx as u64) * chunk_size)
            .collect())
    }

    pub fn pause_task(&self, transfer_id: &str) -> EngineResult<FileTransferTaskDto> {
        let mut guard = self.tasks.write();
        let st = guard
            .get_mut(transfer_id)
            .ok_or_else(|| RemoteEngineError::NotFound(format!("transfer '{transfer_id}' not found")))?;
        st.dto.status = FileTransferStatus::Paused;
        st.dto.updated_at_ms = chrono::Utc::now().timestamp_millis();
        self.persist_state(st)?;
        Ok(st.dto.clone())
    }

    pub fn resume_task(&self, transfer_id: &str) -> EngineResult<FileTransferTaskDto> {
        let mut guard = self.tasks.write();
        let st = guard
            .get_mut(transfer_id)
            .ok_or_else(|| RemoteEngineError::NotFound(format!("transfer '{transfer_id}' not found")))?;
        if st.dto.status != FileTransferStatus::Completed {
            st.dto.status = FileTransferStatus::Running;
        }
        st.dto.updated_at_ms = chrono::Utc::now().timestamp_millis();
        self.persist_state(st)?;
        Ok(st.dto.clone())
    }

    pub fn cancel_task(&self, transfer_id: &str) -> EngineResult<FileTransferTaskDto> {
        let mut guard = self.tasks.write();
        let st = guard
            .get_mut(transfer_id)
            .ok_or_else(|| RemoteEngineError::NotFound(format!("transfer '{transfer_id}' not found")))?;
        st.dto.status = FileTransferStatus::Cancelled;
        st.dto.updated_at_ms = chrono::Utc::now().timestamp_millis();
        self.persist_state(st)?;
        Ok(st.dto.clone())
    }

    pub fn resolve_conflict_path(
        dest: &Path,
        policy: FileConflictPolicy,
    ) -> EngineResult<Option<PathBuf>> {
        if !dest.exists() {
            return Ok(Some(dest.to_path_buf()));
        }
        match policy {
            FileConflictPolicy::Overwrite | FileConflictPolicy::ApplyAll => {
                Ok(Some(dest.to_path_buf()))
            }
            FileConflictPolicy::Skip => Ok(None),
            FileConflictPolicy::Rename => {
                let stem = dest.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
                let ext = dest.extension().and_then(|e| e.to_str());
                let parent = dest.parent().unwrap_or_else(|| Path::new("."));
                for idx in 1..10_000 {
                    let candidate_name = match ext {
                        Some(e) => format!("{stem} ({idx}).{e}"),
                        None => format!("{stem} ({idx})"),
                    };
                    let candidate = parent.join(candidate_name);
                    if !candidate.exists() {
                        return Ok(Some(candidate));
                    }
                }
                Err(RemoteEngineError::Other("exhausted rename suffixes".into()))
            }
        }
    }

    pub fn list_tasks(&self) -> Vec<FileTransferTaskDto> {
        let mut items: Vec<_> = self.tasks.read().values().map(|s| s.dto.clone()).collect();
        items.sort_by_key(|t| t.created_at_ms);
        items
    }

    pub fn get_task(&self, transfer_id: &str) -> Option<FileTransferTaskDto> {
        self.tasks.read().get(transfer_id).map(|s| s.dto.clone())
    }
}
