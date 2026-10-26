//! wowlua in a web page (spec M14 S2, decision T395): a package held in
//! memory that the page edits, checks, builds, formats and shares, and the
//! language server the page's editor talks to. [`Playground`] is the API in
//! Rust (the tests use it natively); [`web`] makes it JavaScript's.
//!
//! The files live in a [`MemoryHost`] installed for the whole program: one
//! playground per WebAssembly instance. Paths the page names are relative
//! to the package's folder, [`ROOT`]; the language server's URIs are
//! `file:///playground/…`.

pub mod debug;
pub mod run;
#[cfg(target_arch = "wasm32")]
mod screen;
mod share;
pub mod web;

use serde_json::{Value as Json, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use wowlua_diagnostics::Severity;
use wowlua_driver::host::{self, MemoryHost};
use wowlua_driver::{BuildRequest, Session};

/// The package's folder.
pub const ROOT: &str = "/playground";

/// A new package's files: its manifest and a first source file.
pub const TEMPLATE: &[(&str, &str)] = &[
    (
        "wowlua.json",
        r#"{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.11"
    },
    "flavors": {
        "camelot": {}
    }
}
"#,
    ),
    (
        "src/main.wowlua",
        r#"local frame = CreateFrame("Frame")
frame:RegisterEvent("PLAYER_LOGIN")
frame:SetScript("OnEvent", function()
    print("Hello from the playground!")
end)
"#,
    ),
];

/// The package the page edits, and its language server.
pub struct Playground {
    memory: Arc<MemoryHost>,
    server: wowlua_lsp::embedded::Embedded,
    game: Option<run::Game>,
}

impl Default for Playground {
    fn default() -> Playground {
        Playground::new()
    }
}

impl Playground {
    /// A playground with [`TEMPLATE`]'s files, its host installed.
    pub fn new() -> Playground {
        let memory = Arc::new(MemoryHost::new());
        host::install(Some(memory.clone()));
        let playground = Playground {
            memory,
            server: wowlua_lsp::embedded::Embedded::new(),
            game: None,
        };
        playground.reset(TEMPLATE.iter().map(|(p, t)| (p.to_string(), t.to_string())));
        playground
    }

    /// The clocks: the wall clock (milliseconds since 1970, for builds'
    /// dates) and a steady one (milliseconds from any start, for the
    /// language server's timing). A page sets them before each call.
    pub fn set_time(&mut self, unix_ms: f64, steady_ms: f64) {
        self.memory
            .set_clock(SystemTime::UNIX_EPOCH + Duration::from_secs_f64(unix_ms.max(0.0) / 1000.0));
        self.server
            .set_time(Duration::from_secs_f64(steady_ms.max(0.0) / 1000.0));
    }

    fn path(rel: &str) -> PathBuf {
        Path::new(ROOT).join(rel.trim_start_matches('/'))
    }

    /// Puts a file there (and its folders), replacing what was there.
    pub fn write_file(&self, rel: &str, text: &str) {
        self.memory
            .insert(Playground::path(rel), text.as_bytes().to_vec());
    }

    /// A file's text: the package's (relative), or any the host has by an
    /// absolute path, such as the built-in declarations go to definition
    /// leads to.
    pub fn read_file(&self, path: &str) -> Option<String> {
        let path = if path.starts_with('/') {
            PathBuf::from(path)
        } else {
            Playground::path(path)
        };
        host::read_to_string(path).ok()
    }

    /// Puts bytes at an absolute path of the host (Blizzard's UI files the
    /// page fetched).
    pub fn write_host_file(&self, path: &str, bytes: &[u8]) {
        self.memory.insert(path, bytes.to_vec());
    }

    /// Whether the host has a file at an absolute path.
    pub fn has_host_file(&self, path: &str) -> bool {
        host::is_file(path)
    }

    /// Removes a file (or a folder and what is in it).
    pub fn delete_file(&self, rel: &str) {
        let path = Playground::path(rel);
        let _ = host::remove_file(&path).or_else(|_| host::remove_dir_all(&path));
    }

