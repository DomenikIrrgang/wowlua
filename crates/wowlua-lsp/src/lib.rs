//! `wowlua lsp`: the language server (spec §16.2) over stdio. It shares the
//! compiler: a package is checked as `wowlua check` checks it, with the
//! editor's unsaved texts over the disk's, a moment after the last edit
//! and on every save, on a thread of its own; documents are formatted as
//! `wowlua fmt` formats them, outlined from their syntax, and their names
//! lead to their definitions from the last check. Completion checks the
//! text with a placeholder at the cursor (`complete`); references and
//! rename check the whole workspace (`refs`).

// (tests set up their files on the disk directly)
#![cfg_attr(test, allow(clippy::disallowed_methods))]

pub mod check;
pub mod complete;
pub mod editor;
pub mod embedded;
pub mod refs;
pub mod symbols;
pub mod testing;
pub mod text;
pub mod transport;

use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// A moment on the server's clock: the time since it started. The clock is
/// the system's natively; a page that embeds the server sets it
/// ([`embedded`]), as the browser has no clock `std` can read.
type Moment = Duration;

/// How long after the last edit a package is checked again.
const SETTLE: Duration = Duration::from_millis(100);

/// How long without edits before a save's packages that depend on the
/// saved one are checked.
const QUIET: Duration = Duration::from_secs(2);

/// An open document.
struct Doc {
    path: PathBuf,
    /// `path` canonical, as checks name files.
    canonical: PathBuf,
    /// The package's manifest, if the file is in one.
    manifest: Option<PathBuf>,
    text: String,
}

/// What the server's loop takes in turn: the editor's messages and the
/// checks the worker finished.
enum Event {
    Message(Json),
    Checked(PathBuf, Result<Vec<check::Checked>, Failure>),
    /// The editor's side closed.
    Closed,
}

struct Server {
    docs: HashMap<String, Doc>,
    /// Packages (by manifest) edited since their last check, and since
    /// when.
    dirty: BTreeMap<PathBuf, Moment>,
    /// Packages whose full check waits until no package was edited for
    /// `QUIET`: since when, and whether with their whole workspace (after a
    /// save or a change on disk).
    later: BTreeMap<PathBuf, (Moment, bool)>,
    /// The check the worker runs now.
    running: Option<Running>,
    /// Checks for the worker.
    jobs: Box<dyn FnMut(Job) -> bool>,
    /// The time now.
    clock: Box<dyn Fn() -> Moment>,
    /// Go to definition requests waiting for their package's check: the
    /// package, the request's id, the document and the position.
    waiting: Vec<(PathBuf, Json, String, Json, Ask)>,
    /// The files each package's last check reported (by the package's
    /// manifest), to clear them.
    published: HashMap<PathBuf, BTreeSet<PathBuf>>,
    /// What each file's names refer to, from its package's last check.
    definitions: HashMap<PathBuf, Vec<check::Definition>>,
    /// What each open file's names and expressions are, from its
    /// package's last check.
    hovers: HashMap<PathBuf, Vec<check::Hover>>,
    /// The files each package's last check gave definitions for.
    defined: HashMap<PathBuf, Vec<PathBuf>>,
    shutdown: bool,
    /// Checks run in this process (they read the overlay).
    in_process: bool,
    /// Files were created or deleted since the last check started.
    rescan: bool,
    /// The editor's workspace folders: canonical, and as the editor spells
    /// them (through a symbolic link, say).
    roots: Vec<(PathBuf, PathBuf)>,
    /// The completion request to check next (only the latest: the editor
    /// asks again as the word grows).
    completing: Option<Completing>,
    /// The last completion's items, for `completionItem/resolve`.
    completed: Vec<check::Completion>,
    /// References and rename requests waiting for their check, in order.
    finds: std::collections::VecDeque<Finding>,
    /// The load failure each package's last check told the editor of.
    told: HashMap<PathBuf, String>,
}

/// A references or rename request: its id, what it wants, its package and
/// what its check looks for.
struct Finding {
    id: Json,
    want: Seek,
    manifest: PathBuf,
    find: refs::Find,
}

/// What a [`Finding`] answers.
#[derive(Clone, Copy, Debug)]
enum Seek {
    /// The references, with the declarations or without.
    References {
        declarations: bool,
    },
    Rename,
}

/// A completion check to run: the requests waiting for it (a completion
/// list, signature help: one check answers those of the same text and
/// cursor), the document and package, and what to check.
struct Completing {
    waiters: Vec<(Json, Want)>,
    uri: String,
    manifest: PathBuf,
    asked: complete::Asked,
}

/// A completion check running: the requests waiting for it, the word typed
/// and the document.
type CompletionRun = (Vec<(Json, Want)>, String, String);

/// What a request wants of a completion check.
enum Want {
    Completion,
    /// Signature help for the call around the cursor.
    Signature(complete::CallSite),
}

impl Want {
    /// The answer to a request whose check did not run (a newer one
    /// replaced it).
    fn nothing(&self) -> Json {
        match self {
            Want::Completion => json!({ "isIncomplete": true, "items": [] }),
            Want::Signature(_) => Json::Null,
        }
    }
}

/// Puts a request on the completion check to run: the one waiting, when it
/// is of the same text and cursor; else a new one (the requests of the one
/// waiting are answered empty: the editor asked again since).
fn wait_for_completion_check(
    server: &mut Server,
    out: &mut impl std::io::Write,
    request: (Json, Want),
    uri: String,
    manifest: PathBuf,
    asked: complete::Asked,
) -> std::io::Result<()> {
    if let Some(c) = server.completing.as_mut()
        && c.uri == uri
        && c.asked.text == asked.text
    {
        c.waiters.push(request);
        return Ok(());
    }
    let next = Completing {
        waiters: vec![request],
        uri,
        manifest,
        asked,
    };
    if let Some(old) = server.completing.replace(next) {
        for (id, want) in old.waiters {
            reply(out, &id, want.nothing())?;
        }
    }
    Ok(())
}

impl Server {
    /// The URI to report `path` (canonical) under: the editor's own for a
    /// document it has open.
    fn uri_for(&self, path: &Path) -> String {
        if let Some((uri, _)) = self.docs.iter().find(|(_, d)| d.canonical == path) {
            return uri.clone();
        }
        // a file below a workspace folder: in the editor's spelling
        for (canonical, given) in &self.roots {
            if let Ok(rest) = path.strip_prefix(canonical) {
                return text::uri_of(&given.join(rest));
            }
        }
        text::uri_of(path)
    }

    /// Marks a package edited: its check is due `SETTLE - early` from now
    /// (`early` = `SETTLE`: at once). A check of it running now is of an
    /// older text: its results are replaced by the next one's.
    fn touch(&mut self, manifest: &Path, early: Duration) {
        let now = (self.clock)();
        let at = now.saturating_sub(early);
        self.dirty.insert(manifest.to_path_buf(), at);
        // (its full check waits for quiet again)
        let entry = self
            .later
            .entry(manifest.to_path_buf())
            .or_insert((now, false));
        entry.0 = now;
    }

    /// Marks a package saved: checked at once, its workspace once quiet.
    fn saved(&mut self, manifest: &Path) {
        self.touch(manifest, SETTLE);
        if let Some(entry) = self.later.get_mut(manifest) {
            entry.1 = true;
        }
    }

