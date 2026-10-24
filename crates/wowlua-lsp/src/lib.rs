//! `wowlua lsp`: the language server (spec §16.2) over stdio. It shares the
//! compiler: a package is checked as `wowlua check` checks it, with the
//! editor's unsaved texts over the disk's, a moment after the last edit
//! and on every save; documents are formatted as `wowlua fmt` formats them,
//! and outlined from their syntax.

pub mod check;
pub mod symbols;
pub mod text;
pub mod transport;

use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How long after the last edit a package is checked again.
const SETTLE: Duration = Duration::from_millis(350);

/// An open document.
struct Doc {
    path: PathBuf,
    text: String,
}

#[derive(Default)]
struct Server {
    docs: HashMap<String, Doc>,
    /// Packages (by manifest) to check again, and since when.
    dirty: BTreeMap<PathBuf, Instant>,
    /// The files each package's last check reported, to clear them.
    published: HashMap<PathBuf, BTreeSet<PathBuf>>,
    shutdown: bool,
}

/// Runs the server on stdin and stdout until the client says `exit`; true
/// when it said `shutdown` before (the protocol's exit code 0).
pub fn run() -> std::io::Result<bool> {
    let (tx, rx) = mpsc::channel::<Json>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut input = stdin.lock();
        while let Ok(Some(message)) = transport::read(&mut input) {
            if tx.send(message).is_err() {
                break;
            }
        }
    });
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    serve(&rx, &mut out)
}

/// The server's loop over `messages`, writing to `out`; true when the client
/// said `shutdown` before `exit`.
pub fn serve(
    messages: &mpsc::Receiver<Json>,
    out: &mut impl std::io::Write,
) -> std::io::Result<bool> {
    let mut server = Server::default();
    loop {
        let wait = if server.dirty.is_empty() {
            Duration::from_secs(3600)
        } else {
            SETTLE
        };
        match messages.recv_timeout(wait) {
            Ok(message) => {
                if handle(&mut server, &message, out)? {
                    return Ok(server.shutdown);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(server.shutdown),
        }
        // the packages edits settled in
        let now = Instant::now();
        let due: Vec<PathBuf> = server
            .dirty
            .iter()
            .filter(|(_, at)| now.duration_since(**at) >= SETTLE)
            .map(|(m, _)| m.clone())
            .collect();
        for manifest in due {
            server.dirty.remove(&manifest);
            publish(&mut server, &manifest, out)?;
        }
    }
}

fn reply(out: &mut impl std::io::Write, id: &Json, result: Json) -> std::io::Result<()> {
    transport::write(
        out,
        &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
    )
}

fn notify(out: &mut impl std::io::Write, method: &str, params: Json) -> std::io::Result<()> {
    transport::write(
        out,
        &json!({ "jsonrpc": "2.0", "method": method, "params": params }),
    )
}

/// The manifest of the package a file belongs to.
fn manifest_of(path: &Path) -> Option<PathBuf> {
    wowlua_manifest::find_manifest(path.parent()?, None)
}

/// Handles one message; true when the server should stop.
fn handle(server: &mut Server, m: &Json, out: &mut impl std::io::Write) -> std::io::Result<bool> {
    let method = m["method"].as_str().unwrap_or("");
    let id = &m["id"];
    let params = &m["params"];
    let uri = params["textDocument"]["uri"]
        .as_str()
        .unwrap_or("")
        .to_string();
    match method {
        "initialize" => reply(
            out,
            id,
            json!({
                "capabilities": {
                    "textDocumentSync": { "openClose": true, "change": 1, "save": { "includeText": false } },
                    "documentFormattingProvider": true,
                    "documentSymbolProvider": true,
                },
                "serverInfo": { "name": "wowlua", "version": env!("CARGO_PKG_VERSION") },
            }),
        )?,
        "shutdown" => {
            server.shutdown = true;
            reply(out, id, Json::Null)?;
        }
        "exit" => return Ok(true),
        "textDocument/didOpen" | "textDocument/didChange" => {
            let text = if method == "textDocument/didOpen" {
                params["textDocument"]["text"].as_str()
            } else {
                // full sync: the last change is the whole text
                params["contentChanges"]
                    .as_array()
                    .and_then(|c| c.last())
                    .and_then(|c| c["text"].as_str())
            };
            if let (Some(text), Some(path)) = (text, text::path_of(&uri)) {
                wowlua_driver::overlay::set(&path, text.as_bytes().to_vec());
                if let Some(manifest) = manifest_of(&path) {
                    server.dirty.insert(manifest, Instant::now());
                }
                server.docs.insert(
                    uri,
                    Doc {
                        path,
                        text: text.to_string(),
                    },
                );
            }
        }
        "textDocument/didSave" => {
            if let Some(manifest) = text::path_of(&uri).and_then(|p| manifest_of(&p)) {
                // now: a save is checked at once
                server.dirty.insert(
                    manifest,
                    Instant::now()
                        .checked_sub(SETTLE)
                        .unwrap_or_else(Instant::now),
                );
            }
        }
        "textDocument/didClose" => {
            if let Some(doc) = server.docs.remove(&uri) {
                wowlua_driver::overlay::remove(&doc.path);
                if let Some(manifest) = manifest_of(&doc.path) {
                    server.dirty.insert(manifest, Instant::now());
                }
            }
        }
        "textDocument/formatting" => {
            let edits = server.docs.get(&uri).and_then(format).unwrap_or_default();
            reply(out, id, Json::Array(edits))?;
        }
        "textDocument/documentSymbol" => {
            let symbols = server
                .docs
                .get(&uri)
                .map(|d| symbols::symbols(&d.text))
                .unwrap_or_default();
            reply(out, id, Json::Array(symbols))?;
        }
        _ => {
            // an unknown request still gets an answer
            if !id.is_null() {
                transport::write(
                    out,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32601, "message": format!("{method} is not supported") },
                    }),
                )?;
            }
        }
    }
    Ok(false)
}

