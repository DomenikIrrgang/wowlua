//! `wowlua lsp`: the language server (spec §16.2) over stdio. It shares the
//! compiler: a package is checked as `wowlua check` checks it, with the
//! editor's unsaved texts over the disk's, a moment after the last edit
//! and on every save, on a thread of its own; documents are formatted as
//! `wowlua fmt` formats them, outlined from their syntax, and their names
//! lead to their definitions from the last check. Completion checks the
//! text with a placeholder at the cursor (`complete`).

pub mod check;
pub mod complete;
pub mod symbols;
pub mod text;
pub mod transport;

use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

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
    dirty: BTreeMap<PathBuf, Instant>,
    /// Packages whose full check waits until no package was edited for
    /// `QUIET`: since when, and whether with their whole workspace (after a
    /// save or a change on disk).
    later: BTreeMap<PathBuf, (Instant, bool)>,
    /// The check the worker runs now.
    running: Option<Running>,
    /// Checks for the worker.
    jobs: mpsc::Sender<Job>,
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
}

/// A completion request: its id, document and package, and what to check.
struct Completing {
    id: Json,
    uri: String,
    manifest: PathBuf,
    asked: complete::Asked,
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
        let at = Instant::now()
            .checked_sub(early)
            .unwrap_or_else(Instant::now);
        self.dirty.insert(manifest.to_path_buf(), at);
        // (its full check waits for quiet again)
        let entry = self
            .later
            .entry(manifest.to_path_buf())
            .or_insert((Instant::now(), false));
        entry.0 = Instant::now();
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
    let mut server = Server {
        docs: HashMap::new(),
        dirty: BTreeMap::new(),
        later: BTreeMap::new(),
        running: None,
        jobs,
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
    };
    loop {
        // the next check to start: when its edits have settled, or a saved
        // workspace's once nothing was edited for a while
        let now = Instant::now();
        let wait = match (&server.running, server.dirty.values().min()) {
            (Some(_), _) => Duration::from_secs(3600),
            (None, Some(at)) => (*at + SETTLE).saturating_duration_since(now),
            (None, None) => server
                .later
                .values()
                .map(|l| l.0)
                .min()
                .map_or(Duration::from_secs(3600), |at| {
                    (at + QUIET).saturating_duration_since(now)
                }),
        };
        match inbox.recv_timeout(wait) {
            Ok(Event::Message(message)) => {
                let handled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    handle(&mut server, &message, out)
                }));
                match handled {
                    Ok(stop) => {
                        if stop? {
                            return Ok(server.shutdown);
                        }
                    }
                    Err(e) => {
                        // a request whose handler panicked still gets an answer
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
                    }
                }
            }
            Ok(Event::Checked(manifest, result)) => {
                let ran = server.running.take();
                // a completion check: its answer, nothing published
                if let Some(Running {
                    completion: Some((id, prefix)),
                    ..
                }) = ran
                {
                    let found = result
                        .ok()
                        .and_then(|packages| packages.into_iter().find_map(|c| c.completions));
                    let list = completion_list(&mut server, found, &prefix);
                    reply(out, &id, list)?;
                    start_due(&mut server);
                    continue;
                }
                match result {
                    Ok(packages) => {
                        for checked in packages {
                            publish(&mut server, checked, out)?;
                        }
                    }
                    Err(Failure::Failed(why)) => {
                        eprintln!("wowlua lsp: checking {} failed: {why}", manifest.display());
                        notify(
                            out,
                            "window/logMessage",
                            json!({ "type": 1, "message": format!("wowlua: checking {} failed: {why}", manifest.display()) }),
                        )?;
                    }
                }
                answer_waiting(&mut server, &manifest, out)?;
            }
            Ok(Event::Closed) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Ok(server.shutdown);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        start_due(&mut server);
    }
}

