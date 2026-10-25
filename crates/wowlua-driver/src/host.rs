//! What a build needs from the system it runs on: files and the clock
//! (spec M14, decision T394). A computer's are the default; a program
//! without them (the browser's) installs its own [`Host`], such as
//! [`MemoryHost`], and every check, build and format reads and writes
//! through it.
//!
//! The functions here mirror `std::fs`; the driver's build path calls them
//! instead of `std::fs` (clippy's `disallowed-methods` keeps it that way).

#![allow(clippy::disallowed_methods)]

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

/// A system's files and clock.
pub trait Host: Send + Sync {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    /// Whether `path` is a file, a directory, or neither.
    fn kind(&self, path: &Path) -> Option<Kind>;
    /// The paths in directory `path`, in any order.
    fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>>;
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    fn remove_dir_all(&self, path: &Path) -> io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// `path` absolute, without `.`, `..` or links.
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
    /// When the file last changed, if the system knows.
    fn modified(&self, _path: &Path) -> Option<SystemTime> {
        None
    }
    /// The time now, if the system has a clock.
    fn now(&self) -> Option<SystemTime> {
        None
    }
}

/// What a path is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
}

static INSTALLED: AtomicBool = AtomicBool::new(false);
static HOST: RwLock<Option<Arc<dyn Host>>> = RwLock::new(None);

/// Makes `host` the system every build reads and writes (for the whole
/// process); `None` goes back to the computer's.
pub fn install(host: Option<Arc<dyn Host>>) {
    let mut h = HOST.write().unwrap_or_else(|e| e.into_inner());
    INSTALLED.store(host.is_some(), Ordering::SeqCst);
    *h = host;
}

/// The installed host, unless it is the computer's.
pub fn installed() -> Option<Arc<dyn Host>> {
    if !INSTALLED.load(Ordering::Relaxed) {
        return None;
    }
    HOST.read().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    match installed() {
        Some(h) => h.read(path.as_ref()),
        None => std::fs::read(path),
    }
}

pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    let bytes = read(path)?;
    String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    match installed() {
        Some(h) => h.write(path.as_ref(), bytes.as_ref()),
        None => std::fs::write(path, bytes),
    }
}

pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    match installed() {
        Some(h) => h.create_dir_all(path.as_ref()),
        None => std::fs::create_dir_all(path),
    }
}

pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    match installed() {
        Some(h) => h.remove_file(path.as_ref()),
        None => std::fs::remove_file(path),
    }
}

pub fn remove_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    match installed() {
        Some(h) => h.remove_dir_all(path.as_ref()),
        None => std::fs::remove_dir_all(path),
    }
}

pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    match installed() {
        Some(h) => h.rename(from.as_ref(), to.as_ref()),
        None => std::fs::rename(from, to),
    }
}

/// Copies a file's bytes (a computer's copy keeps its permissions).
pub fn copy(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<u64> {
    match installed() {
        Some(h) => {
            let bytes = h.read(from.as_ref())?;
            h.write(to.as_ref(), &bytes)?;
            Ok(bytes.len() as u64)
        }
        None => std::fs::copy(from, to),
    }
}

pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    match installed() {
        Some(h) => h.canonicalize(path.as_ref()),
        None => std::fs::canonicalize(path),
    }
}

/// What `path` is, following links (`None`: nothing there).
pub fn kind(path: impl AsRef<Path>) -> Option<Kind> {
    match installed() {
        Some(h) => h.kind(path.as_ref()),
        None => {
            let m = std::fs::metadata(path).ok()?;
            if m.is_dir() {
                Some(Kind::Dir)
            } else {
                Some(Kind::File)
            }
        }
    }
}

pub fn is_file(path: impl AsRef<Path>) -> bool {
    kind(path) == Some(Kind::File)
}

pub fn is_dir(path: impl AsRef<Path>) -> bool {
    kind(path) == Some(Kind::Dir)
}

pub fn exists(path: impl AsRef<Path>) -> bool {
    kind(path).is_some()
}

/// The paths in directory `path`, sorted.
pub fn read_dir(path: impl AsRef<Path>) -> io::Result<Vec<PathBuf>> {
    let mut paths = match installed() {
        Some(h) => h.read_dir(path.as_ref())?,
        None => std::fs::read_dir(path)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect(),
    };
    paths.sort();
    Ok(paths)
}

pub fn modified(path: impl AsRef<Path>) -> Option<SystemTime> {
    match installed() {
        Some(h) => h.modified(path.as_ref()),
        None => std::fs::metadata(path).and_then(|m| m.modified()).ok(),
    }
}

/// Seconds since 1970; 0 on a system without a clock.
pub fn unix_now() -> i64 {
    let now = match installed() {
        Some(h) => h.now(),
        None if cfg!(all(target_family = "wasm", target_os = "unknown")) => None,
        None => Some(SystemTime::now()),
    };
    now.and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}