    /// Whether a package has edits its last check did not see.
    fn pending(&self, manifest: &Path) -> bool {
        self.dirty.contains_key(manifest)
            || self
                .running
                .as_ref()
                .is_some_and(|r| r.manifest == manifest && r.focused)
    }

    /// The open documents of a package (canonical paths).
    fn open_in(&self, manifest: &Path) -> std::collections::HashSet<PathBuf> {
        self.docs
            .values()
            .filter(|d| d.manifest.as_deref() == Some(manifest))
            .map(|d| d.canonical.clone())
            .collect()
    }
}

fn canonical(path: &Path) -> PathBuf {
    text::canonical(path)
}

/// A panic (a compiler bug) is logged and caught where the server checks or
/// answers, never the end of the process as on the command line; a parallel
/// job's is recorded, as the driver expects.
fn catch_panics() {
    std::panic::set_hook(Box::new(|info| {
        if wowlua_driver::parallel::in_job() {
            wowlua_driver::parallel::record_panic(info.to_string());
            return;
        }
        eprintln!("wowlua lsp: internal compiler error: {info}");
    }));
}

/// Runs the server on stdin and stdout until the client says `exit`; true
/// when it said `shutdown` before (the protocol's exit code 0).
pub fn run() -> std::io::Result<bool> {
    catch_panics();
    let (tx, rx) = mpsc::channel::<Json>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut input = stdin.lock();
        loop {
            match transport::read(&mut input) {
                Ok(Some(message)) => {
                    if tx.send(message).is_err() {
                        break;
                    }
                }
                // a message that is no JSON: skipped, the next one read
                Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                    eprintln!("wowlua lsp: skipped a message: {e}");
                }
                Ok(None) | Err(_) => break,
            }
        }
    });
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let checker = match std::env::current_exe() {
        Ok(program) => Checker::Worker(program),
        Err(_) => Checker::InProcess,
    };
    serve_with(rx, &mut out, checker)
}

/// Where packages are checked.
#[derive(Clone, Debug)]
pub enum Checker {
    /// In this process (the crate's tests).
    InProcess,
    /// In a long-lived child process of this program (`wowlua lsp
    /// --worker`) that keeps each package's build cache between checks, so
    /// a check repeats only the work of what changed. A check that crashes,
    /// overflows its stack or hangs ends only the worker, which is started
    /// again; after `RECYCLE` checks it is started afresh, which gives its
    /// memory back to the system.
    Worker(PathBuf),
}

/// Checks a worker runs before it is replaced by a fresh one.
const RECYCLE: usize = 300;

/// How long a check may run before it is stopped.
const CHECK_LIMIT: Duration = Duration::from_secs(120);

/// [`serve_with`] checking in this process.
pub fn serve(
    messages: mpsc::Receiver<Json>,
    out: &mut impl std::io::Write,
) -> std::io::Result<bool> {
    serve_with(messages, out, Checker::InProcess)
}

/// The server's loop over `messages`, writing to `out`; true when the client
/// said `shutdown` before `exit`. Packages are checked on a thread of their
/// own, so requests are answered while one is checked.
pub fn serve_with(
    messages: mpsc::Receiver<Json>,
    out: &mut impl std::io::Write,
    checker: Checker,
) -> std::io::Result<bool> {
    let (events, inbox) = mpsc::channel::<Event>();
    let forward = events.clone();
    std::thread::spawn(move || {
        for m in messages {
            if forward.send(Event::Message(m)).is_err() {
                return;
            }
        }
        let _ = forward.send(Event::Closed);
    });
    let in_process = matches!(checker, Checker::InProcess);
    let (jobs, work) = mpsc::channel::<Job>();
    // (the stack the command line checks on: deep code overflows a smaller
    // one, which ends the process where no panic is caught)
    let worker = std::thread::Builder::new()
        .name("wowlua-lsp-check".into())
        .stack_size(wowlua_driver::parallel::STACK);
    worker.spawn(move || {
        let mut process: Option<WorkerProcess> = None;
        for job in work {
            let result = run_check(&checker, &mut process, &job);
            if events.send(Event::Checked(job.manifest, result)).is_err() {
                return;
            }
        }
    })?;
    #[allow(clippy::disallowed_methods)] // (the computer's loop: its clock)
    let started = Instant::now();
    let mut server = Server::new(
        in_process,
        Box::new(move |job| jobs.send(job).is_ok()),
        Box::new(move || started.elapsed()),
    );
    loop {
        match inbox.recv_timeout(server.wait()) {
            Ok(Event::Message(message)) => {
                if on_message(&mut server, &message, out)? {
                    return Ok(server.shutdown);
                }
            }
            Ok(Event::Checked(manifest, result)) => {
                on_checked(&mut server, &manifest, result, out)?
            }
            Ok(Event::Closed) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Ok(server.shutdown);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        start_due(&mut server, out);
    }
}

impl Server {
    fn new(
        in_process: bool,
        jobs: Box<dyn FnMut(Job) -> bool>,
        clock: Box<dyn Fn() -> Moment>,
    ) -> Server {
        Server {
            docs: HashMap::new(),
            dirty: BTreeMap::new(),
            later: BTreeMap::new(),
            running: None,
            jobs,
            clock,
            waiting: Vec::new(),
            published: HashMap::new(),
            definitions: HashMap::new(),
            hovers: HashMap::new(),
            defined: HashMap::new(),
            shutdown: false,
            in_process,
            rescan: false,
            roots: Vec::new(),
            completing: None,
            completed: Vec::new(),
            finds: std::collections::VecDeque::new(),
            told: HashMap::new(),
        }
    }

    /// How long until the next check is due: when its edits have settled,
    /// or a saved workspace's once nothing was edited for a while (an hour:
    /// none is).
    fn wait(&self) -> Duration {
        let now = (self.clock)();
        let none = Duration::from_secs(3600);
        match (&self.running, self.dirty.values().min()) {
            (Some(_), _) => none,
            (None, Some(at)) => (*at + SETTLE).saturating_sub(now),
            (None, None) => self
                .later
                .values()
                .map(|l| l.0)
                .min()
                .map_or(none, |at| (at + QUIET).saturating_sub(now)),
        }
    }
}

/// Handles one of the editor's messages; true at `exit`. A request whose
/// handler panicked still gets an answer.
fn on_message(
    server: &mut Server,
    message: &Json,
    out: &mut impl std::io::Write,
) -> std::io::Result<bool> {
    let handled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        handle(server, message, out)
    }));
    match handled {
        Ok(stop) => stop,
        Err(e) => {
            let why = panic_message(&*e);
            eprintln!("wowlua lsp: {} failed: {why}", message["method"]);
            if !message["id"].is_null() {
                transport::write(
                    out,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": message["id"],
                        "error": { "code": -32603, "message": why },
                    }),
                )?;
            }
            Ok(false)
        }
    }
}

