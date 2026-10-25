//! The language server inside another program, such as a web page's
//! WebAssembly (spec M14 S2, decision T395): the program hands it the
//! editor's messages and gets the server's back, and runs the checks that
//! are due when [`Embedded::wait`] says, on its own thread. The clock is
//! the program's ([`Embedded::set_time`]); checks keep their caches
//! between runs in memory.

use crate::{Failure, Job, Server, check, on_checked, on_message, start_due, transport};
use serde_json::Value as Json;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

/// A language server driven by its program.
pub struct Embedded {
    server: Server,
    jobs: Rc<RefCell<VecDeque<Job>>>,
    now: Rc<Cell<Duration>>,
    warm: check::Warm,
    exited: bool,
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
        }
    }

    /// The time on the program's clock, from any start that stays put (a
    /// page's `performance.now()`); it must not go back.
    pub fn set_time(&mut self, now: Duration) {
        self.now.set(now.max(self.now.get()));
    }

    /// Takes one of the editor's messages (a JSON-RPC object); returns the
    /// server's answers and notifications.
    pub fn message(&mut self, message: &Json) -> Vec<Json> {
        let mut out = Vec::new();
        if let Ok(true) = on_message(&mut self.server, message, &mut out) {
            self.exited = true;
        }
        start_due(&mut self.server);
        messages(&out)
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
        let mut out = Vec::new();
        loop {
            start_due(&mut self.server);
            let Some(job) = self.jobs.borrow_mut().pop_front() else {
                break;
            };
            let result: Result<Vec<check::Checked>, Failure> =
                crate::check_in_process(&job, Some(&mut self.warm));
            if on_checked(&mut self.server, &job.manifest, result, &mut out).is_err() {
                break;
            }
        }
        messages(&out)
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
