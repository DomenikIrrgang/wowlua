//! The language server inside another program, such as a web page's
//! WebAssembly (spec M14 S2, decision T395): the program hands it the
//! editor's messages and gets the server's back, and runs the checks that
//! are due when [`Embedded::wait`] says, on its own thread. The clock is
//! the program's ([`Embedded::set_time`]); checks keep their caches
//! between runs in memory. The program's own changes to the files (its
//! host) reach the server as an editor's report of files changed on disk
//! ([`Embedded::files_changed`]).

use crate::{Failure, Job, Server, check, on_checked, on_message, start_due, transport};
use serde_json::Value as Json;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

/// A language server driven by its program.
pub struct Embedded {
    server: Server,
    jobs: Rc<RefCell<VecDeque<Job>>>,
    now: Rc<Cell<Duration>>,
    warm: check::Warm,
    exited: bool,
    /// The editor said `initialize`.
    started: bool,
}

/// How the program changed a file ([`Embedded::files_changed`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Created,
    Changed,
    Deleted,
}

impl Default for Embedded {
    fn default() -> Embedded {
        Embedded::new()
    }
}

impl Embedded {
    pub fn new() -> Embedded {
        let jobs: Rc<RefCell<VecDeque<Job>>> = Rc::default();
        let now: Rc<Cell<Duration>> = Rc::default();
        let queue = jobs.clone();
        let clock = now.clone();
        Embedded {
            server: Server::new(
                true,
                Box::new(move |job| {
                    queue.borrow_mut().push_back(job);
                    true
                }),
                Box::new(move || clock.get()),
            ),
            jobs,
            now,
            warm: check::Warm::default(),
            exited: false,
            started: false,
        }
    }

    /// Keeps each package's checked modules on the disk too
    /// (`$WOWLUA_HOME/cache/check/`, as `wowlua lsp --worker` does): a
    /// program that starts the server again checks unchanged modules
    /// from there (T505).
    pub fn keep_on_disk(&mut self) {
        self.warm.store = Some(check::Store::Home);
    }

    /// The time on the program's clock, from any start that stays put (a
    /// page's `performance.now()`); it must not go back.
    pub fn set_time(&mut self, now: Duration) {
        self.now.set(now.max(self.now.get()));
    }

    /// Takes one of the editor's messages (a JSON-RPC object); returns the
    /// server's answers and notifications.
    pub fn message(&mut self, message: &Json) -> Vec<Json> {
        if message["method"] == "initialize" {
            self.started = true;
        }
        let mut out = Vec::new();
        if let Ok(true) = on_message(&mut self.server, message, &mut out) {
            self.exited = true;
        }
        start_due(&mut self.server, &mut out);
        messages(&out)
    }

    /// Files the program changed itself (created, changed, deleted: a
    /// folder deleted is one change), as `workspace/didChangeWatchedFiles`
    /// reports files changed on disk: their packages are checked again,
    /// those that depend on them too. A document the editor has open keeps
    /// the editor's text. Nothing before the editor's `initialize`.
    pub fn files_changed(&mut self, changes: &[(&Path, Change)]) {
        if !self.started || self.exited || changes.is_empty() {
            return;
        }
        let changes: Vec<Json> = changes
            .iter()
            .map(|(path, change)| {
                let kind = match change {
                    Change::Created => 1,
                    Change::Changed => 2,
                    Change::Deleted => 3,
                };
                serde_json::json!({ "uri": crate::text::uri_of(path), "type": kind })
            })
            .collect();
        // (a notification: the server answers nothing)
        self.message(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "workspace/didChangeWatchedFiles",
            "params": { "changes": changes },
        }));
    }

    /// How long until [`run_due`](Self::run_due) has a check to run; `None`
    /// when no check waits.
    pub fn wait(&self) -> Option<Duration> {
        if !self.jobs.borrow().is_empty() {
            return Some(Duration::ZERO);
        }
        let s = &self.server;
        if s.running.is_none() && (s.completing.is_some() || !s.finds.is_empty()) {
            return Some(Duration::ZERO);
        }
        (!s.dirty.is_empty() || !s.later.is_empty()).then(|| s.wait())
    }

    /// Runs the checks that are due now, one after the other; returns what
    /// they told the editor (diagnostics, answers that waited for them).
    pub fn run_due(&mut self) -> Vec<Json> {
        let mut all = Vec::new();
        self.run_due_with(|m| all.push(m));
        all
    }

    /// [`run_due`](Self::run_due), handing each message to `tell` as soon
    /// as it is made: a check's `wowlua/checking` beginning before the
    /// check runs.
    pub fn run_due_with(&mut self, mut tell: impl FnMut(Json)) {
        loop {
            let mut out = Vec::new();
            start_due(&mut self.server, &mut out);
            messages(&out).into_iter().for_each(&mut tell);
            let Some(job) = self.jobs.borrow_mut().pop_front() else {
                break;
            };
            let result: Result<Vec<check::Checked>, Failure> =
                crate::check_in_process(&job, Some(&mut self.warm), |path, text| {
                    // (also those the page's package calls took out of the way)
                    wowlua_driver::overlay::set(path, text);
                });
            let mut out = Vec::new();
            let failed = on_checked(&mut self.server, &job.manifest, result, &mut out).is_err();
            messages(&out).into_iter().for_each(&mut tell);
            if failed {
                break;
            }
        }
    }

    /// Whether the editor said `exit`.
    pub fn exited(&self) -> bool {
        self.exited
    }
}

/// The messages written to `out`, framed as on a pipe.
fn messages(out: &[u8]) -> Vec<Json> {
    let mut input = std::io::Cursor::new(out);
    let mut found = Vec::new();
    while let Ok(Some(m)) = transport::read(&mut input) {
        found.push(m);
    }
    found
}
