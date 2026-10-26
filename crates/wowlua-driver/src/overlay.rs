//! Files' contents an editor has open but not saved (`wowlua lsp`, spec
//! §16.2): a build reads a source from here before the disk, so its
//! diagnostics are those of what the editor shows. Empty unless a language
//! server fills it; keyed by canonical path.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

static OVERLAY: RwLock<Option<HashMap<PathBuf, Arc<[u8]>>>> = RwLock::new(None);

fn key(path: &Path) -> PathBuf {
    crate::host::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The editor's text of `path`.
pub fn set(path: &Path, bytes: impl Into<Arc<[u8]>>) {
    let mut o = OVERLAY.write().unwrap_or_else(|e| e.into_inner());
    o.get_or_insert_with(HashMap::new)
        .insert(key(path), bytes.into());
}

/// The editor's text of `path` when it has the file open (in the overlay);
/// whether it had. One step with [`remove`]: a file closed meanwhile stays
/// closed.
pub fn set_open(path: &Path, bytes: impl Into<Arc<[u8]>>) -> bool {
    let mut o = OVERLAY.write().unwrap_or_else(|e| e.into_inner());
    match o.as_mut().and_then(|map| map.get_mut(&key(path))) {
        Some(text) => {
            *text = bytes.into();
            true
        }
        None => false,
    }
}

/// `path` as it is on disk again.
pub fn remove(path: &Path) {
    let mut o = OVERLAY.write().unwrap_or_else(|e| e.into_inner());
    if let Some(map) = o.as_mut() {
        map.remove(&key(path));
    }
}

/// The editor's texts of exactly these files (any others read from disk
/// again).
pub fn replace_all(files: impl IntoIterator<Item = (PathBuf, Arc<[u8]>)>) {
    let map: HashMap<PathBuf, Arc<[u8]>> = files.into_iter().map(|(p, b)| (key(&p), b)).collect();
    let mut o = OVERLAY.write().unwrap_or_else(|e| e.into_inner());
    *o = Some(map);
}

/// A source's bytes: the editor's when it has the file open, else the disk's.
pub fn read(path: &Path) -> std::io::Result<Vec<u8>> {
    {
        let o = OVERLAY.read().unwrap_or_else(|e| e.into_inner());
        // (only a path whose file name is an open file's is made canonical
        // to look it up: a build reads hundreds of files)
        if let Some(map) = o.as_ref().filter(|m| !m.is_empty())
            && map.keys().any(|k| k.file_name() == path.file_name())
            && let Some(bytes) = map.get(&key(path))
        {
            return Ok(bytes.to_vec());
        }
    }
    crate::host::read(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_editor_s_text_over_the_disk_s() {
        let dir = std::env::temp_dir().join(format!("wowlua-overlay-{}", std::process::id()));
        crate::host::create_dir_all(&dir).unwrap();
        let file = dir.join("a.wowlua");
        crate::host::write(&file, b"disk").unwrap();
        assert_eq!(read(&file).unwrap(), b"disk");
        set(&file, b"editor".to_vec());
        assert_eq!(read(&file).unwrap(), b"editor");
        remove(&file);
        assert_eq!(read(&file).unwrap(), b"disk");
        replace_all([(file.clone(), Arc::from(&b"all"[..]))]);
        assert_eq!(read(&file).unwrap(), b"all");
        replace_all([]);
        assert_eq!(read(&file).unwrap(), b"disk");
        // a closed file stays closed
        assert!(!set_open(&file, b"late".to_vec()));
        assert_eq!(read(&file).unwrap(), b"disk");
        set(&file, b"open".to_vec());
        assert!(set_open(&file, b"asked".to_vec()));
        assert_eq!(read(&file).unwrap(), b"asked");
        remove(&file);
        crate::host::remove_dir_all(&dir).ok();
    }
}
