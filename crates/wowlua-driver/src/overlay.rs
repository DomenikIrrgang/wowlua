//! Files' contents an editor has open but not saved (`wowlua lsp`, spec
//! §16.2): a build reads a source from here before the disk, so its
//! diagnostics are those of what the editor shows. Empty unless a language
//! server fills it; keyed by canonical path.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

static OVERLAY: RwLock<Option<HashMap<PathBuf, Arc<[u8]>>>> = RwLock::new(None);

fn key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The editor's text of `path`.
pub fn set(path: &Path, bytes: impl Into<Arc<[u8]>>) {
    let mut o = OVERLAY.write().unwrap_or_else(|e| e.into_inner());
    o.get_or_insert_with(HashMap::new)
        .insert(key(path), bytes.into());
}

/// `path` as it is on disk again.
pub fn remove(path: &Path) {
    let mut o = OVERLAY.write().unwrap_or_else(|e| e.into_inner());
    if let Some(map) = o.as_mut() {
        map.remove(&key(path));
    }
}

/// A source's bytes: the editor's when it has the file open, else the disk's.
pub fn read(path: &Path) -> std::io::Result<Vec<u8>> {
    {
        let o = OVERLAY.read().unwrap_or_else(|e| e.into_inner());
        if let Some(map) = o.as_ref().filter(|m| !m.is_empty())
            && let Some(bytes) = map.get(&key(path))
        {
            return Ok(bytes.to_vec());
        }
    }
    std::fs::read(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_editor_s_text_over_the_disk_s() {
        let dir = std::env::temp_dir().join(format!("wowlua-overlay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.wowlua");
        std::fs::write(&file, b"disk").unwrap();
        assert_eq!(read(&file).unwrap(), b"disk");
        set(&file, b"editor".to_vec());
        assert_eq!(read(&file).unwrap(), b"editor");
        remove(&file);
        assert_eq!(read(&file).unwrap(), b"disk");
        std::fs::remove_dir_all(&dir).ok();
    }
}