/// Starts the check of the package whose edits settled first, else, when
/// nothing was edited for `QUIET`, a saved package's workspace; unless a
/// check runs.
fn start_due(server: &mut Server) {
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
            };
            if server.jobs.send(job).is_ok() {
                server.running = Some(Running {
                    manifest: c.manifest,
                    focused: false,
                    completion: Some((c.id, c.asked.prefix)),
                });
            }
            return;
        }
    }
    let now = Instant::now();
    let edited = server
        .dirty
        .iter()
        .filter(|(_, at)| now.duration_since(**at) >= SETTLE)
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
                .filter(|(_, (at, _))| now.duration_since(*at) >= QUIET)
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
        };
        if server.jobs.send(job).is_ok() {
            server.running = Some(Running {
                manifest,
                focused,
                completion: None,
            });
        }
    }
}

/// The check the worker runs.
struct Running {
    manifest: PathBuf,
    /// The first pass of an edit (its open files only).
    focused: bool,
    /// A completion check: the request's id and the word typed.
    completion: Option<(Json, String)>,
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
}

/// Runs a check where `checker` says. A check that panics, crashes or
/// takes too long is an error the server reports, and goes on.
fn run_check(
    checker: &Checker,
    process: &mut Option<WorkerProcess>,
    job: &Job,
) -> Result<Vec<check::Checked>, Failure> {
    match checker {
        Checker::InProcess => {
            // (the open documents as the job has them, a completion's text
            // with its placeholder; the next job sets them again. Each set
            // alone: the crate's tests run servers side by side. A check
            // here runs to its end)
            for (path, text) in &job.docs {
                wowlua_driver::overlay::set(path, text.as_bytes().to_vec());
            }
            let open = job.docs.iter().map(|(p, _)| canonical(p)).collect();
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                check::check(
                    &job.manifest,
                    job.whole,
                    &open,
                    job.focus.as_ref(),
                    job.complete.as_deref(),
                )
            }))
            .map_err(|e| Failure::Failed(panic_message(&*e)))
        }
        Checker::Worker(program) => check_in_worker(program, process, job),
    }
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
    let request = check::request(
        &job.manifest,
        job.whole,
        &job.docs,
        job.focus.as_ref(),
        job.rescan,
        job.complete.as_deref(),
    );
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
    let mut warm = check::Warm::default();
    let stdout = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        let answer = match serde_json::from_str::<Json>(&line) {
            Ok(request) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                check::worker(&mut warm, &request)
            }))
            .unwrap_or_else(|e| {
                warm = check::Warm::default();
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
    wowlua_manifest::find_manifest(path.parent()?, None).map(|m| canonical(&m))
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
                "hoverProvider": true,
                "completionProvider": { "triggerCharacters": [".", ":", "\"", "'"], "resolveProvider": true },
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
        "textDocument/completion" => {
            let asked = server.docs.get(&uri).and_then(|d| {
                let offset = text::offset(&d.text, &params["position"]);
                Some((d.manifest.clone()?, complete::prepare(&d.text, offset)?))
            });
            match asked {
                None => reply(out, id, completion_list(server, None, ""))?,
                Some((manifest, asked)) => {
                    // (an older request still waiting is answered empty)
                    let next = Completing {
                        id: id.clone(),
                        uri,
                        manifest,
                        asked,
                    };
                    if let Some(old) = server.completing.replace(next) {
                        reply(out, &old.id, json!({ "isIncomplete": true, "items": [] }))?;
                    }
                }
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

/// A package's check to the client: its diagnostics (its files without any
/// and its open documents without any cleared), its definitions kept.
fn publish(
    server: &mut Server,
    checked: check::Checked,
    out: &mut impl std::io::Write,
) -> std::io::Result<()> {
    let manifest = checked.manifest.clone();
    let manifest = manifest.as_path();
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

/// The `---` comment lines right above line `line` of the file at `uri`
/// (decorators between them and it skipped), without their dashes.
fn doc_comment(server: &Server, uri: &str, line: usize) -> Option<String> {
    let owned;
    let text = match server.docs.get(uri) {
        Some(d) => d.text.as_str(),
        None => {
            owned = std::fs::read_to_string(text::path_of(uri)?).ok()?;
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
