//! [`Playground`] for JavaScript (wasm-bindgen): JSON in and out as
//! strings, so the page's code needs nothing of Rust's. `@wowlua/web`
//! (`web/` in the repository) wraps this in its JavaScript API.

use crate::Playground;
use serde_json::Value as Json;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn error(message: &str);
}

/// A panic's message goes to the console before the module ends: the page
/// can tell what failed.
fn report_panics() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| error(&format!("wowlua panicked: {info}"))));
    });
}

#[wasm_bindgen(js_name = Playground)]
pub struct WebPlayground(Playground);

impl Default for WebPlayground {
    fn default() -> WebPlayground {
        WebPlayground::new()
    }
}

impl WebPlayground {
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn inner(&mut self) -> &mut Playground {
        &mut self.0
    }
}

#[wasm_bindgen(js_class = Playground)]
impl WebPlayground {
    /// A playground with the template package (one per instance).
    #[wasm_bindgen(constructor)]
    pub fn new() -> WebPlayground {
        report_panics();
        WebPlayground(Playground::new())
    }

    /// wowlua's version.
    #[wasm_bindgen(js_name = version)]
    pub fn version() -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    /// `Date.now()` and `performance.now()`.
    #[wasm_bindgen(js_name = setTime)]
    pub fn set_time(&mut self, unix_ms: f64, steady_ms: f64) {
        self.0.set_time(unix_ms, steady_ms);
    }

    #[wasm_bindgen(js_name = writeFile)]
    pub fn write_file(&self, path: &str, text: &str) {
        self.0.write_file(path, text);
    }

    #[wasm_bindgen(js_name = readFile)]
    pub fn read_file(&self, path: &str) -> Option<String> {
        self.0.read_file(path)
    }

    #[wasm_bindgen(js_name = deleteFile)]
    pub fn delete_file(&self, path: &str) {
        self.0.delete_file(path);
    }

    /// `{ "<path>": "<text>", … }`.
    pub fn files(&self) -> String {
        let map: serde_json::Map<String, Json> = self
            .0
            .files()
            .into_iter()
            .map(|(p, t)| (p, Json::String(t)))
            .collect();
        Json::Object(map).to_string()
    }

    /// Replaces the package's files with `{ "<path>": "<text>", … }`.
    pub fn reset(&self, files: &str) -> Result<(), JsError> {
        let value: Json = serde_json::from_str(files)?;
        let map = value
            .as_object()
            .ok_or_else(|| JsError::new("files are an object of texts"))?;
        self.0.reset(
            map.iter()
                .map(|(p, t)| (p.clone(), t.as_str().unwrap_or_default().to_string())),
        );
        Ok(())
    }

    /// `{ ok, diagnostics, files }`.
    pub fn build(&self, profile: &str) -> String {
        self.0.build(profile).to_string()
    }

    /// The diagnostics.
    pub fn check(&self) -> String {
        self.0.check().to_string()
    }

    /// `{ text }` or `{ diagnostics }`.
    pub fn format(&self, path: &str) -> String {
        self.0.format(path).to_string()
    }

    pub fn share(&self) -> String {
        self.0.share()
    }

    #[wasm_bindgen(js_name = openShared)]
    pub fn open_shared(&self, link: &str) -> bool {
        self.0.open_shared(link)
    }

    /// Puts bytes at an absolute path (Blizzard's UI files the page fetched).
    #[wasm_bindgen(js_name = writeHostFile)]
    pub fn write_host_file(&self, path: &str, bytes: &[u8]) {
        self.0.write_host_file(path, bytes);
    }

    #[wasm_bindgen(js_name = hasHostFile)]
    pub fn has_host_file(&self, path: &str) -> bool {
        self.0.has_host_file(path)
    }

    /// What to fetch for a flavor's Blizzard UI (JSON), or `undefined`.
    #[wasm_bindgen(js_name = blizzardUI)]
    pub fn blizzard_ui(flavor: &str) -> Option<String> {
        crate::run::blizzard_ui(flavor).map(|j| j.to_string())
    }