    /// The package's files, by relative path, without what builds wrote.
    pub fn files(&self) -> Vec<(String, String)> {
        let out = Playground::path("dist");
        self.memory
            .files()
            .into_iter()
            .filter(|(p, _)| p.starts_with(ROOT) && !p.starts_with(&out))
            .filter_map(|(p, bytes)| {
                let rel = p
                    .strip_prefix(ROOT)
                    .ok()?
                    .to_string_lossy()
                    .replace('\\', "/");
                Some((rel, String::from_utf8_lossy(&bytes).into_owned()))
            })
            .collect()
    }

    /// Replaces every file of the package with `files`.
    pub fn reset(&self, files: impl IntoIterator<Item = (String, String)>) {
        let _ = host::remove_dir_all(ROOT);
        let _ = host::create_dir_all(ROOT);
        for (rel, text) in files {
            self.write_file(&rel, &text);
        }
    }

    fn load(&self, session: &mut Session) -> Option<wowlua_driver::Project> {
        let options = wowlua_driver::LoadOptions {
            cwd: ROOT.into(),
            offline: true,
            ..Default::default()
        };
        wowlua_driver::load_project(session, &options)
    }

    /// Builds the package as `wowlua build --profile <profile>` does (the
    /// output is not written): whether it built, its diagnostics and its
    /// files, the AddOn folder's (`{ path, text }`, or `{ path, base64 }`
    /// for one that is no UTF-8 text, such as a font; URL-safe base64).
    pub fn build(&self, profile: &str) -> Json {
        self.build_with(profile, true)
    }

