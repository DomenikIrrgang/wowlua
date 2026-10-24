//! A client for the tests: the server runs on a thread of its own, and
//! what it writes arrives message by message, as an editor reads it.

#![allow(dead_code)]

use serde_json::{Value as Json, json};
use std::sync::mpsc;
use std::time::Duration;

/// Collects the server's output and hands on each whole message.
struct Frames {
    buf: Vec<u8>,
    tx: mpsc::Sender<Json>,
}

impl std::io::Write for Frames {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        loop {
            let mut input = std::io::Cursor::new(&self.buf[..]);
            match wowlua_lsp::transport::read(&mut input) {
                Ok(Some(m)) => {
                    let used = input.position() as usize;
                    self.buf.drain(..used);
                    let _ = self.tx.send(m);
                }
                _ => return Ok(()),
            }
        }
    }
}

pub struct Client {
    to: mpsc::Sender<Json>,
    from: mpsc::Receiver<Json>,
    server: std::thread::JoinHandle<bool>,
    /// What the server wrote that no wait took yet.
    pub seen: Vec<Json>,
    next_id: u64,
}

impl Client {
    pub fn start() -> Client {
        Client::start_with(wowlua_lsp::Checker::InProcess)
    }

    pub fn start_with(checker: wowlua_lsp::Checker) -> Client {
        let (to, inbox) = mpsc::channel();
        let (tx, from) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let mut out = Frames {
                buf: Vec::new(),
                tx,
            };
            wowlua_lsp::serve_with(inbox, &mut out, checker).expect("the server runs")
        });
        let mut c = Client {
            to,
            from,
            server,
            seen: Vec::new(),
            next_id: 1,
        };
        c.request("initialize", json!({ "capabilities": {} }));
        c.notify("initialized", json!({}));
        c
    }

    pub fn notify(&self, method: &str, params: Json) {
        self.to
            .send(json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .unwrap();
    }

    /// A request's reply.
    pub fn request(&mut self, method: &str, params: Json) -> Json {
        let id = self.next_id;
        self.next_id += 1;
        self.to
            .send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .unwrap();
        self.wait(&format!("the reply to {method}"), |m| {
            m["id"] == id && m.get("method").is_none()
        })
    }

    /// The first message (written already or coming) that `f` takes.
    pub fn wait(&mut self, what: &str, f: impl Fn(&Json) -> bool) -> Json {
        if let Some(i) = self.seen.iter().position(&f) {
            return self.seen.remove(i);
        }
        loop {
            match self.from.recv_timeout(Duration::from_secs(120)) {
                Ok(m) if f(&m) => return m,
                Ok(m) => self.seen.push(m),
                Err(_) => panic!("no {what}; seen: {:#?}", self.seen),
            }
        }
    }

    /// The next diagnostics for `uri` that `f` takes.
    pub fn diagnostics(&mut self, uri: &str, f: impl Fn(&[Json]) -> bool) -> Vec<Json> {
        let m = self.wait(&format!("diagnostics for {uri}"), |m| {
            m["method"] == "textDocument/publishDiagnostics"
                && m["params"]["uri"] == uri
                && f(m["params"]["diagnostics"].as_array().unwrap())
        });
        m["params"]["diagnostics"].as_array().unwrap().clone()
    }

    /// `shutdown`, `exit`: whether the server stopped cleanly.
    pub fn finish(mut self) -> bool {
        self.request("shutdown", Json::Null);
        self.notify("exit", Json::Null);
        self.server.join().expect("the server stops")
    }
}