/// The package's diagnostics to the client, its files without any cleared.
fn publish(
    server: &mut Server,
    manifest: &Path,
    out: &mut impl std::io::Write,
) -> std::io::Result<()> {
    let found = check::check(manifest);
    let now: BTreeSet<PathBuf> = found.keys().cloned().collect();
    for (path, items) in &found {
        notify(
            out,
            "textDocument/publishDiagnostics",
            json!({ "uri": text::uri_of(path), "diagnostics": items }),
        )?;
    }
    if let Some(before) = server.published.get(manifest) {
        for gone in before.difference(&now) {
            notify(
                out,
                "textDocument/publishDiagnostics",
                json!({ "uri": text::uri_of(gone), "diagnostics": [] }),
            )?;
        }
    }
    server.published.insert(manifest.to_path_buf(), now);
    Ok(())
}

/// The document formatted as its package's `fmt` settings say: one edit
/// replacing the whole text, none when it is formatted already or does not
/// parse.
fn format(doc: &Doc) -> Option<Vec<Json>> {
    let wowlua = doc.path.extension().is_some_and(|e| e == "wowlua");
    if !wowlua {
        return None;
    }
    let mut db = wowlua_span::SourceDb::new();
    let config = manifest_of(&doc.path)
        .and_then(|m| {
            wowlua_manifest::load(&mut db, &m, &wowlua_driver::compiler_version()).manifest
        })
        .map(|m| wowlua_driver::format::config(&m.manifest.fmt))
        .unwrap_or_default();
    let file = db.add(doc.path.clone(), doc.text.clone());
    let formatted = wowlua_fmt::format(doc.text.as_bytes(), file, &config).ok()?;
    let formatted = String::from_utf8(formatted).ok()?;
    if formatted == doc.text {
        return Some(Vec::new());
    }
    Some(vec![json!({
        "range": text::range(&doc.text, 0, doc.text.len()),
        "newText": formatted,
    })])
}
