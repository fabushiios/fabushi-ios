//! Target Symbol: StorageFileLockPosix
//! Telegram Desktop 1:1 Submodule: `StorageFileLockPosix`
//! Subsystem: `storage`, Feature ID: `TCG-N34`
//! Disposition: `rust_domain_or_server_1to1`

use std::fs::File;
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct StorageFileLockPosix {
    path: Option<PathBuf>,
    file: Option<File>,
    locked: bool,
}

impl StorageFileLockPosix {
    pub fn new() -> Self {
        Self {
            path: None,
            file: None,
            locked: false,
        }
    }

    #[cfg(unix)]
    pub fn lock(&mut self, path: &Path) -> Result<bool, String> {
        self.unlock();

        let file = match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
        {
            Ok(f) => f,
            Err(e) => return Err(format!("Could not open lockfile {}: {}", path.display(), e)),
        };

        let fd = file.as_raw_fd();
        let mut flock_data = libc::flock {
            l_start: 0,
            l_len: 4, // kLockLimit = 4 bytes
            l_pid: 0,
            l_type: libc::F_WRLCK as i16,
            l_whence: libc::SEEK_SET as i16,
        };

        let res = unsafe { libc::fcntl(fd, libc::F_SETLK, &flock_data) };
        if res == 0 {
            self.file = Some(file);
            self.path = Some(path.to_path_buf());
            self.locked = true;
            Ok(true)
        } else {
            // Check locking PID
            let _ = unsafe { libc::fcntl(fd, libc::F_GETLK, &mut flock_data) };
            Ok(false)
        }
    }

    #[cfg(not(unix))]
    pub fn lock(&mut self, path: &Path) -> Result<bool, String> {
        self.unlock();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| format!("Could not open lockfile {}: {}", path.display(), e))?;
        self.file = Some(file);
        self.path = Some(path.to_path_buf());
        self.locked = true;
        Ok(true)
    }

    #[cfg(unix)]
    pub fn unlock(&mut self) {
        if let Some(file) = self.file.take() {
            let fd = file.as_raw_fd();
            let flock_data = libc::flock {
                l_start: 0,
                l_len: 4,
                l_pid: 0,
                l_type: libc::F_UNLCK as i16,
                l_whence: libc::SEEK_SET as i16,
            };
            unsafe {
                libc::fcntl(fd, libc::F_SETLK, &flock_data);
            }
        }
        self.locked = false;
        self.path = None;
    }

    #[cfg(not(unix))]
    pub fn unlock(&mut self) {
        self.file = None;
        self.locked = false;
        self.path = None;
    }

    pub fn is_locked(&self) -> bool {
        self.locked
    }
}

impl Drop for StorageFileLockPosix {
    fn drop(&mut self) {
        self.unlock();
    }
}