    /// [`build`](Self::build), keeping the compiled code or not (a check
    /// reads none of it).
    fn build_with(&self, profile: &str, keep_code: bool) -> Json {
        let mut session = Session::new();
        let built = self.load(&mut session).and_then(|project| {
            let request = BuildRequest {
                profile: profile.to_string(),
                flavors: None,
                out_dir: None,
                write: false,
                nolib: false,
                minify: None,
                bundle: None,
                only: None,
                jobs: wowlua_driver::parallel::Jobs::one(),
                keep_code,
                tests: false,
                left_out: Vec::new(),
            };
            wowlua_driver::build(&mut session, &project, &request)
        });
        let ok = built.is_some() && !session.has_errors();
        let files: Vec<Json> = built
            .map(|b| {
                b.files
                    .iter()
                    .map(|(path, content)| {
                        let bytes = match content {
                            wowlua_driver::build::Content::Bytes(b) => b.clone(),
                            wowlua_driver::build::Content::Copy(p) => {
                                host::read(p).unwrap_or_default()
                            }
                        };
                        match String::from_utf8(bytes) {
                            Ok(text) => json!({ "path": path, "text": text }),
                            Err(e) => {
                                json!({ "path": path, "base64": share::base64(e.as_bytes()) })
                            }
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        json!({ "ok": ok, "diagnostics": diagnostics(&mut session), "files": files })
    }

    /// Checks the package (`wowlua check`): its diagnostics.
    pub fn check(&self) -> Json {
        let mut built = self.build_with("dev", false);
        built["diagnostics"].take()
    }

    /// Formats a file of the package as `wowlua fmt` does: its new text, or
    /// the diagnostics when it could not be formatted (a syntax error).
    pub fn format(&self, rel: &str) -> Json {
        let mut session = Session::new();
        let Some(project) = self.load(&mut session) else {
            return json!({ "diagnostics": diagnostics(&mut session) });
        };
        let request = wowlua_driver::format::FormatRequest {
            paths: vec![Playground::path(rel)],
            check: false,
        };
        wowlua_driver::format::format(&mut session, &project, &request);
        if session.has_errors() {
            return json!({ "diagnostics": diagnostics(&mut session) });
        }
        json!({ "text": self.read_file(rel) })
    }

    /// Builds the package for `options.flavor` and runs it (the running one
    /// stops): `{ ok, diagnostics, error }`.
    pub fn run(&mut self, options: run::Options) -> Json {
        self.game = None;
        let mut session = Session::new();
        let Some(project) = self.load(&mut session) else {
            return json!({ "ok": false, "diagnostics": diagnostics(&mut session) });
        };
        let request = BuildRequest {
            profile: "dev".into(),
            flavors: Some(vec![options.flavor.clone()]),
            out_dir: None,
            write: true,
            nolib: false,
            minify: None,
            bundle: None,
            only: None,
            jobs: wowlua_driver::parallel::Jobs::one(),
            keep_code: true,
            tests: false,
            left_out: Vec::new(),
        };
        let built = wowlua_driver::build(&mut session, &project, &request);
        let found = diagnostics(&mut session);
        let Some(build) = built.filter(|_| !session.has_errors()) else {
            if let Some(setup) = options.debug {
                let shared = wowlua_debug::Shared::new(
                    setup.link,
                    Box::new(|_| None),
                    std::rc::Rc::new(|| 0),
                );
                debug::end(&shared, Some("the package did not build".into()));
            }
            return json!({ "ok": false, "diagnostics": found });
        };
        match run::Game::start(project, build, options) {
            Ok(game) => {
                self.game = Some(game);
                json!({ "ok": true, "diagnostics": found })
            }
            Err(error) => json!({ "ok": false, "diagnostics": found, "error": error }),
        }
    }

    /// The running addon, if one runs (one whose debugger ended it is
    /// stopped here).
    pub fn game(&mut self) -> Option<&mut run::Game> {
        if self.game.as_ref().is_some_and(|g| g.ended()) {
            self.game = None;
        }
        self.game.as_mut()
    }

    /// Stops the running addon.
    pub fn stop(&mut self) {
        self.game = None;
    }

    /// The package's files in a link's fragment (compressed, URL-safe).
    pub fn share(&self) -> String {
        share::encode(&self.files())
    }

    /// Opens a shared package: its files replace these. False (and nothing
    /// changed) when `link` is not one.
    pub fn open_shared(&self, link: &str) -> bool {
        match share::decode(link) {
            Some(files) => {
                self.reset(files);
                true
            }
            None => false,
        }
    }

    /// Hands the language server one of the editor's messages; its answers
    /// and notifications.
    pub fn lsp_message(&mut self, message: &Json) -> Vec<Json> {
        self.server.message(message)
    }

    /// How long until [`lsp_run_due`](Self::lsp_run_due) has a check to
    /// run; `None` when none waits.
    pub fn lsp_wait(&self) -> Option<Duration> {
        self.server.wait()
    }

    /// Runs the language server's checks that are due; what they told the
    /// editor.
    pub fn lsp_run_due(&mut self) -> Vec<Json> {
        self.server.run_due()
    }
}

/// A session's diagnostics: `{ severity, code, message, file, range }`,
/// `file` relative to the package (`null` for the package as a whole) and
/// `range` as the language server's (lines and UTF-16 columns from 0).
fn diagnostics(session: &mut Session) -> Json {
    let found = session.take_diagnostics().into_vec();
    let items: Vec<Json> = found
        .iter()
        .map(|d| {
            let severity = match d.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
                Severity::Note => "note",
            };
            let mut message = d.message.clone();
            if let Some(label) = d.labels.first().filter(|l| !l.message.is_empty()) {
                message.push_str(&format!(": {}", label.message));
            }
            for note in &d.notes {
                message.push_str(&format!("\n{note}"));
            }
            if let Some(help) = &d.help {
                message.push_str(&format!("\nhelp: {help}"));
            }
            let (file, range) = match d.labels.first() {
                Some(label) => {
                    let file = session.db.get(label.span.file);
                    let r = label.span.range();
                    (
                        Some(relative(file.path())),
                        wowlua_lsp::text::range(file.text(), r.start, r.end),
                    )
                }
                None => (
                    d.path.as_deref().map(relative),
                    wowlua_lsp::text::range("", 0, 0),
                ),
            };
            json!({
                "severity": severity,
                "code": d.code,
                "message": message,
                "file": file,
                "range": range,
            })
        })
        .collect();
    Json::Array(items)
}

/// A session's path relative to the package (they are relative to its
/// folder, or absolute).
fn relative(path: &Path) -> String {
    path.strip_prefix(ROOT)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