/// Takes in a finished check: answers what waited for it and publishes
/// its diagnostics.
fn on_checked(
    server: &mut Server,
    manifest: &Path,
    result: Result<Vec<check::Checked>, Failure>,
    out: &mut impl std::io::Write,
) -> std::io::Result<()> {
    let mut ran = server.running.take();
    if ran.as_ref().is_some_and(|r| r.completion.is_none()) {
        checking(out, manifest, "end", None, result.is_err())?;
    }
    let finding = ran.as_mut().and_then(|r| r.finding.take());
    // a completion check: its answer, nothing published
    if let Some(Running {
        completion: Some((waiters, prefix, uri)),
        ..
    }) = ran
    {
        let checked = result.ok().and_then(|packages| packages.into_iter().next());
        for (id, want) in waiters {
            let answer = match want {
                Want::Completion => {
                    let found = checked.as_ref().and_then(|c| c.completions.clone());
                    completion_list(server, found, &prefix)
                }
                Want::Signature(site) => signature_help(server, checked.as_ref(), &site, &uri),
            };
            reply(out, &id, answer)?;
        }
        return Ok(());
    }
    match result {
        Ok(mut packages) => {
            let found = packages.first_mut().and_then(|c| c.found.take());
            for checked in packages {
                publish(server, checked, out)?;
            }
            if let Some(f) = finding {
                let new = f.find.rename.as_deref();
                let answer = found_answer(server, f.want, new, found);
                respond(out, &f.id, answer)?;
            }
        }
        Err(Failure::Failed(why)) => {
            if let Some(f) = finding {
                let failed = format!("wowlua could not check the workspace: {why}");
                respond(out, &f.id, Err(failed))?;
            }
            eprintln!("wowlua lsp: checking {} failed: {why}", manifest.display());
            notify(
                out,
                "window/logMessage",
                json!({ "type": 1, "message": format!("wowlua: checking {} failed: {why}", manifest.display()) }),
            )?;
        }
    }
    answer_waiting(server, manifest, out)
}

/// A check of a package began or ended (spec §16.2.9): the editor's
/// background jobs show it.
fn checking(
    out: &mut impl std::io::Write,
    manifest: &Path,
    state: &str,
    first: Option<bool>,
    failed: bool,
) -> std::io::Result<()> {
    let mut params = json!({ "state": state, "manifest": text::uri_of(manifest) });
    if let Some(first) = first {
        params["first"] = json!(first);
    }
    if failed {
        params["failed"] = json!(true);
    }
    notify(out, "wowlua/checking", params)
}

/// Starts the check that is due (see [`start_now`]), and tells the editor
/// when one began.
fn start_due(server: &mut Server, out: &mut impl std::io::Write) {
    let idle = server.running.is_none();
    start_now(server);
    if idle && let Some(r) = server.running.as_ref().filter(|r| r.completion.is_none()) {
        let _ = checking(out, &r.manifest.clone(), "begin", Some(r.focused), false);
    }
}

/// Starts the check of the package whose edits settled first, else, when
/// nothing was edited for `QUIET`, a saved package's workspace; unless a
/// check runs.
fn start_now(server: &mut Server) {
    if server.running.is_some() {
        return;
    }
    // a completion first: someone waits for it
    if let Some(c) = server.completing.take() {
        let canonical = server.docs.get(&c.uri).map(|d| d.canonical.clone());
        if let Some(canonical) = canonical {
            let docs: Vec<(PathBuf, String)> = server
                .docs
                .iter()
                .map(|(uri, d)| {
                    let text = if *uri == c.uri {
                        c.asked.text.clone()
                    } else {
                        d.text.clone()
                    };
                    (d.path.clone(), text)
                })
                .collect();
            let job = Job {
                manifest: c.manifest.clone(),
                whole: false,
                focus: Some(std::iter::once(canonical).collect()),
                docs,
                rescan: false,
                complete: Some(c.asked.prefix.clone()),
                find: None,
            };
            if (server.jobs)(job) {
                server.running = Some(Running {
                    manifest: c.manifest,
                    focused: false,
                    completion: Some((c.waiters, c.asked.prefix, c.uri)),
                    finding: None,
                });
            }
            return;
        }
    }
    // then references and renames: someone waits for them too (a whole
    // check of the workspace, with the texts as they are now)
    if let Some(f) = server.finds.pop_front() {
        let job = Job {
            manifest: f.manifest.clone(),
            whole: true,
            focus: None,
            docs: server
                .docs
                .values()
                .map(|d| (d.path.clone(), d.text.clone()))
                .collect(),
            rescan: std::mem::take(&mut server.rescan),
            complete: None,
            find: Some(f.find.clone()),
        };
        if (server.jobs)(job) {
            server.running = Some(Running {
                manifest: f.manifest.clone(),
                focused: false,
                completion: None,
                finding: Some(f),
            });
        }
        return;
    }
    let now = (server.clock)();
    let edited = server
        .dirty
        .iter()
        .filter(|(_, at)| now.saturating_sub(**at) >= SETTLE)
        .min_by_key(|(_, at)| **at)
        .map(|(m, _)| m.clone());
    // an edit: its open files (and what they import) first; quiet: the
    // full package, after a save its whole workspace
    let (manifest, whole, focus) = match edited {
        Some(m) => {
            server.dirty.remove(&m);
            let open = server.open_in(&m);
            let focus = (!open.is_empty()).then_some(open);
            (m, false, focus)
        }
        None if server.dirty.is_empty() => {
            let quiet = server
                .later
                .iter()
                .filter(|(_, (at, _))| now.saturating_sub(*at) >= QUIET)
                .min_by_key(|(_, (at, _))| *at)
                .map(|(m, _)| m.clone());
            match quiet {
                Some(m) => {
                    let (_, whole) = server.later.remove(&m).unwrap_or((now, false));
                    (m, whole, None)
                }
                None => return,
            }
        }
        None => return,
    };
    let focused = focus.is_some();
    {
        // (the open documents as they are now: what this check sees)
        let docs: Vec<(PathBuf, String)> = server
            .docs
            .values()
            .map(|d| (d.path.clone(), d.text.clone()))
            .collect();
        let job = Job {
            manifest: manifest.clone(),
            whole,
            focus,
            docs,
            rescan: std::mem::take(&mut server.rescan),
            complete: None,
            find: None,
        };
        if (server.jobs)(job) {
            server.running = Some(Running {
                manifest,
                focused,
                completion: None,
                finding: None,
            });
        }
    }
}

/// The check the worker runs.
struct Running {
    manifest: PathBuf,
    /// The first pass of an edit (its open files only).
    focused: bool,
    /// A completion check: the requests waiting for it, the word typed and
    /// the document.
    completion: Option<CompletionRun>,
    /// A references or rename check: the request it answers.
    finding: Option<Finding>,
}

/// Why a check gave no answer.
#[derive(Debug)]
enum Failure {
    /// It crashed, failed or took too long.
    Failed(String),
}

/// Whether a file of the package whose manifest is `manifest` is one no
/// check reads: what builds write (`dist/`) or in a hidden folder of the
/// package (`.git/`).
fn generated(manifest: &Path, path: &Path) -> bool {
    let root = manifest.parent().unwrap_or(Path::new(""));
    let inside = canonical(path);
    let Ok(rel) = inside.strip_prefix(root) else {
        return false;
    };
    rel.components().any(|c| {
        let c = c.as_os_str().to_string_lossy();
        c == "dist" || c.starts_with('.')
    })
}

