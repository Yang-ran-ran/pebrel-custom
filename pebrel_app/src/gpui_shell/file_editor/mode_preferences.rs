//! Per-document reading/source mode preferences.
//!
//! The preference is keyed by a digest of the document identity so local paths
//! and SSH destinations are not written in clear text to the settings file.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const FILE_NAME: &str = "document-view-modes.json";
static STORE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Store {
    version: u8,
    modes: BTreeMap<String, bool>,
}

pub(super) fn key(identity: &str) -> String {
    let digest = Sha256::digest(identity.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn load(identity: &str) -> bool {
    let Ok(_guard) = STORE_LOCK.lock() else { return false };
    load_from(&store_path(), identity)
}

pub(super) fn save(identity: &str, reading: bool) -> std::io::Result<()> {
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| std::io::Error::other("document mode store lock poisoned"))?;
    save_to(&store_path(), identity, reading)
}

fn store_path() -> std::path::PathBuf {
    crate::platform::dirs::data_dir().join(FILE_NAME)
}

fn load_from(path: &Path, identity: &str) -> bool {
    read(path).modes.get(&key(identity)).copied().unwrap_or(false)
}

fn save_to(path: &Path, identity: &str, reading: bool) -> std::io::Result<()> {
    let _file_lock = crate::atomic_file::try_lock(path)?.ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::WouldBlock, "document mode store is busy")
    })?;
    let mut store = read(path);
    store.version = 1;
    store.modes.insert(key(identity), reading);
    let bytes = serde_json::to_vec_pretty(&store).map_err(std::io::Error::other)?;
    crate::atomic_file::write(path, &bytes)
}

fn read(path: &Path) -> Store {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Store::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_modes_round_trip_without_cross_document_leakage() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(FILE_NAME);
        let first = "local:/docs/first.md";
        let second = "local:/docs/second.md";

        assert!(!load_from(&path, first));
        save_to(&path, first, true).unwrap();

        assert!(load_from(&path, first));
        assert!(!load_from(&path, second));
        save_to(&path, first, false).unwrap();
        assert!(!load_from(&path, first));
    }

    #[test]
    fn identity_keys_are_stable_and_do_not_expose_document_names() {
        let first = key("ssh://work:/home/user/notes.md");
        assert_eq!(first, key("ssh://work:/home/user/notes.md"));
        assert_eq!(first.len(), 64);
        assert!(!first.contains("notes"));
    }
}
