//! Small advisory lock helpers (cross-process) for V2 persistence paths.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

use fs2::FileExt;

/// Exclusive lock file guard: unlocks on drop.
pub struct ExclusiveFileLock {
    file: File,
}

impl ExclusiveFileLock {
    /// Opens `path` (creating it), then blocks until an exclusive lock is acquired.
    pub fn acquire_exclusive(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        file.lock_exclusive()?;
        Ok(Self { file })
    }

    /// Non-blocking attempt: `Ok(None)` if another process holds an exclusive lock.
    pub fn try_acquire_exclusive(path: &Path) -> io::Result<Option<Self>> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { file })),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl Drop for ExclusiveFileLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn try_acquire_exclusive_returns_none_when_other_thread_holds_lock() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "trilogicon_flock_try_{}_{}",
            std::process::id(),
            nanos
        ));
        let _ = std::fs::remove_file(&path);
        let path_c = path.clone();
        let holder = thread::spawn(move || {
            let _lock = ExclusiveFileLock::acquire_exclusive(&path_c).unwrap();
            thread::sleep(Duration::from_millis(400));
        });
        thread::sleep(Duration::from_millis(50));
        let second = ExclusiveFileLock::try_acquire_exclusive(&path).unwrap();
        assert!(
            second.is_none(),
            "second exclusive lock should not succeed while holder runs"
        );
        holder.join().unwrap();
        let after = ExclusiveFileLock::try_acquire_exclusive(&path).unwrap();
        assert!(after.is_some(), "lock should be free after holder exits");
        let _ = std::fs::remove_file(&path);
    }
}