/// A check for the worker: the package, whether its whole workspace, the
/// files a first pass checks, and the open documents.
struct Job {
    manifest: PathBuf,
    whole: bool,
    /// The files a first pass checks (and what they import).
    focus: Option<std::collections::HashSet<PathBuf>>,
    docs: Vec<(PathBuf, String)>,
    /// Files were created or deleted since the last check.
    rescan: bool,
    /// A completion check (`check::check`): the word typed.
    complete: Option<String>,
    /// A references or rename check (`check::find`).
    find: Option<refs::Find>,
}

/// Runs a check where `checker` says. A check that panics, crashes or
/// takes too long is an error the server reports, and goes on.
fn run_check(
    checker: &Checker,
    process: &mut Option<WorkerProcess>,
    job: &Job,
) -> Result<Vec<check::Checked>, Failure> {
    match checker {
        // (the server thread sets each file as it opens, and removes it as
        // it closes: one closed since the job was made must not come back)
        Checker::InProcess => check_in_process(job, None, |path, text| {
            wowlua_driver::overlay::set_open(path, text);
        }),
        Checker::Worker(program) => check_in_worker(program, process, job),
    }
}

/// Runs a check in this process, with what `warm` kept from earlier checks.
/// The open documents are the job's, a completion's text with its
/// placeholder, which `set` puts in the overlay; the next job sets them
/// again. Each is set alone: the crate's tests run servers side by side. A
/// check here runs to its end.
fn check_in_process(
    job: &Job,
    warm: Option<&mut check::Warm>,
    set: impl Fn(&Path, Vec<u8>),
) -> Result<Vec<check::Checked>, Failure> {
    for (path, text) in &job.docs {
        set(path, text.as_bytes().to_vec());
    }
    let open = job.docs.iter().map(|(p, _)| canonical(p)).collect();
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &job.find {
        Some(f) => check::find(&job.manifest, &open, f, warm, drop),
        None => check::check_warm(
            &job.manifest,
            job.whole,
            &open,
            job.focus.as_ref(),
            job.complete.as_deref(),
            warm,
        ),
    }))
    .map_err(|e| Failure::Failed(panic_message(&*e)))
}

/// The worker process: its input, its answers (one line each), the last
/// of what it wrote to stderr, and the checks it ran.
struct WorkerProcess {
    child: std::process::Child,
    input: std::process::ChildStdin,
    answers: mpsc::Receiver<String>,
    errors: std::sync::Arc<std::sync::Mutex<String>>,
    checks: usize,
}

impl WorkerProcess {
    fn start(program: &Path) -> Result<WorkerProcess, String> {
        use std::io::{BufRead, Read};
        use std::process::{Command, Stdio};
        let mut child = Command::new(program)
            .args(["lsp", "--worker"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", program.display()))?;
        let input = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let (tx, answers) = mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                if tx.send(line).is_err() {
                    return;
                }
            }
        });
        let errors = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let tail = errors.clone();
        let mut stderr = child.stderr.take().expect("piped");
        std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            while let Ok(n) = stderr.read(&mut chunk) {
                if n == 0 {
                    return;
                }
                let text = String::from_utf8_lossy(&chunk[..n]);
                // (shown in the editor's log as it comes; the last kept for
                // the reason of a crash)
                eprint!("{text}");
                let mut t = tail.lock().unwrap_or_else(|e| e.into_inner());
                t.push_str(&text);
                if t.len() > 16384 {
                    let cut = t.len() - 16384;
                    let cut = (cut..t.len()).find(|&i| t.is_char_boundary(i)).unwrap_or(0);
                    t.drain(..cut);
                }
            }
        });
        Ok(WorkerProcess {
            child,
            input,
            answers,
            errors,
            checks: 0,
        })
    }

    /// The last lines it wrote to stderr.
    fn last_errors(&self) -> String {
        let t = self.errors.lock().unwrap_or_else(|e| e.into_inner());
        let lines: Vec<&str> = t.lines().rev().take(20).collect();
        lines.into_iter().rev().collect::<Vec<_>>().join("\n")
    }
}

impl Drop for WorkerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Runs a check in the worker (started, or started again, as needed): the
/// request one line on its input, the answer one line on its output. A
/// check whose text an edit made old runs to its end: stopping it would
/// stop the worker and lose what it keeps.
fn check_in_worker(
    program: &Path,
    process: &mut Option<WorkerProcess>,
    job: &Job,
) -> Result<Vec<check::Checked>, Failure> {
    use std::io::Write;
    if process.as_ref().is_some_and(|p| p.checks >= RECYCLE) {
        *process = None;
    }
    if process.is_none() {
        *process = Some(WorkerProcess::start(program).map_err(Failure::Failed)?);
    }
    let worker = process.as_mut().expect("started");
    worker.checks += 1;
    let mut request = check::request(
        &job.manifest,
        job.whole,
        &job.docs,
        job.focus.as_ref(),
        job.rescan,
        job.complete.as_deref(),
    );
    if let Some(f) = &job.find {
        request["find"] = f.to_json();
    }
    let mut line = serde_json::to_vec(&request).unwrap_or_default();
    line.push(b'\n');
    if worker
        .input
        .write_all(&line)
        .and_then(|()| worker.input.flush())
        .is_err()
    {
        let why = worker.last_errors();
        *process = None;
        return Err(Failure::Failed(format!("the checker stopped: {why}")));
    }
    #[allow(clippy::disallowed_methods)] // (the computer's loop: its clock)
    let started = Instant::now();
    let answer = loop {
        match worker.answers.recv_timeout(Duration::from_millis(20)) {
            Ok(answer) => break answer,
            Err(mpsc::RecvTimeoutError::Timeout) if started.elapsed() > CHECK_LIMIT => {
                *process = None;
                return Err(Failure::Failed(format!(
                    "the check took longer than {} s and was stopped",
                    CHECK_LIMIT.as_secs()
                )));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let why = worker.last_errors();
                *process = None;
                return Err(Failure::Failed(format!("the checker stopped: {why}")));
            }
        }
    };
    let json: Json = serde_json::from_str(&answer)
        .map_err(|_| Failure::Failed("the check's answer could not be read".into()))?;
    if let Some(why) = json["error"].as_str() {
        return Err(Failure::Failed(why.to_string()));
    }
    check::answer(&json)
        .ok_or_else(|| Failure::Failed("the check's answer could not be read".into()))
}

/// The worker (`wowlua lsp --worker`): a request per line on stdin, its
/// answer a line on stdout, keeping each package's build cache between
/// checks. A check that panics answers with the error, and what was kept
/// is dropped.
pub fn run_worker() -> std::io::Result<()> {
    use std::io::{BufRead, Write};
    catch_panics();
    // (folders are walked again only when what is in them changed)
    wowlua_driver::remember_walks();
    // (the packages' modules kept on disk: a new server's first check
    // takes the unchanged ones, spec compiler §13.9 Phase 2)
    let fresh = || check::Warm::persistent(check::Store::Home);
    let mut warm = fresh();
    let stdout = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        let answer = match serde_json::from_str::<Json>(&line) {
            Ok(request) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                check::worker(&mut warm, &request)
            }))
            .unwrap_or_else(|e| {
                warm = fresh();
                json!({ "error": panic_message(&*e) })
            }),
            Err(e) => json!({ "error": format!("a request that is no JSON: {e}") }),
        };
        let mut out = stdout.lock();
        serde_json::to_writer(&mut out, &answer)?;
        out.write_all(b"\n")?;
        out.flush()?;
    }
    Ok(())
}