/// The files under `dir` (itself a file: just it), sorted by path;
/// directories for which `enter` is false are skipped, and hidden entries
/// (a name starting with `.`) unless `hidden`. Links are not followed.
pub fn walk(dir: &Path, hidden: bool, enter: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    walk_entries(dir, hidden, enter)
        .into_iter()
        .filter(|(_, k)| *k == Kind::File)
        .map(|(p, _)| p)
        .collect()
}

/// [`walk`]'s files and the directories it entered (not `dir` itself).
pub fn walk_entries(
    dir: &Path,
    hidden: bool,
    enter: &dyn Fn(&Path) -> bool,
) -> Vec<(PathBuf, Kind)> {
    let mut out = Vec::new();
    let visible = |p: &Path| {
        hidden
            || !p
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
    };
    if installed().is_none() {
        let walker = walkdir::WalkDir::new(dir)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(|e| {
                e.depth() == 0
                    || (visible(e.path()) && (!e.file_type().is_dir() || enter(e.path())))
            });
        for e in walker.flatten() {
            if e.file_type().is_file() {
                out.push((e.into_path(), Kind::File));
            } else if e.file_type().is_dir() && e.depth() > 0 {
                out.push((e.into_path(), Kind::Dir));
            }
        }
        return out;
    }
    fn into(
        dir: &Path,
        visible: &dyn Fn(&Path) -> bool,
        enter: &dyn Fn(&Path) -> bool,
        out: &mut Vec<(PathBuf, Kind)>,
    ) {
        for path in read_dir(dir).unwrap_or_default() {
            if !visible(&path) {
                continue;
            }
            match kind(&path) {
                Some(Kind::Dir) if enter(&path) => {
                    out.push((path.clone(), Kind::Dir));
                    into(&path, visible, enter, out);
                }
                Some(Kind::File) => out.push((path, Kind::File)),
                _ => {}
            }
        }
    }
    match kind(dir) {
        Some(Kind::File) => out.push((dir.to_path_buf(), Kind::File)),
        Some(Kind::Dir) => into(dir, &visible, enter, &mut out),
        None => {}
    }
    out
}

/// Files in memory: the browser's system, and tests'. Directories are the
/// ones made and those holding a file; paths are absolute (`/` the root).
#[derive(Default)]
pub struct MemoryHost {
    state: RwLock<Memory>,
    clock: RwLock<Option<SystemTime>>,
}

#[derive(Default)]
struct Memory {
    files: BTreeMap<PathBuf, Arc<[u8]>>,
    dirs: BTreeSet<PathBuf>,
}

impl MemoryHost {
    pub fn new() -> MemoryHost {
        MemoryHost::default()
    }

    /// With a clock that always shows `now`.
    pub fn with_clock(now: SystemTime) -> MemoryHost {
        let host = MemoryHost::default();
        host.set_clock(now);
        host
    }

    /// The clock shows `now` from here on (a page sets it from its own).
    pub fn set_clock(&self, now: SystemTime) {
        *self.clock.write().unwrap_or_else(|e| e.into_inner()) = Some(now);
    }

    /// Every file, by path.
    pub fn files(&self) -> Vec<(PathBuf, Arc<[u8]>)> {
        let m = self.state.read().unwrap_or_else(|e| e.into_inner());
        m.files
            .iter()
            .map(|(p, b)| (p.clone(), b.clone()))
            .collect()
    }

    fn not_found(path: &Path) -> io::Error {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("no such file or directory: {}", path.display()),
        )
    }
}

/// `path` absolute (from `/`) without `.` and `..`.
fn normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::from("/");
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(n) => out.push(n),
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    out
}

impl Memory {
    fn add_parents(&mut self, path: &Path) {
        let mut p = path.parent();
        while let Some(dir) = p {
            if !self.dirs.insert(dir.to_path_buf()) {
                break;
            }
            p = dir.parent();
        }
    }

    fn kind(&self, path: &Path) -> Option<Kind> {
        if self.files.contains_key(path) {
            Some(Kind::File)
        } else if self.dirs.contains(path) || path == Path::new("/") {
            Some(Kind::Dir)
        } else {
            None
        }
    }
}

