//! Screenshots in tests (spec debugger §18.12.4, decision T315):
//! `expect(frame).toMatchScreenshot(name?)` compares the frame's image
//! with `__screenshots__/<test file>/<key>.png` next to the test file; a
//! missing one is written, `-u` rewrites one that differs; a mismatch
//! writes `<key>.actual.png` and `<key>.diff.png` beside it.
//! `mock.screenshot(name?, frame?)` writes an image and returns its path.
//! The drawing is the host's ([`Camera`]): the test runner has no renderer.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use wowlua_engine::widgets::{WidgetId, widget_of};
use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Value, Vm};

/// Draws and compares screenshots for the test runner.
pub trait Camera: Send + Sync {
    /// A PNG of the screen, or of a frame and what it holds, at `scale`
    /// times the screen's size.
    fn shoot(&self, vm: &mut Vm, frame: Option<WidgetId>, scale: f64) -> Result<Vec<u8>, String>;
    /// The share of pixels that differ (`threshold` as pixelmatch's), and
    /// a diff image (PNG).
    fn compare(
        &self,
        expected: &[u8],
        actual: &[u8],
        threshold: f64,
    ) -> Result<(f64, Vec<u8>), String>;
    /// Where `mock.screenshot` writes.
    fn out_dir(&self) -> PathBuf;
    /// The client's own tables, where the installation is at hand
    /// (decision T321).
    fn client_data(&self) -> Option<Arc<dyn wowlua_engine::clientdata::ClientData>> {
        None
    }
}

#[derive(Default)]
struct State {
    camera: Option<Arc<dyn Camera>>,
    dir: PathBuf,
    file_name: String,
    update: bool,
    written: usize,
    updated: usize,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

/// A test file begins.
pub(crate) fn begin(test_file: &Path, update: bool, camera: Option<Arc<dyn Camera>>) {
    let name = test_file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dir = test_file
        .parent()
        .unwrap_or(Path::new(""))
        .join("__screenshots__")
        .join(&name);
    STATE.with(|s| {
        *s.borrow_mut() = State {
            camera,
            dir,
            file_name: name,
            update,
            written: 0,
            updated: 0,
        }
    });
}

/// Screenshots written and rewritten in the file's run.
pub(crate) fn finish() -> (usize, usize) {
    STATE.with(|s| {
        let s = s.borrow();
        (s.written, s.updated)
    })
}

/// A key as a file name.
fn file_key(key: &str) -> String {
    key.chars()
        .map(|c| match c {
            c if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' => c,
            ' ' => '-',
            _ => '_',
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn frame_arg(vm: &Vm, v: Value) -> Result<Option<WidgetId>, String> {
    if v.is_nil() {
        return Ok(None);
    }
    widget_of(vm, v)
        .map(Some)
        .ok_or_else(|| "expected a frame".to_string())
}

fn camera() -> Result<Arc<dyn Camera>, String> {
    STATE.with(|s| s.borrow().camera.clone()).ok_or_else(|| {
        "screenshots need `wowlua test` with a renderer (a GPU adapter, or a software one)"
            .to_string()
    })
}

/// `screenshotMatch(key, frame?, threshold, maxDiffRatio)`: `true`, or
/// `false` and why.
pub(crate) fn screenshot_match(vm: &mut Vm, c: NativeCall) -> Ret {
    let key = vm.check_str(&c, 1)?;
    let key = String::from_utf8_lossy(vm.str_bytes(key)).into_owned();
    let threshold = vm.opt_number(&c, 3, 0.1)?;
    let max_ratio = vm.opt_number(&c, 4, 0.001)?;
    let result = (|| -> Result<Option<String>, String> {
        let frame = frame_arg(vm, c.arg(2))?;
        let cam = camera()?;
        let actual = cam.shoot(vm, frame, 1.0)?;
        let (dir, update) = STATE.with(|s| (s.borrow().dir.clone(), s.borrow().update));
        let base = file_key(&key);
        let stored = dir.join(format!("{base}.png"));
        let actual_path = dir.join(format!("{base}.actual.png"));
        let diff_path = dir.join(format!("{base}.diff.png"));
        let write = |path: &Path, bytes: &[u8]| -> Result<(), String> {
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::write(path, bytes).map_err(|e| format!("writing {}: {e}", path.display()))
        };
        let Ok(expected) = std::fs::read(&stored) else {
            write(&stored, &actual)?;
            STATE.with(|s| s.borrow_mut().written += 1);
            return Ok(None);
        };
        let (ratio, diff) = cam.compare(&expected, &actual, threshold)?;
        if ratio <= max_ratio {
            let _ = std::fs::remove_file(&actual_path);
            let _ = std::fs::remove_file(&diff_path);
            return Ok(None);
        }
        if update {
            write(&stored, &actual)?;
            let _ = std::fs::remove_file(&actual_path);
            let _ = std::fs::remove_file(&diff_path);
            STATE.with(|s| s.borrow_mut().updated += 1);
            return Ok(None);
        }
        write(&actual_path, &actual)?;
        write(&diff_path, &diff)?;
        Ok(Some(format!(
            "the screenshot \"{key}\" differs: {:.2}% of its pixels (at most {:.2}%; wowlua test -u updates it)\n  stored: {}\n  now:    {}\n  diff:   {}",
            ratio * 100.0,
            max_ratio * 100.0,
            stored.display(),
            actual_path.display(),
            diff_path.display()
        )))
    })();
    match result {
        Ok(None) => Ok(vec![Value::Bool(true)]),
        Ok(Some(why)) | Err(why) => {
            let why = vm.str(&why);
            Ok(vec![Value::Bool(false), why])
        }
    }
}

/// `screenshot(name, frame?)`: writes the image, returns its path.
pub(crate) fn screenshot(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = vm.check_str(&c, 1)?;
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    let frame = frame_arg(vm, c.arg(2)).map_err(|e| vm.error(&format!("mock.screenshot: {e}")))?;
    let cam = camera().map_err(|e| vm.error(&e))?;
    let png = cam.shoot(vm, frame, 1.0).map_err(|e| vm.error(&e))?;
    let file = STATE.with(|s| s.borrow().file_name.clone());
    let dir = cam.out_dir().join(file);
    let path = dir.join(format!("{}.png", file_key(&name)));
    std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(&path, &png))
        .map_err(|e| vm.error(&format!("mock.screenshot: {e}")))?;
    Ok(vec![vm.str(&path.to_string_lossy())])
}