/// What a panic said.
fn panic_message(e: &(dyn std::any::Any + Send)) -> String {
    e.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| e.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic".into())
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

/// The manifest of the package a file belongs to (canonical, as checks
/// name packages).
fn manifest_of(path: &Path) -> Option<PathBuf> {
    wowlua_driver::project::find_manifest(path.parent()?).map(|m| canonical(&m))
}

/// The reply to `initialize`: what the server can do.
fn initialized(out: &mut impl std::io::Write, id: &Json) -> std::io::Result<()> {
    reply(
        out,
        id,
        json!({
            "capabilities": {
                "textDocumentSync": { "openClose": true, "change": 2, "save": { "includeText": false } },
                "documentFormattingProvider": true,
                "documentSymbolProvider": true,
                "definitionProvider": true,
                "referencesProvider": true,
                "renameProvider": { "prepareProvider": true },
                "hoverProvider": true,
                "completionProvider": { "triggerCharacters": [".", ":", "\"", "'", "@"], "resolveProvider": true },
                "signatureHelpProvider": { "triggerCharacters": ["(", ","], "retriggerCharacters": [","] },
                "foldingRangeProvider": editor::capabilities()["foldingRangeProvider"],
                "semanticTokensProvider": editor::capabilities()["semanticTokensProvider"],
                "inlayHintProvider": editor::capabilities()["inlayHintProvider"],
                "codeActionProvider": editor::capabilities()["codeActionProvider"],
                "codeLensProvider": editor::capabilities()["codeLensProvider"],
            },
            "serverInfo": { "name": "wowlua", "version": env!("CARGO_PKG_VERSION") },
        }),
    )
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
        "initialize" => {
            let folders = params["workspaceFolders"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|f| &f["uri"])
                .chain(std::iter::once(&params["rootUri"]));
            for uri in folders {
                if let Some(given) = uri.as_str().and_then(text::path_of) {
                    let pair = (canonical(&given), given);
                    if !server.roots.contains(&pair) {
                        server.roots.push(pair);
                    }
                }
            }
            initialized(out, id)?;
        }
        "shutdown" => {
            server.shutdown = true;
            reply(out, id, Json::Null)?;
        }
        "exit" => return Ok(true),
        "textDocument/didOpen" | "textDocument/didChange" => {
            let text = if method == "textDocument/didOpen" {
                params["textDocument"]["text"].as_str().map(str::to_string)
            } else {
                // incremental sync: each change replaces its range of the
                // text before it (without a range, the whole text)
                let mut text = server
                    .docs
                    .get_mut(&uri)
                    .map(|d| std::mem::take(&mut d.text))
                    .unwrap_or_default();
                for change in params["contentChanges"].as_array().into_iter().flatten() {
                    let new = change["text"].as_str().unwrap_or_default();
                    match change.get("range").filter(|r| r.is_object()) {
                        Some(range) => {
                            let start = text::offset(&text, &range["start"]);
                            let end = text::offset(&text, &range["end"]).max(start);
                            text.replace_range(start..end, new);
                        }
                        None => text = new.to_string(),
                    }
                }
                Some(text)
            };
            if let (Some(text), Some(path)) = (text, text::path_of(&uri)) {
                // (a check in this process reads the overlay; a child gets
                // the open documents with its request)
                if server.in_process {
                    wowlua_driver::overlay::set(&path, text.as_bytes().to_vec());
                }
                // (an open document keeps the package it was opened in)
                let manifest = match server.docs.get(&uri) {
                    Some(d) => d.manifest.clone(),
                    None => manifest_of(&path),
                };
                if let Some(m) = &manifest {
                    server.touch(m, Duration::ZERO);
                }
                server.docs.insert(
                    uri,
                    Doc {
                        canonical: canonical(&path),
                        path,
                        manifest,
                        text,
                    },
                );
            }
        }
        "textDocument/didSave" => {
            let manifest = match server.docs.get(&uri) {
                Some(d) => d.manifest.clone(),
                None => text::path_of(&uri).and_then(|p| manifest_of(&p)),
            };
            if let Some(m) = manifest {
                // a save is checked at once, the packages that depend on it
                // once edits pause
                server.saved(&m);
            }
        }
        "textDocument/didClose" => {
            if let Some(doc) = server.docs.remove(&uri) {
                wowlua_driver::overlay::remove(&doc.path);
                if let Some(m) = &doc.manifest {
                    server.touch(m, Duration::ZERO);
                }
            }
        }
        // files changed on disk (a branch checked out, a formatter run):
        // their packages, with those that depend on them, again
        "workspace/didChangeWatchedFiles" => {
            for change in params["changes"].as_array().into_iter().flatten() {
                // (created or deleted: the files to check are found anew)
                if change["type"] != 2 {
                    server.rescan = true;
                }
                let manifest = change["uri"]
                    .as_str()
                    .and_then(text::path_of)
                    .and_then(|p| Some((manifest_of(&p)?, p)))
                    .filter(|(m, p)| !generated(m, p))
                    .map(|(m, _)| m);
                if let Some(m) = manifest {
                    server.saved(&m);
                }
            }
        }
        "textDocument/formatting" => {
            let edits = server.docs.get(&uri).and_then(format).unwrap_or_default();
            reply(out, id, Json::Array(edits))?;
        }
        "textDocument/definition" | "textDocument/hover" => {
            let ask = if method == "textDocument/hover" {
                Ask::Hover
            } else {
                Ask::Definition
            };
            let manifest = server.docs.get(&uri).and_then(|d| d.manifest.clone());
            match manifest {
                // edits not checked yet: the answer waits for the check,
                // which starts now
                Some(m) if server.pending(&m) => {
                    if server.dirty.contains_key(&m) {
                        server.touch(&m, SETTLE);
                    }
                    server
                        .waiting
                        .push((m, id.clone(), uri, params["position"].clone(), ask));
                }
                _ => reply(out, id, ask.answer(server, &uri, &params["position"]))?,
            }
        }
        // a whole check of the workspace finds them, with the texts as they
        // are now
        "textDocument/references" | "textDocument/rename" => {
            let want = if method == "textDocument/rename" {
                Seek::Rename
            } else {
                Seek::References {
                    declarations: params["context"]["includeDeclaration"] != false,
                }
            };
            let new = params["newName"].as_str().map(str::to_string);
            let refused = match (want, &new) {
                (Seek::Rename, Some(n)) => refs::invalid_name(n),
                (Seek::Rename, None) => Some("no new name".to_string()),
                _ => None,
            };
            let doc = server.docs.get(&uri);
            let asked = doc.and_then(|d| {
                let offset = text::offset(&d.text, &params["position"]) as u32;
                Some((d.manifest.clone()?, d.canonical.clone(), offset))
            });
            match (refused, asked) {
                (Some(why), _) => respond(out, id, Err(why))?,
                (None, None) => respond(out, id, found_answer(server, want, None, None))?,
                (None, Some((manifest, path, offset))) => server.finds.push_back(Finding {
                    id: id.clone(),
                    want,
                    manifest,
                    find: refs::Find {
                        path,
                        offset,
                        rename: new.filter(|_| matches!(want, Seek::Rename)),
                    },
                }),
            }
        }
        "textDocument/prepareRename" => {
            let answer = prepare_rename(server, &uri, &params["position"]);
            respond(out, id, answer)?;
        }
        "textDocument/completion" => {
            let asked = server.docs.get(&uri).and_then(|d| {
                let offset = text::offset(&d.text, &params["position"]);
                Some((d.manifest.clone()?, complete::prepare(&d.text, offset)?))
            });
            match asked {
                None => reply(out, id, completion_list(server, None, ""))?,
                Some((manifest, asked)) => wait_for_completion_check(
                    server,
                    out,
                    (id.clone(), Want::Completion),
                    uri,
                    manifest,
                    asked,
                )?,
            }
        }
        // the parameters of the call the cursor is in, the one it is at
        "textDocument/signatureHelp" => {
            let asked = server.docs.get(&uri).and_then(|d| {
                let offset = text::offset(&d.text, &params["position"]);
                let site = complete::call_at(&d.text, offset)?;
                Some((
                    d.manifest.clone()?,
                    site,
                    complete::prepare(&d.text, offset)?,
                ))
            });
            match asked {
                None => reply(out, id, Json::Null)?,
                Some((manifest, site, asked)) => wait_for_completion_check(
                    server,
                    out,
                    (id.clone(), Want::Signature(site)),
                    uri,
                    manifest,
                    asked,
                )?,
            }
        }
        // a completion's type and doc comment, once the editor shows it
        "completionItem/resolve" => {
            let mut item = params.clone();
            let found = params["data"]
                .as_u64()
                .and_then(|i| server.completed.get(i as usize))
                .filter(|c| params["label"] == c.label.as_str())
                .cloned();
            if let Some(c) = found {
                if !c.detail.is_empty() {
                    item["detail"] = json!(c.detail);
                }
                if let Some((uri, line)) = &c.declared
                    && let Some(comment) = doc_comment(server, uri, *line as usize)
                {
                    item["documentation"] = json!({ "kind": "markdown", "value": comment });
                }
            }
            reply(out, id, item)?;
        }
        // the test explorer: a test file's tests (its text as open, else on
        // disk)
        "wowlua/tests" => {
            let found = match server.docs.get(&uri) {
                Some(d) => testing::tests(&d.text),
                None => text::path_of(&uri)
                    .and_then(|p| wowlua_driver::host::read_to_string(p).ok())
                    .map(|t| testing::tests(&t))
                    .unwrap_or_default(),
            };
            reply(out, id, Json::Array(found))?;
        }
        "textDocument/documentSymbol" => {
            let symbols = server
                .docs
                .get(&uri)
                .map(|d| symbols::symbols(&d.text))
                .unwrap_or_default();
            reply(out, id, Json::Array(symbols))?;
        }
        "textDocument/foldingRange" => {
            let ranges = server
                .docs
                .get(&uri)
                .map(|d| editor::folding_ranges(&d.text))
                .unwrap_or_default();
            reply(out, id, Json::Array(ranges))?;
        }
        "textDocument/semanticTokens/full" => {
            let data = server
                .docs
                .get(&uri)
                .map(|d| editor::semantic_tokens(&d.text))
                .unwrap_or_default();
            reply(out, id, json!({ "data": data }))?;
        }
        "textDocument/inlayHint" => {
            let line = |p: &Json| p["line"].as_u64().map(|l| l as usize);
            let first = line(&params["range"]["start"]).unwrap_or(0);
            let last = line(&params["range"]["end"]).unwrap_or(usize::MAX);
            let hints = server
                .docs
                .get(&uri)
                .map(|d| {
                    let hovers = server
                        .hovers
                        .get(&d.canonical)
                        .map_or(&[][..], Vec::as_slice);
                    editor::inlay_hints(&d.text, hovers, first, last)
                })
                .unwrap_or_default();
            reply(out, id, Json::Array(hints))?;
        }
        "textDocument/codeAction" => {
            let diagnostics = params["context"]["diagnostics"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            reply(out, id, Json::Array(editor::code_actions(&diagnostics)))?;
        }
        "textDocument/codeLens" => {
            let lenses = server
                .docs
                .get(&uri)
                .map(|d| {
                    // Definitions name files by their canonical URI.
                    let target = text::uri_of(&d.canonical);
                    editor::reference_lenses(
                        &d.text,
                        &uri,
                        &target,
                        server.definitions.values().flatten(),
                    )
                })
                .unwrap_or_default();
            reply(out, id, Json::Array(lenses))?;
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

/// A package's check to the client: its diagnostics (its files without any
/// and its open documents without any cleared), its definitions kept.
fn publish(
    server: &mut Server,
    checked: check::Checked,
    out: &mut impl std::io::Write,
) -> std::io::Result<()> {
    let manifest = checked.manifest.clone();
    let manifest = manifest.as_path();
    // a package that can't be loaded is checked no further: the editor
    // shows why, once until it changes or is fixed
    match &checked.unloaded {
        Some(why) if server.told.get(manifest) != Some(why) => {
            notify(
                out,
                "window/showMessage",
                json!({ "type": 1, "message": why }),
            )?;
            server.told.insert(manifest.to_path_buf(), why.clone());
        }
        Some(_) => {}
        None => {
            server.told.remove(manifest);
        }
    }
    // (a focused check speaks for its files; a full one for the package)
    let covers = |path: &Path| checked.scope.as_ref().is_none_or(|s| s.contains(path));
    let now: BTreeSet<PathBuf> = checked.diagnostics.keys().cloned().collect();
    for (path, items) in &checked.diagnostics {
        notify(
            out,
            "textDocument/publishDiagnostics",
            json!({ "uri": server.uri_for(path), "diagnostics": items }),
        )?;
    }
    let before = server.published.remove(manifest).unwrap_or_default();
    let mut clean: BTreeSet<PathBuf> = before
        .iter()
        .filter(|p| covers(p) && !now.contains(*p))
        .cloned()
        .collect();
    // the open documents the check found nothing in: they are checked
    clean.extend(
        server
            .docs
            .values()
            .filter(|d| {
                d.manifest.as_deref() == Some(manifest)
                    && covers(&d.canonical)
                    && !now.contains(&d.canonical)
            })
            .map(|d| d.canonical.clone()),
    );
    for path in &clean {
        notify(
            out,
            "textDocument/publishDiagnostics",
            json!({ "uri": server.uri_for(path), "diagnostics": [] }),
        )?;
    }
    // what it did not cover stays as the last check found it
    let mut kept: BTreeSet<PathBuf> = before.into_iter().filter(|p| !covers(p)).collect();
    kept.extend(now);
    server.published.insert(manifest.to_path_buf(), kept);
    let mut files: Vec<PathBuf> = server.defined.remove(manifest).unwrap_or_default();
    files.retain(|f| {
        let replaced = covers(f);
        if replaced {
            server.definitions.remove(f);
            server.hovers.remove(f);
        }
        !replaced
    });
    files.extend(checked.definitions.keys().cloned());
    files.extend(checked.hovers.keys().cloned());
    files.sort();
    files.dedup();
    server.defined.insert(manifest.to_path_buf(), files);
    server.definitions.extend(checked.definitions);
    server.hovers.extend(checked.hovers);
    Ok(())
}

/// Answers the go to definition requests that waited for a package's check.
fn answer_waiting(
    server: &mut Server,
    manifest: &Path,
    out: &mut impl std::io::Write,
) -> std::io::Result<()> {
    let (ready, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut server.waiting)
        .into_iter()
        .partition(|(m, ..)| m == manifest);
    server.waiting = rest;
    for (_, id, uri, position, ask) in ready {
        let found = ask.answer(server, &uri, &position);
        reply(out, &id, found)?;
    }
    Ok(())
}

/// A request answered from a package's last check.
#[derive(Clone, Copy, Debug)]
enum Ask {
    Definition,
    Hover,
}

impl Ask {
    fn answer(self, server: &Server, uri: &str, position: &Json) -> Json {
        match self {
            Ask::Definition => definition(server, uri, position),
            Ask::Hover => hover(server, uri, position),
        }
    }
}

/// What the name or expression at `position` is (the innermost one there):
/// its type, as `name: type` for a name, and the doc comment of what it
/// refers to (or declares); `null` where the last check knows none.
fn hover(server: &Server, uri: &str, position: &Json) -> Json {
    let Some(doc) = server.docs.get(uri) else {
        return Json::Null;
    };
    let offset = text::offset(&doc.text, position);
    let Some(found) = server
        .hovers
        .get(&doc.canonical)
        .into_iter()
        .flatten()
        .filter(|h| h.start as usize <= offset && offset <= h.end as usize)
        .min_by_key(|h| h.end - h.start)
    else {
        return Json::Null;
    };
    let (start, end) = (found.start as usize, found.end as usize);
    // a name, or the field or method at the end of an expression
    // (`self.count`) when the pointer is on it
    let name = doc.text.get(start..end).and_then(|s| {
        let tail = s.len()
            - s.chars()
                .rev()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .map(char::len_utf8)
                .sum::<usize>();
        let n = &s[tail..];
        let named = !n.is_empty()
            && n.len() <= 80
            && !n.starts_with(|c: char| c.is_ascii_digit())
            && (tail == 0 || offset >= start + tail);
        named.then_some(n)
    });
    let mut value = match name {
        Some(n) => format!("```wowlua\n{n}: {}\n```", found.ty),
        None => format!("```wowlua\n{}\n```", found.ty),
    };
    // the doc comment of what it refers to, else of what it declares here
    let target = server
        .definitions
        .get(&doc.canonical)
        .into_iter()
        .flatten()
        .find(|d| d.start as usize <= offset && offset <= d.end as usize)
        .map(|d| (d.target.uri.to_string(), d.target.range[0] as usize));
    let (target_uri, line) = target.unwrap_or_else(|| {
        let line = doc.text[..start.min(doc.text.len())].matches('\n').count();
        (uri.to_string(), line)
    });
    if let Some(comment) = doc_comment(server, &target_uri, line) {
        value.push_str("\n\n");
        value.push_str(&comment);
    }
    json!({
        "contents": { "kind": "markdown", "value": value },
        "range": text::range(&doc.text, start, end),
    })
}

/// The keywords a statement or an expression may start with, completed
/// where a name is.
const KEYWORDS: &[&str] = &[
    "and",
    "break",
    "class",
    "const",
    "continue",
    "do",
    "else",
    "elseif",
    "end",
    "enum",
    "export",
    "false",
    "for",
    "function",
    "if",
    "import",
    "in",
    "interface",
    "local",
    "nil",
    "not",
    "or",
    "repeat",
    "return",
    "then",
    "true",
    "type",
    "until",
    "while",
];

/// A completion check's answer as the protocol's `CompletionList` (an
/// empty one without), keywords added where a name is. The items are kept
/// for `completionItem/resolve`: the game's (thousands, for a short
/// prefix) get their type there, every item its doc comment.
fn completion_list(server: &mut Server, found: Option<check::Completions>, prefix: &str) -> Json {
    let Some(found) = found else {
        return json!({ "isIncomplete": false, "items": [] });
    };
    let mut items: Vec<Json> = found
        .items
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut item = json!({ "label": c.label, "kind": c.kind, "data": i });
            if c.rank < 2 {
                item["sortText"] = json!(format!("{}{}", c.rank, c.label));
                if !c.detail.is_empty() {
                    item["detail"] = json!(c.detail);
                }
            }
            if let Some(doc) = &c.doc {
                item["documentation"] = json!({ "kind": "markdown", "value": doc });
            }
            item
        })
        .collect();
    server.completed = found.items;
    if found.context == "names" {
        items.extend(
            KEYWORDS
                .iter()
                .filter(|k| k.starts_with(prefix) || prefix.is_empty())
                .map(|k| json!({ "label": k, "kind": 14, "sortText": format!("3{k}") })),
        );
    }
    json!({ "isIncomplete": found.incomplete, "items": items })
}

/// Signature help for the call `site` names, from a completion check: each
/// form of the function (`name(a: T, b: U) -> R`, its parameters' places in
/// that label), its doc comment's summary and each parameter's line of it,
/// the form and parameter the cursor is at.
fn signature_help(
    server: &Server,
    checked: Option<&check::Checked>,
    site: &complete::CallSite,
    uri: &str,
) -> Json {
    let Some(forms) = checked
        .and_then(|c| c.signature.as_ref())
        .filter(|f| !f.is_empty())
    else {
        return Json::Null;
    };
    // the doc comment of what the name before the `(` refers to
    let comment = server.docs.get(uri).and_then(|doc| {
        checked?
            .definitions
            .get(&doc.canonical)?
            .iter()
            .find(|d| d.start as usize <= site.name_end && site.name_end <= d.end as usize)
            .and_then(|d| doc_comment(server, &d.target.uri, d.target.range[0] as usize))
    });
    let (summary, param_docs) = split_doc(comment.as_deref().unwrap_or(""));
    let utf16 = |s: &str| s.encode_utf16().count();
    let signatures: Vec<Json> = forms
        .iter()
        .map(|form| {
            let mut label = format!("{}(", site.name);
            let mut parameters = Vec::new();
            let parts = form.params.iter().chain(form.rest.iter());
            for (i, p) in parts.enumerate() {
                if i > 0 {
                    label.push_str(", ");
                }
                let start = utf16(&label);
                label.push_str(p);
                let mut param = json!({ "label": [start, utf16(&label)] });
                let name = p.split(':').next().unwrap_or("").trim();
                if let Some(doc) = param_docs.iter().find(|(n, _)| n == name).map(|(_, d)| d) {
                    param["documentation"] = json!({ "kind": "markdown", "value": doc });
                }
                parameters.push(param);
            }
            label.push(')');
            if form.results != "()" {
                label.push_str(&format!(" -> {}", form.results));
            }
            let mut sig = json!({ "label": label, "parameters": parameters });
            if !summary.is_empty() {
                sig["documentation"] = json!({ "kind": "markdown", "value": summary });
            }
            sig
        })
        .collect();
    // the first form with a parameter at the cursor's argument
    let fits = |f: &check::SignatureForm| site.active < f.params.len() || f.rest.is_some();
    let active_signature = forms.iter().position(fits).unwrap_or(0);
    let form = &forms[active_signature];
    // (past the parameters: the rest's, else none is highlighted)
    let active_parameter = if site.active < form.params.len() || form.rest.is_none() {
        site.active
    } else {
        form.params.len()
    };
    json!({
        "signatures": signatures,
        "activeSignature": active_signature,
        "activeParameter": active_parameter,
    })
}

/// A doc comment's summary (what comes before its **Parameters** or
/// **Returns**) and its parameters' lines (`- \`name\`: text`).
fn split_doc(comment: &str) -> (String, Vec<(String, String)>) {
    let summary = comment
        .split("\n\n**")
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let mut params = Vec::new();
    let mut in_params = false;
    for line in comment.lines() {
        let t = line.trim();
        if t.starts_with("**") {
            in_params = t == "**Parameters**";
            continue;
        }
        if !in_params {
            continue;
        }
        if let Some(rest) = t.strip_prefix("- `")
            && let Some((name, text)) = rest.split_once("`: ")
        {
            params.push((name.to_string(), text.to_string()));
        }
    }
    (summary, params)
}

/// The `---` comment lines right above line `line` of the file at `uri`
/// (decorators between them and it skipped), without their dashes.
fn doc_comment(server: &Server, uri: &str, line: usize) -> Option<String> {
    let owned;
    let text = match server.docs.get(uri) {
        Some(d) => d.text.as_str(),
        None => {
            owned = wowlua_driver::host::read_to_string(text::path_of(uri)?).ok()?;
            owned.as_str()
        }
    };
    let lines: Vec<&str> = text.lines().take(line).collect();
    let mut comment: Vec<&str> = Vec::new();
    for l in lines.iter().rev() {
        let t = l.trim_start();
        if t.starts_with('@') && comment.is_empty() {
            continue;
        }
        match t.strip_prefix("---") {
            Some(rest) if !rest.starts_with('-') => {
                comment.push(rest.strip_prefix(' ').unwrap_or(rest));
            }
            _ => break,
        }
    }
    if comment.is_empty() {
        return None;
    }
    comment.reverse();
    Some(comment.join("\n"))
}

/// A reply: its result, or an error the editor shows (the protocol's
/// `RequestFailed`).
fn respond(
    out: &mut impl std::io::Write,
    id: &Json,
    answer: Result<Json, String>,
) -> std::io::Result<()> {
    match answer {
        Ok(result) => reply(out, id, result),
        Err(message) => transport::write(
            out,
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32803, "message": message },
            }),
        ),
    }
}

