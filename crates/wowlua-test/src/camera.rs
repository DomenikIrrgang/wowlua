//! The test runner's camera (spec debugger §18.11): the scene drawn
//! off-screen by one renderer per process (made at the first screenshot),
//! with the addons' files and the installation's textures and fonts
//! (opened at the first one too). `wowlua test`'s and wowlua studio's.

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
    /// The flavor's client folders (`_forever_`).
    folders: Vec<String>,
    assets: OnceLock<ClientAssets>,
    out: PathBuf,
}

impl Sources {
    pub fn new(project: &Project, build: &BuildOutput, flavor: &str) -> Sources {
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
        let _ = wowlua_host::write(path, format!("{scene:#?}"));
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

struct RenderCamera {
    sources: Arc<Sources>,
}

impl crate::screenshots::Camera for RenderCamera {
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
) -> Option<Arc<dyn crate::screenshots::Camera>> {
    Some(Arc::new(RenderCamera {
        sources: Arc::new(Sources::new(project, build, flavor)),
    }))
}
