//! The running addon drawn on the page's canvas (spec M14 S3, decision
//! T396): the renderer's frames into a canvas surface, WebGPU where the
//! browser has it, else WebGL 2. Pictures and fonts come from the addon's
//! folder and the user's installation when the page opened it (S5,
//! T398); without it Blizzard's are the renderer's placeholders and
//! stand-in fonts.

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wowlua_render::Renderer;

/// A canvas the running addon is drawn on.
#[wasm_bindgen]
pub struct Screen {
    /// (shared with a snapshot reading the GPU's answer back)
    renderer: std::rc::Rc<std::cell::RefCell<Renderer>>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    backend: String,
}

/// Draws on `canvas` (an `HTMLCanvasElement`, or an `OffscreenCanvas` in a
/// worker), at its size.
#[wasm_bindgen(js_name = createScreen)]
pub async fn create_screen(canvas: JsValue) -> Result<Screen, JsError> {
    // WebGPU when the browser has it and an adapter for it, else WebGL 2:
    // decided before the canvas gets a context, as it takes only one kind.
    let with = |backends| {
        wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        })
    };
    let mut backends = wgpu::Backends::GL;
    if wgpu::util::is_browser_webgpu_supported().await
        && with(wgpu::Backends::BROWSER_WEBGPU)
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .is_ok()
    {
        backends = wgpu::Backends::BROWSER_WEBGPU;
    }
    let instance = with(backends);
    let (target, width, height) = match canvas.dyn_into::<web_sys::HtmlCanvasElement>() {
        Ok(c) => {
            let (w, h) = (c.width(), c.height());
            (wgpu::SurfaceTarget::Canvas(c), w, h)
        }
        Err(other) => {
            let c: web_sys::OffscreenCanvas = other
                .dyn_into()
                .map_err(|_| JsError::new("a canvas or an OffscreenCanvas to draw on"))?;
            let (w, h) = (c.width(), c.height());
            (wgpu::SurfaceTarget::OffscreenCanvas(c), w, h)
        }
    };
    let surface = instance
        .create_surface(target)
        .map_err(|e| JsError::new(&format!("the canvas: {e}")))?;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            ..Default::default()
        })
        .await
        .map_err(|e| JsError::new(&format!("no WebGPU or WebGL 2 to draw with: {e}")))?;
    let info = adapter.get_info();
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("wowlua"),
            required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                .using_resolution(adapter.limits()),
            ..Default::default()
        })
        .await
        .map_err(|e| JsError::new(&format!("the GPU device: {e}")))?;
    let caps = surface.get_capabilities(&adapter);
    if !caps.formats.contains(&wowlua_render::FORMAT) {
        return Err(JsError::new(&format!(
            "the canvas cannot take {:?} (it takes {:?})",
            wowlua_render::FORMAT,
            caps.formats
        )));
    }
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: wowlua_render::FORMAT,
        color_space: wgpu::SurfaceColorSpace::Auto,
        width: width.max(1),
        height: height.max(1),
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: 2,
        // (the world behind the UI is black: the renderer's alpha is not
        // the screen's, as its screenshots make every pixel opaque)
        alpha_mode: if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            caps.alpha_modes[0]
        },
        view_formats: vec![],
    };
    surface.configure(&device, &config);
    let renderer = Renderer::with_device(device.clone(), queue.clone(), info.name.clone());
    Ok(Screen {
        renderer: std::rc::Rc::new(std::cell::RefCell::new(renderer)),
        surface,
        device,
        queue,
        config,
        backend: format!("{:?}", info.backend),
    })
}

#[wasm_bindgen]
impl Screen {
    /// What draws: `BrowserWebGpu` or `Gl`.
    #[wasm_bindgen(getter)]
    pub fn backend(&self) -> String {
        self.backend.clone()
    }

    /// The canvas's new size in pixels.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    /// Draws the running addon's screen, scaled to the canvas's width.
    pub fn draw(&mut self, playground: &mut crate::web::WebPlayground) -> Result<(), JsError> {
        let Some(game) = playground.inner().game() else {
            return Ok(());
        };
        let mut assets = game.assets();
        let scene = wowlua_engine::scene::build(game.vm());
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            other => {
                // (lost or outdated: configured again, drawn next time)
                self.surface.configure(&self.device, &self.config);
                return match other {
                    wgpu::CurrentSurfaceTexture::Timeout
                    | wgpu::CurrentSurfaceTexture::Occluded
                    | wgpu::CurrentSurfaceTexture::Outdated
                    | wgpu::CurrentSurfaceTexture::Lost => Ok(()),
                    _ => Err(JsError::new("the canvas cannot be drawn on")),
                };
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let scale = self.config.width as f32 / scene.width.max(1) as f32;
        // (a snapshot being read back: this frame is skipped)
        let Ok(mut renderer) = self.renderer.try_borrow_mut() else {
            return Ok(());
        };
        let encoder = renderer.draw(
            &scene,
            &view,
            self.config.width,
            self.config.height,
            scale,
            &mut assets,
        );
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        Ok(())
    }

    /// The running addon's screen as a PNG (its size in pixels, times
    /// `scale`), drawn off the canvas and read back: what
    /// `wowlua screenshot` gives.
    // (the renderer stays borrowed while the GPU answers: a frame drawn
    // meanwhile is skipped, see `draw`)
    #[allow(clippy::await_holding_refcell_ref)]
    pub fn snapshot(
        &mut self,
        playground: &mut crate::web::WebPlayground,
        scale: f32,
    ) -> Result<js_sys::Promise, JsError> {
        let game = playground
            .inner()
            .game()
            .ok_or_else(|| JsError::new("no addon runs"))?;
        let mut assets = game.assets();
        let scene = wowlua_engine::scene::build(game.vm());
        let renderer = self.renderer.clone();
        Ok(wasm_bindgen_futures::future_to_promise(async move {
            let mut renderer = renderer
                .try_borrow_mut()
                .map_err(|_| JsValue::from_str("a snapshot is being taken"))?;
            let image = renderer
                .render(&scene, scale.max(0.1), None, &mut assets)
                .await
                .map_err(|e| JsValue::from_str(&e))?;
            let png = wowlua_render::encode_png(&image).map_err(|e| JsValue::from_str(&e))?;
            Ok(js_sys::Uint8Array::from(&png[..]).into())
        }))
    }
}