/// The answer to a references or rename request (the new name `new`) from
/// what its check found: the declarations (when asked) and references as
/// locations; the renamed names' edits, or why there are none.
fn found_answer(
    server: &Server,
    want: Seek,
    new: Option<&str>,
    found: Option<refs::Found>,
) -> Result<Json, String> {
    let location =
        |p: &refs::Place| json!({ "uri": server.uri_for(&p.path), "range": p.lsp_range() });
    match want {
        Seek::References { declarations } => {
            let Some(f) = found else {
                return Ok(Json::Array(Vec::new()));
            };
            let mut list: Vec<Json> = Vec::new();
            if declarations {
                list.extend(f.declarations.iter().map(location));
            }
            list.extend(f.references.iter().map(location));
            Ok(Json::Array(list))
        }
        Seek::Rename => match found.and_then(|f| f.rename) {
            None => Err("wowlua found nothing here to rename".into()),
            Some(Err(why)) => Err(why),
            Some(Ok(edits)) => {
                let mut changes = serde_json::Map::new();
                for e in &edits {
                    let edit =
                        json!({ "range": e.lsp_range(), "newText": new.unwrap_or_default() });
                    if let Some(list) = changes
                        .entry(server.uri_for(&e.path))
                        .or_insert_with(|| Json::Array(Vec::new()))
                        .as_array_mut()
                    {
                        list.push(edit);
                    }
                }
                Ok(json!({ "changes": changes }))
            }
        },
    }
}

