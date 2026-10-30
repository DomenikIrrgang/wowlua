//! Screenshots of the emulated client's UI (spec debugger §18.11): a debug
//! session's and `wowlua screenshot`'s, drawn by the test runner's camera
//! (`wowlua_test::camera`).

use std::sync::{Arc, Mutex};

use wowlua_vm::Vm;

pub use wowlua_test::camera::camera;

/// The screenshot function debug sessions use.
pub fn screenshotter(
    suite: &wowlua_test::Suite,
) -> Option<wowlua_debug::controller::Screenshotter> {
    let camera = suite.camera.clone()?;
    Some(Box::new(
        move |vm: &mut Vm, frame: Option<&str>, scale: Option<f64>| {
            let id = match frame {
                Some(name) => Some(
                    wowlua_debug::ui::widget_named(vm, name)
                        .ok_or_else(|| format!("no frame {name}"))?,
                ),
                None => None,
            };
            let png = camera.shoot(vm, id, scale.unwrap_or(1.0))?;
            let img = wowlua_render::decode_png(&png)?;
            Ok((png, img.width, img.height))
        },
    ))
}

/// `wowlua screenshot`: runs a test file (or one test of it) and writes
/// the screen, or a frame, as each test left it.
pub fn command(
    out: &crate::output::Output,
    global: &crate::GlobalArgs,
    args: &crate::ScreenshotArgs,
) -> bool {
    let file = args.file.to_string_lossy().replace('\\', "/");
    let written: Arc<Mutex<Vec<String>>> = Arc::default();
    let errors: Arc<Mutex<Vec<String>>> = Arc::default();
    let r = crate::commands::with_suite(out, global, args.flavor.as_deref(), None, &mut |suite| {
        let Some(camera) = suite.camera.clone() else {
            return;
        };
        // (`--world`: the 3D world behind the UI, T522)
        suite.world = args.world;
        let root = suite.root.clone();
        let rel = std::path::Path::new(&file);
        let rel = std::fs::canonicalize(rel)
            .ok()
            .and_then(|abs| {
                abs.strip_prefix(std::fs::canonicalize(&root).ok()?)
                    .ok()
                    .map(|p| p.to_path_buf())
            })
            .unwrap_or_else(|| rel.to_path_buf());
        let rel = rel.to_string_lossy().replace('\\', "/");
        let Some(test) = suite.build.tests.iter().find(|t| t.rel == rel) else {
            if let Ok(mut e) = errors.lock() {
                e.push(format!("{rel} is not a test file of the package"));
            }
            return;
        };
        let target = args.output.clone();
        let frame = args.frame.clone();
        let scale = args.scale.unwrap_or(1.0);
        let only = args.test.clone();
        let filters: Vec<String> = only.iter().cloned().collect();
        let (written_in, errors_in) = (written.clone(), errors.clone());
        let after = move |vm: &mut Vm, name: &str| {
            let (written, errors) = (&written_in, &errors_in);
            if only.as_ref().is_some_and(|t| !name.contains(t.as_str())) {
                return;
            }
            let id = match &frame {
                Some(f) => match wowlua_debug::ui::widget_named(vm, f) {
                    Some(id) => Some(id),
                    None => {
                        if let Ok(mut e) = errors.lock() {
                            e.push(format!("{name}: no frame {f}"));
                        }
                        return;
                    }
                },
                None => None,
            };
            match camera.shoot(vm, id, scale) {
                Ok(png) => {
                    let count = written.lock().map(|w| w.len()).unwrap_or(0);
                    // The first test's image at the path given, the others' beside it.
                    let path = if count == 0 {
                        target.clone()
                    } else {
                        let stem = target
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let safe: String = name
                            .chars()
                            .map(|c| if c.is_alphanumeric() { c } else { '-' })
                            .collect();
                        target.with_file_name(format!("{stem}-{safe}.png"))
                    };
                    match std::fs::write(&path, png) {
                        Ok(()) => {
                            if let Ok(mut w) = written.lock() {
                                w.push(format!("{} ({name})", path.display()));
                            }
                        }
                        Err(e) => {
                            if let Ok(mut errs) = errors.lock() {
                                errs.push(format!("{}: {e}", path.display()));
                            }
                        }
                    }
                }
                Err(e) => {
                    if let Ok(mut errs) = errors.lock() {
                        errs.push(format!("{name}: {e}"));
                    }
                }
            }
        };
        suite.after_test = Some(Arc::new(after));
        let result = wowlua_test::run_file(suite, test, &filters);
        suite.after_test = None;
        if let Some(e) = result.error
            && let Ok(mut errs) = errors.lock()
        {
            errs.push(e);
        }
    });
    if let Err(e) = r {
        eprintln!("error: {e}");
        return false;
    }
    for w in written.lock().map(|w| w.clone()).unwrap_or_default() {
        out.status("Wrote", w);
    }
    let errs = errors.lock().map(|e| e.clone()).unwrap_or_default();
    for e in &errs {
        eprintln!("error: {e}");
    }
    errs.is_empty() && written.lock().is_ok_and(|w| !w.is_empty())
}
