//! A directory of a test's own under the system's temporary one.
use std::path::{Path, PathBuf};

/// A directory of its own under the system's temporary one, removed with what is in it once
/// dropped: a ledger in it goes with the `-wal` and `-shm` files `SQLite` keeps beside it, and
/// a test that fails leaves nothing behind for the next run either.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.subsec_nanos());
        let path =
            std::env::temp_dir().join(format!("home-link-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).expect("the temporary directory takes a directory");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