/// Whether the name at `position` can be renamed, as far as the document
/// and its package's last check tell: its range and text; `null` where no
/// name is; an error for a keyword, `self`, and what wowlua declares (the
/// rename's own check decides the rest).
fn prepare_rename(server: &Server, uri: &str, position: &Json) -> Result<Json, String> {
    let Some(doc) = server.docs.get(uri) else {
        return Ok(Json::Null);
    };
    let offset = text::offset(&doc.text, position);
    let is_name = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let bytes = doc.text.as_bytes();
    let start = (0..offset.min(bytes.len()))
        .rev()
        .take_while(|&i| is_name(bytes[i]))
        .last()
        .unwrap_or(offset);
    let end = (offset..bytes.len())
        .take_while(|&i| is_name(bytes[i]))
        .last()
        .map_or(offset, |i| i + 1);
    let word = &doc.text[start..end];
    if word.is_empty() || word.starts_with(|c: char| c.is_ascii_digit()) {
        return Ok(Json::Null);
    }
    if refs::RESERVED.contains(&word) {
        return Err(format!("`{word}` is a keyword"));
    }
    if matches!(word, "self" | "super") {
        return Err(format!(
            "`{word}` is part of the language and can't be renamed"
        ));
    }
    let checked = !doc.manifest.as_ref().is_some_and(|m| server.pending(m));
    let builtin = format!(
        "{}/",
        text::uri_of(&check::builtin_dir()).trim_end_matches('/')
    );
    let declared_by_wowlua = checked
        && server
            .definitions
            .get(&doc.canonical)
            .into_iter()
            .flatten()
            .filter(|d| d.start as usize <= offset && offset <= d.end as usize)
            .any(|d| d.target.uri.starts_with(&builtin));
    if declared_by_wowlua {
        return Err(format!(
            "`{word}` is declared by wowlua (the game's API, Lua's library or the standard library) and can't be renamed"
        ));
    }
    Ok(json!({ "range": text::range(&doc.text, start, end), "placeholder": word }))
}