impl Host for MemoryHost {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let m = self.state.read().unwrap_or_else(|e| e.into_inner());
        let path = normal(path);
        m.files
            .get(&path)
            .map(|b| b.to_vec())
            .ok_or_else(|| MemoryHost::not_found(&path))
    }

    fn kind(&self, path: &Path) -> Option<Kind> {
        let m = self.state.read().unwrap_or_else(|e| e.into_inner());
        m.kind(&normal(path))
    }

    fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        let m = self.state.read().unwrap_or_else(|e| e.into_inner());
        let path = normal(path);
        if m.kind(&path) != Some(Kind::Dir) {
            return Err(MemoryHost::not_found(&path));
        }
        let children = |set: &mut dyn Iterator<Item = &PathBuf>| -> Vec<PathBuf> {
            set.filter(|p| p.parent() == Some(path.as_path()))
                .cloned()
                .collect()
        };
        let mut out = children(&mut m.files.keys());
        out.extend(children(&mut m.dirs.iter()));
        Ok(out)
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut m = self.state.write().unwrap_or_else(|e| e.into_inner());
        let path = normal(path);
        if m.dirs.contains(&path) {
            return Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                format!("is a directory: {}", path.display()),
            ));
        }
        // (writing needs the directory, as on a disk)
        if let Some(parent) = path.parent()
            && m.kind(parent) != Some(Kind::Dir)
        {
            return Err(MemoryHost::not_found(parent));
        }
        m.files.insert(path, Arc::from(bytes));
        Ok(())
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        let mut m = self.state.write().unwrap_or_else(|e| e.into_inner());
        let path = normal(path);
        if m.files.contains_key(&path) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("a file is there: {}", path.display()),
            ));
        }
        m.add_parents(&path);
        m.dirs.insert(path);
        Ok(())
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        let mut m = self.state.write().unwrap_or_else(|e| e.into_inner());
        let path = normal(path);
        m.files
            .remove(&path)
            .map(drop)
            .ok_or_else(|| MemoryHost::not_found(&path))
    }

    fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
        let mut m = self.state.write().unwrap_or_else(|e| e.into_inner());
        let path = normal(path);
        if m.kind(&path) != Some(Kind::Dir) {
            return Err(MemoryHost::not_found(&path));
        }
        m.files.retain(|p, _| !p.starts_with(&path));
        m.dirs.retain(|p| !p.starts_with(&path));
        Ok(())
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let mut m = self.state.write().unwrap_or_else(|e| e.into_inner());
        let (from, to) = (normal(from), normal(to));
        match m.kind(&from) {
            Some(Kind::File) => {
                let bytes = m.files.remove(&from).unwrap_or_else(|| Arc::from(&[][..]));
                m.add_parents(&to);
                m.files.insert(to, bytes);
            }
            Some(Kind::Dir) => {
                // (what is at `to` is replaced, as on Unix for an empty one)
                m.files.retain(|p, _| !p.starts_with(&to));
                m.dirs.retain(|p| !p.starts_with(&to));
                let moved = |p: &Path| to.join(p.strip_prefix(&from).unwrap_or(p));
                let files: Vec<_> = m
                    .files
                    .iter()
                    .filter(|(p, _)| p.starts_with(&from))
                    .map(|(p, b)| (p.clone(), b.clone()))
                    .collect();
                let dirs: Vec<_> = m
                    .dirs
                    .iter()
                    .filter(|p| p.starts_with(&from))
                    .cloned()
                    .collect();
                for (p, b) in files {
                    m.files.remove(&p);
                    m.files.insert(moved(&p), b);
                }
                for p in dirs {
                    m.dirs.remove(&p);
                    m.dirs.insert(moved(&p));
                }
                m.add_parents(&to);
            }
            None => return Err(MemoryHost::not_found(&from)),
        }
        Ok(())
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        let path = normal(path);
        match self.kind(&path) {
            Some(_) => Ok(path),
            None => Err(MemoryHost::not_found(&path)),
        }
    }

    fn now(&self) -> Option<SystemTime> {
        *self.clock.read().unwrap_or_else(|e| e.into_inner())
    }
}

impl MemoryHost {
    /// Puts a file there, making its directories.
    pub fn insert(&self, path: impl AsRef<Path>, bytes: impl Into<Arc<[u8]>>) {
        let mut m = self.state.write().unwrap_or_else(|e| e.into_inner());
        let path = normal(path.as_ref());
        m.add_parents(&path);
        m.files.insert(path, bytes.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_files_behave_as_a_disk() {
        let host = MemoryHost::new();
        host.insert("/p/src/a.wowlua", &b"a"[..]);
        assert_eq!(host.kind(Path::new("/p/src")), Some(Kind::Dir));
        assert_eq!(host.kind(Path::new("/p/src/./a.wowlua")), Some(Kind::File));
        assert_eq!(host.read(Path::new("/p/x/../src/a.wowlua")).unwrap(), b"a");
        assert!(host.write(Path::new("/q/b"), b"b").is_err(), "no directory");
        host.create_dir_all(Path::new("/q")).unwrap();
        host.write(Path::new("/q/b"), b"b").unwrap();
        assert_eq!(
            host.read_dir(Path::new("/p")).unwrap(),
            vec![PathBuf::from("/p/src")]
        );
        host.rename(Path::new("/q/b"), Path::new("/q/c")).unwrap();
        assert!(host.read(Path::new("/q/b")).is_err());
        host.rename(Path::new("/q"), Path::new("/r/s")).unwrap();
        assert_eq!(host.read(Path::new("/r/s/c")).unwrap(), b"b");
        assert_eq!(host.kind(Path::new("/q")), None);
        host.remove_dir_all(Path::new("/p")).unwrap();
        assert_eq!(host.kind(Path::new("/p/src/a.wowlua")), None);
        assert_eq!(host.files().len(), 1);
    }
}
