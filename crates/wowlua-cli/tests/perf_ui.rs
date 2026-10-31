//! How long a frame of an addon's session takes with Blizzard's UI
//! (decision T541): the package at `PERF_PACKAGE` loaded as a debug session
//! loads it (`test.blizzard-ui`, its `test.profile` world), the 3D world on
//! from the installation, the camera zoomed out (`PERF_ZOOM` steps, 50 by
//! default) over the profile's units; the engine's frame and the scene
//! timed apart. Run by hand: `PERF_PACKAGE=<dir> cargo test --release -p
//! wowlua-cli --test perf_ui -- --ignored --nocapture`.

use std::time::{Duration, Instant};

use wowlua_driver::project::{LoadOptions, load_projects};
use wowlua_test::runs::{self, Switches};
use wowlua_vm::{Value, Vm};

fn run(vm: &mut Vm, src: &str) {
    let f = vm.load(src.as_bytes(), "@Perf.lua").expect("compiles");
    if let Err(e) = vm.call(f, &[]) {
        let text = match vm.hidden(e.value) {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            other => format!("{other:?}"),
        };
        panic!("{text}");
    }
}

#[test]
#[ignore]
fn a_frame_of_an_addon_with_blizzards_ui() {
    let Some(root) = std::env::var_os("PERF_PACKAGE") else {
        eprintln!("no PERF_PACKAGE: skipped");
        return;
    };
    let flavor = std::env::var("PERF_FLAVOR").unwrap_or_else(|_| "camelot".into());
    let mut driver = wowlua_driver::Session::new();
    let options = LoadOptions {
        cwd: root.into(),
        lock_in_memory: true,
        offline: true,
        ..LoadOptions::default()
    };
    let projects = load_projects(&mut driver, &options).expect("the package loads");
    let project = &projects[0];
    let (build, _) = runs::build(
        &mut driver,
        project,
        vec![flavor.clone()],
        &flavor,
        "test",
        wowlua_driver::parallel::Jobs::default(),
        &mut |_| {},
    )
    .expect("the build");
    let ui = wowlua_test::ui_source(&flavor, &|_| {}).ok();
    let mut suite = runs::suite(
        project,
        &build,
        &flavor,
        ui,
        Switches::default(),
        None,
        None,
    );
    // (PERF_PROFILE: another world, such as the built-in `northshire`)
    if let Ok(profile) = std::env::var("PERF_PROFILE") {
        suite.profile = Some(profile);
    }
    let mut session = wowlua_test::AddonSession::open(&suite).expect("the session");
    let vm = &mut session.vm;
    // (the world from the installation, as the game page's "3D world")
    let install = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let storage = wowlua_casc::FsStorage { root: install };
    let product = wowlua_casc::Casc::product_of_folder(&storage, "_forever_").expect("camelot");
    let casc =
        std::sync::Arc::new(wowlua_casc::Casc::open(Box::new(storage), &product).expect("opens"));
    let data = std::sync::Arc::new(wowlua_render::client::CascClientData::new(Box::new(
        move || Some(casc.clone()),
    )));
    wowlua_engine::clientdata::set(vm, data);
    wowlua_engine::set_world(vm, true);
    // (the units' nameplates and names on, the camera out over them)
    let zoom = std::env::var("PERF_ZOOM").unwrap_or_else(|_| "50".into());
    run(
        vm,
        &format!(
            "for _, n in ipairs({{ 'nameplateShowEnemies', 'nameplateShowFriendlyNpcs', 'UnitNameInteractiveNPC', 'UnitNameHostleNPC' }}) do
    C_CVar.SetCVar(n, '1')
end
CameraZoomOut({zoom})"
        ),
    );
    let frames: u32 = std::env::var("PERF_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);
    let (mut engine, mut scene) = (Duration::ZERO, Duration::ZERO);
    for i in 0..frames + 10 {
        let t = Instant::now();
        wowlua_engine::clock::advance_frames(vm, 1, 1000.0 / 60.0);
        let e = t.elapsed();
        let t = Instant::now();
        let built = wowlua_engine::scene::build(vm);
        let s = t.elapsed();
        std::hint::black_box(built);
        // (the first frames read the world: not counted)
        if i >= 10 {
            engine += e;
            scene += s;
        }
    }
    let ms = |d: Duration| d.as_secs_f64() * 1000.0 / f64::from(frames);
    eprintln!(
        "a frame: engine {:.2} ms, scene {:.2} ms ({} widgets, Blizzard's UI {})",
        ms(engine),
        ms(scene),
        wowlua_engine::engine(vm).world.widgets.len(),
        if suite.blizzard_ui.is_some() {
            "loaded"
        } else {
            "missing"
        }
    );
}
