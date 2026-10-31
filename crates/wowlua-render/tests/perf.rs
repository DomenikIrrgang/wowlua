//! How long a frame of the 3D world takes (decision T538): Northshire's
//! start among its 27 test NPCs (T533), the names and nameplates on, the
//! player running; the engine's frame, the scene, the drawing (the CPU's
//! part, as Studio's window pays it: posing and encoding the frame) and
//! the GPU's time after it timed apart, each frame drawn with its units
//! posed on the GPU and again on the CPU (T540). Run by hand: `cargo test --release -p wowlua-render --test perf
//! -- --ignored --nocapture`.

use std::time::{Duration, Instant};
use wowlua_vm::{Value, Vm};

fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm.load(src.as_bytes(), "@Perf.lua").expect("compiles");
    let text = |vm: &Vm, v: Value| match vm.hidden(v) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) => format!("{n}"),
        other => format!("{other:?}"),
    };
    match vm.call(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

/// Northshire's NPCs as the built-in profile places them: on two arcs
/// north of the start, and five to fight south-east.
const PEOPLE: &str = "mock.setPlayer({ race = 'Human', class = 'WARRIOR', sex = 2, level = 1 })
mock.moveTo({ world = { y = -8949.95, x = -132.49, z = 0, instanceID = 0 }, facing = 0 })
for _, n in ipairs({ 'nameplateShowEnemies', 'nameplateShowFriendlyNpcs', 'UnitNameInteractiveNPC', 'UnitNameHostleNPC' }) do
    C_CVar.SetCVar(n, '1')
end
local displays = { 1261, 1276, 1277, 1288, 1287, 1285, 1280, 1440, 1289, 1290, 1543, 1291, 1295, 1832, 1296, 1527, 1890 }
local function arc(i, n, r, span)
    local a = math.rad(span / 2 - i * span / (n - 1))
    return -8949.95 + r * math.cos(a), -132.49 + r * math.sin(a)
end
for i = 0, 16 do
    local y, x
    if i < 8 then y, x = arc(i, 8, 16, 120) else y, x = arc(i - 8, 9, 24, 100) end
    local u = mock.spawnUnit({ name = 'Npc' .. i, title = 'Test Title', reaction = 'friendly', level = 30,
        displayID = displays[i + 1], world = { y = y, x = x, z = 0 } })
    mock.defineVendor(u, { items = { { id = 159 } } })
end
for i = 0, 4 do
    mock.spawnUnit({ name = 'Foe' .. i, reaction = 'hostile', level = 3, displayID = ({ 163, 365, 903, 193, 706 })[i + 1],
        world = { y = -8960 - i * 3, x = -150 - i * 2, z = 0 } })
end
for i = 0, 4 do
    mock.spawnUnit({ name = 'Service' .. i, title = 'Banker', reaction = 'friendly', level = 30,
        displayID = 1292 + i, world = { y = -8954 + i * 4, x = -104, z = 0 } })
end
mock.advanceFrames(2, 1000 / 60)
MoveForwardStart(GetTime())
";

#[test]
#[ignore]
fn a_frame_of_northshire() {
    let root = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let storage = wowlua_casc::FsStorage { root };
    let product = wowlua_casc::Casc::product_of_folder(&storage, "_forever_").expect("camelot");
    let casc =
        std::sync::Arc::new(wowlua_casc::Casc::open(Box::new(storage), &product).expect("opens"));
    let for_data = casc.clone();
    let data = std::sync::Arc::new(wowlua_render::client::CascClientData::new(Box::new(
        move || Some(for_data.clone()),
    )));
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
    wowlua_engine::clientdata::set(&mut vm, data);
    wowlua_engine::set_world(&mut vm, true);
    // (PERF_ALONE: the player alone, what the units cost apart)
    let people = if std::env::var_os("PERF_ALONE").is_some() {
        PEOPLE
            .split("local displays")
            .next()
            .unwrap_or(PEOPLE)
            .to_string()
            + "MoveForwardStart(GetTime())"
    } else {
        PEOPLE.to_string()
    };
    run(&mut vm, &people);
    let mut assets = wowlua_render::client::ClientAssets::new(Some(casc), &[]);
    // (a device as Studio's window gets one: wgpu's default limits)
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).expect("a GPU");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
        ..Default::default()
    }))
    .expect("a device");
    // (each frame drawn twice: its units posed on the GPU, then on the
    // CPU as on WebGL; T540)
    let name = adapter.get_info().name;
    let mut renderers = [
        wowlua_render::Renderer::with_device(device.clone(), queue.clone(), name.clone()),
        wowlua_render::Renderer::with_device(device.clone(), queue.clone(), name),
    ];
    renderers[1].skin_on_the_cpu();
    let mut target: Option<(wgpu::TextureView, [u32; 2])> = None;
    let (mut engine, mut scene_t) = (Duration::ZERO, Duration::ZERO);
    let (mut draw, mut gpu) = ([Duration::ZERO; 2], [Duration::ZERO; 2]);
    let frames: u32 = std::env::var("PERF_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    for i in 0..frames + 3 {
        let t = Instant::now();
        wowlua_engine::clock::advance_frames(&mut vm, 1, 1000.0 / 60.0);
        let e = t.elapsed();
        let t = Instant::now();
        let scene = wowlua_engine::scene::build(&mut vm);
        let s = t.elapsed();
        let size = [scene.width.max(1), scene.height.max(1)];
        if target.as_ref().is_none_or(|(_, s)| *s != size) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("perf"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wowlua_render::FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            target = Some((texture.create_view(&Default::default()), size));
        }
        let view = &target.as_ref().expect("the target").0;
        // (the first frames read the world: not counted)
        if i >= 3 {
            engine += e;
            scene_t += s;
        }
        for (k, r) in renderers.iter_mut().enumerate() {
            let t = Instant::now();
            let encoder = r.draw(&scene, view, size[0], size[1], 1.0, &mut assets);
            queue.submit([encoder.finish()]);
            let d = t.elapsed();
            let t = Instant::now();
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            let g = t.elapsed();
            if i >= 3 {
                draw[k] += d;
                gpu[k] += g;
            }
        }
    }
    let ms = |d: Duration| d.as_secs_f64() * 1000.0 / f64::from(frames);
    eprintln!(
        "a frame: engine {:.2} ms, scene {:.2} ms",
        ms(engine),
        ms(scene_t)
    );
    for (k, how) in ["on the GPU", "on the CPU"].iter().enumerate() {
        eprintln!(
            "units posed {how}: drawing {:.2} ms, the GPU after it {:.2} ms",
            ms(draw[k]),
            ms(gpu[k])
        );
    }
}
