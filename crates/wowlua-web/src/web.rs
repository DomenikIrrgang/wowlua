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
