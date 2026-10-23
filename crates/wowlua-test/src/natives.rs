//! The runner's part of `wowlua:mock`'s engine: what `print` printed, the
//! errors that reached the error handler, logging in and secrets. Each test
//! file runs on its own thread, so the state is the thread's.

use std::cell::RefCell;
use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, TableRef, Value, Vm};

#[derive(Default)]
struct State {
    printed: Vec<u8>,
    errors: Vec<String>,
    /// Where each error was raised (its stack), for the runner's reports.
    stacks: Vec<String>,
    /// How many of `errors` a test has read (`mock.errors()`) or the runner
    /// reported.
    read: usize,
    /// Where the running test's errors start in `errors` (`mock.errors()`
    /// lists them, not the file's earlier tests').
    test_start: usize,
    /// The errors the client drops without a report (`secureexecuterange`'s
    /// callbacks), with their stacks: no test sees them (`mock.errors()` is
    /// what reached the error handler), the runner fails on them.
    dropped: Vec<String>,
    dropped_read: usize,
    /// How many of the engine's refusals of protected actions a test has
    /// read (`mock.refusals()`, `toBeBlocked`) or the runner reported.
    refusals_read: usize,
    addon: String,
    /// The SavedVariables a reload keeps, set at the next login.
    saved: Option<Vec<(String, crate::saved::Saved)>>,
    has_bindings: bool,
}

/// Where a run's output goes as it happens.
pub type Tee = Box<dyn FnMut(Output)>;

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
    static TEE: RefCell<Option<Tee>> = RefCell::default();
}

/// Something a run printed or an error it raised, as it happens.
#[derive(Clone, Debug)]
pub enum Output {
    Printed(String),
    Error { message: String, stack: String },
}

/// Sends what this thread's runs print and the errors they raise to `f`
/// too (`None`: no more).
pub fn set_tee(f: Option<Tee>) {
    TEE.with(|t| *t.borrow_mut() = f);
}

fn tee(o: Output) {
    TEE.with(|t| {
        if let Some(f) = t.borrow_mut().as_mut() {
            f(o);
        }
    });
}

pub fn reset() {
    STATE.with(|s| *s.borrow_mut() = State::default());
}

pub fn print_sink(bytes: &[u8]) {
    STATE.with(|s| s.borrow_mut().printed.extend_from_slice(bytes));
    tee(Output::Printed(String::from_utf8_lossy(bytes).into_owned()));
}

pub fn take_printed() -> String {
    STATE.with(|s| {
        String::from_utf8_lossy(&std::mem::take(&mut s.borrow_mut().printed)).into_owned()
    })
}

/// The errors since the last read, marked read: each with where it was
/// raised.
pub fn unread_errors() -> Vec<String> {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let unread: Vec<String> = (s.read..s.errors.len())
            .map(|i| match s.stacks.get(i) {
                Some(stack) if !stack.is_empty() => format!("{}\n{stack}", s.errors[i]),
                _ => s.errors[i].clone(),
            })
            .collect();
        s.read = s.errors.len();
        unread
    })
}

/// The errors the client dropped since the last read, marked read.
pub fn unread_dropped() -> Vec<String> {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let unread = s.dropped[s.dropped_read..].to_vec();
        s.dropped_read = s.dropped.len();
        unread
    })
}

/// `Vm::on_dropped_error`: an error the client drops, for the runner.
pub fn dropped_error(_vm: &mut Vm, message: String, stack: String) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let text = format!("{}{message}", wowlua_engine::ui::DROPPED);
        s.dropped.push(if stack.is_empty() {
            text
        } else {
            format!("{text}\n{stack}")
        });
    });
}

/// The error handler: records the error (with the stack of where it was
/// raised) and returns it, as the default handler does.
pub fn error_handler(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = c.arg(1);
    let message = match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        other => match vm.tostring(other) {
            Ok(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            _ => "?".into(),
        },
    };
    let stack = vm.traceback(1);
    tee(Output::Error {
        message: message.clone(),
        stack: stack.clone(),
    });
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.errors.push(message);
        s.stacks.push(stack);
    });
    Ok(vec![v])
}

/// A test begins: `mock.errors()` lists the errors from here on.
pub fn begin_test() {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.test_start = s.errors.len();
    });
}

/// `mock.errors()`: the running test's errors so far (outside a test, the
/// file's), all marked read.
fn errors(vm: &mut Vm, _c: NativeCall) -> Ret {
    let list = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.read = s.errors.len();
        s.errors[s.test_start.min(s.errors.len())..].to_vec()
    });
    let t = vm.new_table();
    for (i, e) in list.iter().enumerate() {
        let v = vm.str(e);
        let _ = vm.heap.table_mut(t).set(Value::Num(i as f64 + 1.0), v);
    }
    Ok(vec![Value::Table(t)])
}