/// Where the name at `position` of the document is defined, from its
/// package's last check: the protocol's locations, or `null`.
fn definition(server: &Server, uri: &str, position: &Json) -> Json {
    let Some(doc) = server.docs.get(uri) else {
        return Json::Null;
    };
    let offset = text::offset(&doc.text, position);
    let found: Vec<Json> = server
        .definitions
        .get(&doc.canonical)
        .into_iter()
        .flatten()
        // (the cursor just after a name is on it too)
        .filter(|d| d.start as usize <= offset && offset <= d.end as usize)
        .map(|d| d.target.location())
        .collect();
    if found.is_empty() {
        Json::Null
    } else {
        Json::Array(found)
    }
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
        .and_then(|m| wowlua_driver::project::load_manifest(&mut db, &m).manifest)
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

#[cfg(test)]
mod tests {
    use super::generated;
    use std::path::Path;

    #[test]
    fn build_output_and_hidden_folders_of_a_package_are_generated() {
        let m = Path::new("/home/u/.projects/addon/wowlua.json");
        assert!(!generated(
            m,
            Path::new("/home/u/.projects/addon/src/a.wowlua")
        ));
        assert!(generated(
            m,
            Path::new("/home/u/.projects/addon/dist/camelot/a.lua")
        ));
        assert!(generated(
            m,
            Path::new("/home/u/.projects/addon/.git/index")
        ));
        assert!(!generated(m, Path::new("/elsewhere/dist/a.lua")));
    }
}