    /// Builds and runs the package: `options` is `{ flavor, blizzardUI,
    /// profile, debug }` (`debug`: the debugger's setup, see
    /// `debug::Setup`; the worker's mailbox carries its requests);
    /// `{ ok, diagnostics, error }`.
    pub fn run(&mut self, options: &str) -> Result<String, JsError> {
        let o: Json = serde_json::from_str(options)?;
        let debug = match &o["debug"] {
            Json::Null | Json::Bool(false) => None,
            settings => Some(crate::debug::Setup {
                link: page_link()?,
                settings: if settings.is_object() {
                    settings.clone()
                } else {
                    Json::Null
                },
            }),
        };
        let options = crate::run::Options {
            flavor: o["flavor"].as_str().unwrap_or("camelot").to_string(),
            blizzard_ui: o["blizzardUI"].as_bool().unwrap_or(false),
            profile: o["profile"].as_str().map(str::to_string),
            debug,
            installation: None,
        };
        Ok(self.0.run(options).to_string())
    }

    /// Opens `flavor`'s client from the installation the worker was given
    /// (`wowluaInstall*`): `{ flavor, product }`.
    #[wasm_bindgen(js_name = useInstallation)]
    pub fn use_installation(&mut self, flavor: &str) -> Result<String, JsError> {
        #[cfg(target_arch = "wasm32")]
        {
            let info = self
                .0
                .use_installation(Box::new(crate::installation::PageStorage), flavor)
                .map_err(|e| JsError::new(&e))?;
            Ok(info.to_string())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = flavor;
            Err(JsError::new("the installation needs a page"))
        }
    }

    pub fn stop(&mut self) {
        self.0.stop();
    }

    /// Whether an addon runs.
    #[wasm_bindgen(getter)]
    pub fn running(&mut self) -> bool {
        self.0.game().is_some()
    }

    /// Moves the running addon's time to `now_ms` (`performance.now()`):
    /// a JSON array of what it printed and raised (`{ kind, text, stack }`).
    pub fn advance(&mut self, now_ms: f64) -> Result<String, JsError> {
        let Some(game) = self.0.game() else {
            return Ok("[]".into());
        };
        let out = game.advance(now_ms).map_err(|e| JsError::new(&e))?;
        Ok(Json::Array(out).to_string())
    }

    /// Input as the user gives it (JSON: `{ action: "click", x, y,
    /// button }`, `hover`, `type`, `key`, `slash`); screen pixels from the
    /// top left.
    pub fn input(&mut self, input: &str) -> Result<String, JsError> {
        let input: Json = serde_json::from_str(input)?;
        let game = self.0.game().ok_or_else(|| JsError::new("no addon runs"))?;
        game.input(&input).map_err(|e| JsError::new(&e))?;
        Ok(Json::Array(game.take_output()).to_string())
    }

    /// The running addon's screen size in pixels, `[width, height]`.
    #[wasm_bindgen(js_name = screenSize)]
    pub fn screen_size(&mut self) -> Vec<u32> {
        match self.0.game() {
            Some(g) => {
                let (w, h) = g.screen();
                vec![w, h]
            }
            None => Vec::new(),
        }
    }

    /// One JSON-RPC message from the editor; a JSON array of the server's.
    #[wasm_bindgen(js_name = lspMessage)]
    pub fn lsp_message(&mut self, message: &str) -> Result<String, JsError> {
        let message: Json = serde_json::from_str(message)?;
        Ok(Json::Array(self.0.lsp_message(&message)).to_string())
    }

    /// Milliseconds until `lspRunDue` has a check to run; -1 for none.
    #[wasm_bindgen(js_name = lspWait)]
    pub fn lsp_wait(&self) -> f64 {
        self.0.lsp_wait().map_or(-1.0, |d| d.as_secs_f64() * 1000.0)
    }

    /// Runs the due checks; a JSON array of the server's messages.
    #[wasm_bindgen(js_name = lspRunDue)]
    pub fn lsp_run_due(&mut self) -> String {
        Json::Array(self.0.lsp_run_due()).to_string()
    }
}

/// The debugger's link to the page (the worker's mailbox).
fn page_link() -> Result<Box<dyn wowlua_debug::Link>, JsError> {
    #[cfg(target_arch = "wasm32")]
    {
        Ok(Box::new(crate::debug::PageLink))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(JsError::new("the debugger needs a page"))
    }
}