/// What `print` shows, as Blizzard's print handler joins it: each value's
/// `tostring`, separated by spaces.
fn record_print(vm: &mut Vm, c: NativeCall) -> Ret {
    let mut parts = Vec::new();
    for &v in &c.args {
        let s = vm.tostring(v)?;
        let s = vm.hidden(s);
        parts.push(match s {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            _ => "?".into(),
        });
    }
    let line = parts.join(" ");
    tee(Output::Printed(format!("{line}\n")));
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.printed.extend_from_slice(line.as_bytes());
        s.printed.push(b'\n');
    });
    Ok(vec![])
}

/// What the chat frame shows (`mock.printed()`): with Blizzard's UI code,
/// every line `DEFAULT_CHAT_FRAME:AddMessage` gets (`print` too, through
/// its print handler); without it, `print`'s lines.
pub fn install_print(vm: &mut Vm) {
    let record = vm.native("print", record_print);
    let src = b"local record = ...
if DEFAULT_CHAT_FRAME and hooksecurefunc then
    hooksecurefunc(DEFAULT_CHAT_FRAME, 'AddMessage', function(_, text) record(text) end)
else
    print = function(...) record(...) end
end";
    if let Ok(f) = vm.load(src, "=[wowlua test]") {
        let _ = vm.call(f, &[record]);
    }
}

/// `mock.refusals()`: every refused protected action so far, all marked
/// read.
fn refusals(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = wowlua_engine::engine(vm).refusals.len();
    STATE.with(|s| s.borrow_mut().refusals_read = n);
    wowlua_engine::protection::refusals_control(vm, c)
}

/// The refusals since the last read, marked read: (action, taint chain).
pub fn unread_refusals(vm: &Vm) -> Vec<(String, String)> {
    let all = &wowlua_engine::engine(vm).refusals;
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let from = s.refusals_read.min(all.len());
        s.refusals_read = all.len();
        all[from..]
            .iter()
            .map(|r| (r.action.clone(), r.report.clone()))
            .collect()
    })
}

/// `mock.printed()`: the lines printed so far.
fn printed(vm: &mut Vm, _c: NativeCall) -> Ret {
    let text = STATE.with(|s| String::from_utf8_lossy(&s.borrow().printed).into_owned());
    let t = vm.new_table();
    for (i, line) in text.lines().enumerate() {
        let v = vm.str(line);
        let _ = vm.heap.table_mut(t).set(Value::Num(i as f64 + 1.0), v);
    }
    Ok(vec![Value::Table(t)])
}

/// `mock.secret(v)`.
fn secret(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = c.arg(1);
    Ok(vec![vm.secret_wrap(v)])
}

/// The client's login: `ADDON_LOADED` for the addon, `PLAYER_LOGIN`,
/// `PLAYER_ENTERING_WORLD` (an initial login).
/// The SavedVariables the next login sets (after `mock.reloadUI`).
pub fn keep_saved(saved: Vec<(String, crate::saved::Saved)>) {
    STATE.with(|s| s.borrow_mut().saved = Some(saved));
}

/// Forgets SavedVariables no login took.
pub fn drop_saved() {
    STATE.with(|s| s.borrow_mut().saved = None);
}

/// Logging in: the SavedVariables a reload kept set before `ADDON_LOADED`,
/// then the client's login (`wowlua_engine::login`).
pub fn login(vm: &mut Vm, addon: &str) {
    let saved = STATE.with(|s| s.borrow_mut().saved.take());
    let reloading = saved.is_some();
    if let Some(saved) = &saved {
        crate::saved::restore(vm, saved);
    }
    let has_bindings = STATE.with(|s| s.borrow().has_bindings);
    wowlua_engine::login::login(vm, addon, has_bindings, reloading);
}

/// Whether the addon has a `Bindings.xml` (`ADDON_LOADED`'s second value).
pub fn set_has_bindings(has: bool) {
    STATE.with(|s| s.borrow_mut().has_bindings = has);
}

fn login_control(vm: &mut Vm, _c: NativeCall) -> Ret {
    let addon = STATE.with(|s| s.borrow().addon.clone());
    login(vm, &addon);
    Ok(vec![])
}

/// Adds the runner's controls to the engine's.
pub fn extend(vm: &mut Vm, controls: TableRef, addon: &str) {
    STATE.with(|s| s.borrow_mut().addon = addon.to_string());
    for (key, f) in [
        ("errors", errors as wowlua_vm::NativeFn),
        ("printed", printed),
        ("secret", secret),
        ("login", login_control),
        ("snapshot", crate::snapshots::snapshot_native),
        ("refusals", refusals),
    ] {
        let native = vm.native(key, f);
        let k = vm.heap.intern(key.as_bytes());
        vm.heap.table_mut(controls).set_str(k, native);
    }
}
