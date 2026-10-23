//! Screenshots of the emulated client's UI (spec debugger §18.11): the
//! scene drawn off-screen by one renderer per process (made at the first
//! screenshot), with the addons' files and the installation's textures and
//! fonts (opened at the first one too).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use wowlua_driver::{BuildOutput, Project};
use wowlua_engine::widgets::WidgetId;
use wowlua_render::Renderer;
use wowlua_render::client::ClientAssets;
use wowlua_vm::Vm;

fn renderer() -> &'static Mutex<Option<Result<Renderer, String>>> {
    static R: OnceLock<Mutex<Option<Result<Renderer, String>>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(None))
}

/// Where a package's screenshots find their files.
pub struct Sources {
    addons: Vec<(String, PathBuf)>,
    /// The flavor's client folders (`_classic_beta_`).
    folders: Vec<String>,
    assets: OnceLock<ClientAssets>,
    out: PathBuf,
}

impl Sources {
    fn new(project: &Project, build: &BuildOutput, flavor: &str) -> Sources {
        let mut addons = vec![(build.addon_name.clone(), build.dir.clone())];
        for a in &project.manifest.manifest.test.addons {
            let dir = project.root().join(a);
            if let Some(name) = dir.file_name() {
                addons.push((name.to_string_lossy().into_owned(), dir.clone()));
            }
        }
        let folders = wowlua_flavors::table()
            .clients_of(flavor)
            .flat_map(|c| c.folders.iter().cloned())
            .collect();
        Sources {
            addons,
            folders,
            assets: OnceLock::new(),
            out: project.manifest.out_dir().join("screenshots"),
        }
    }

    /// The addons' folders, and the installation's storage when there is
    /// one with the flavor's client.
    fn assets(&self) -> ClientAssets {
        self.assets
            .get_or_init(|| {
                let casc = wowlua_driver::install::find_wow(None)
                    .ok()
                    .and_then(|install| {
                        wowlua_render::client::open_client(&install, &self.folders).ok()
                    });
                ClientAssets::new(casc, &self.addons)
            })
            .clone()
    }
}

/// A PNG of the screen (or a frame's part) at `scale`: its bytes, width
/// and height.
pub fn shoot(
    vm: &mut Vm,
    sources: &Sources,
    frame: Option<WidgetId>,
    scale: f64,
) -> Result<(Vec<u8>, u32, u32), String> {
    let scene = wowlua_engine::scene::build(vm);
    // The scene as data, for looking into a drawing.
    if let Some(path) = std::env::var_os("WOWLUA_SCENE_DUMP") {
        let _ = std::fs::write(path, format!("{scene:#?}"));
    }
    let crop = match frame {
        Some(id) => Some(
            wowlua_engine::scene::extent(vm, id)
                .ok_or("the frame has no rectangle (it is not laid out)")?,
        ),
        None => None,
    };
    let mut assets = sources.assets();
    let mut guard = renderer()
        .lock()
        .map_err(|_| "the renderer failed earlier".to_string())?;
    let r = guard.get_or_insert_with(Renderer::new_blocking);
    let r = r.as_mut().map_err(|e| e.clone())?;
    let img = r.render_blocking(&scene, scale as f32, crop, &mut assets)?;
    let png = wowlua_render::encode_png(&img)?;
    Ok((png, img.width, img.height))
}

struct CliCamera {
    sources: Arc<Sources>,
}

impl wowlua_test::screenshots::Camera for CliCamera {
    fn shoot(&self, vm: &mut Vm, frame: Option<WidgetId>, scale: f64) -> Result<Vec<u8>, String> {
        shoot(vm, &self.sources, frame, scale).map(|(png, _, _)| png)
    }

    fn compare(
        &self,
        expected: &[u8],
        actual: &[u8],
        threshold: f64,
    ) -> Result<(f64, Vec<u8>), String> {
        let a = wowlua_render::decode_png(expected)?;
        let b = wowlua_render::decode_png(actual)?;
        let d = wowlua_render::compare::compare(&a, &b, threshold);
        Ok((d.ratio(), wowlua_render::encode_png(&d.image)?))
    }

    fn out_dir(&self) -> PathBuf {
        self.sources.out.clone()
    }

    fn client_data(&self) -> Option<Arc<dyn wowlua_engine::clientdata::ClientData>> {
        let sources = self.sources.clone();
        Some(Arc::new(wowlua_render::client::CascClientData::new(
            Box::new(move || sources.assets().casc),
        )))
    }
}

/// The test runner's camera for a package's test build.
pub fn camera(
    project: &Project,
    build: &BuildOutput,
    flavor: &str,
) -> Option<Arc<dyn wowlua_test::screenshots::Camera>> {
    Some(Arc::new(CliCamera {
        sources: Arc::new(Sources::new(project, build, flavor)),
    }))
}

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
