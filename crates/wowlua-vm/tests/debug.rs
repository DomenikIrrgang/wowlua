//! Debug hooks and the profiler (spec debugger §18.2, §18.8).

use std::cell::RefCell;
use std::rc::Rc;

use wowlua_vm::debug::{Debugger, LineCause, Step};
use wowlua_vm::heap::FuncRef;
use wowlua_vm::{LuaError, Value, Vm};

#[derive(Default)]
struct Log {
    events: Vec<String>,
}

type OnLine = Box<dyn FnMut(&mut Vm, u32, LineCause)>;

/// Records what the hooks report; `on_line` decides what to do at a line.
struct Recorder {
    log: Rc<RefCell<Log>>,
    on_line: OnLine,
}

impl Debugger for Recorder {
    fn line(&mut self, vm: &mut Vm, line: u32, cause: LineCause) {
        let tag = match cause {
            LineCause::Line => "line",
            LineCause::FunctionEntry => "entry",
        };
        self.log.borrow_mut().events.push(format!("{tag} {line}"));
        (self.on_line)(vm, line, cause);
    }
    fn api_call(&mut self, vm: &mut Vm, f: FuncRef) {
        let name = vm.native_name(f);
        self.log.borrow_mut().events.push(format!("api {name}"));
    }
    fn error(&mut self, vm: &mut Vm, error: &LuaError, caught: bool) {
        let msg = String::from_utf8_lossy(&vm.raw_tostring(error.value)).into_owned();
        self.log
            .borrow_mut()
            .events
            .push(format!("error {caught} {msg}"));
    }
    fn interrupted(&mut self, vm: &mut Vm) {
        self.log.borrow_mut().events.push("interrupted".into());
        vm.abort();
    }
}

fn attach(vm: &mut Vm, on_line: impl FnMut(&mut Vm, u32, LineCause) + 'static) -> Rc<RefCell<Log>> {
    let log = Rc::new(RefCell::new(Log::default()));
    vm.attach_debugger(Box::new(Recorder {
        log: log.clone(),
        on_line: Box::new(on_line),
    }));
    log
}

fn load(vm: &mut Vm, src: &str) -> Value {
    vm.load(src.as_bytes(), "=t").expect("compiles")
}

fn proto_of(vm: &Vm, f: Value) -> Rc<wowlua_vm::proto::Proto> {
    let Value::Func(f) = f else {
        panic!("a function")
    };
    vm.func_proto(f).expect("a Lua function")
}

fn events(log: &Rc<RefCell<Log>>) -> Vec<String> {
    log.borrow().events.clone()
}

const TWO_FUNCTIONS: &str = "local function f(x)\n\
    local y = x + 1\n\
    return y\n\
end\n\
local function g(x)\n\
    return f(x) * 2\n\
end\n\
return g(1) + g(2)\n";

#[test]
fn line_events_come_only_from_marked_functions() {
    let mut vm = Vm::new();
    let log = attach(&mut vm, |_, _, _| {});
    let chunk = load(&mut vm, TWO_FUNCTIONS);
    let proto = proto_of(&vm, chunk);
    // `f` (lines 1 to 4) has a breakpoint; `g` and the main chunk none.
    vm.mark_breakpoints(&proto, &mut |p| p.line_defined == 1);
    let r = vm.call(chunk, &[]).expect("runs");
    assert!(matches!(r[0], Value::Num(n) if n == 10.0));
    assert_eq!(events(&log), ["line 2", "line 3", "line 2", "line 3"]);
}

#[test]
fn a_loop_reports_its_line_each_time_round() {
    let mut vm = Vm::new();
    let log = attach(&mut vm, |_, _, _| {});
    let chunk = load(
        &mut vm,
        "local s = 0\nfor i = 1, 3 do\n  s = s + i\nend\nreturn s\n",
    );
    let proto = proto_of(&vm, chunk);
    vm.mark_breakpoints(&proto, &mut |_| true);
    vm.call(chunk, &[]).expect("runs");
    let lines = events(&log);
    assert_eq!(
        lines.iter().filter(|l| *l == "line 3").count(),
        3,
        "{lines:?}"
    );
    assert_eq!(lines.first().map(String::as_str), Some("line 1"));
    assert_eq!(lines.last().map(String::as_str), Some("line 5"));
}

#[test]
fn step_over_stays_in_the_function() {
    let mut vm = Vm::new();
    let started = Rc::new(RefCell::new(false));
    let s = started.clone();
    let log = attach(&mut vm, move |vm, _, _| {
        if !*s.borrow() {
            *s.borrow_mut() = true;
            // Step over from the main chunk: lines of its depth or less.
            let (thread, depth) = vm.depth();
            vm.mark_breakpoints(&vm_proto(vm), &mut |_| false);
            vm.set_step(Some(Step {
                thread: Some(thread),
                max_depth: depth,
            }));
        }
    });
    let chunk = load(&mut vm, TWO_FUNCTIONS);
    let proto = proto_of(&vm, chunk);
    vm.mark_breakpoints(&proto, &mut |p| p.line_defined == 0);
    vm.call(chunk, &[]).expect("runs");
    // The main chunk's lines only: 1 (`f`), 5 (`g`), 8 (the return).
    assert_eq!(events(&log), ["line 4", "line 7", "line 8"]);
}

/// The running frame's prototype.
fn vm_proto(vm: &Vm) -> Rc<wowlua_vm::proto::Proto> {
    vm.lua_frames()[0].proto.clone()
}

#[test]
fn step_into_goes_into_calls() {
    let mut vm = Vm::new();
    let log = attach(&mut vm, |vm, _, _| {
        vm.set_step(Some(Step {
            thread: None,
            max_depth: usize::MAX,
        }));
    });
    let chunk = load(&mut vm, TWO_FUNCTIONS);
    let proto = proto_of(&vm, chunk);
    vm.mark_breakpoints(&proto, &mut |p| p.line_defined == 0);
    vm.call(chunk, &[]).expect("runs");
    let lines = events(&log);
    // g's line 6, then f's lines 2 and 3, for both calls.
    assert_eq!(
        lines.iter().filter(|l| *l == "line 2").count(),
        2,
        "{lines:?}"
    );
    assert_eq!(
        lines.iter().filter(|l| *l == "line 6").count(),
        2,
        "{lines:?}"
    );
}

#[test]
fn a_function_breakpoint_stops_at_its_first_line() {
    let mut vm = Vm::new();
    let log = attach(&mut vm, |_, _, _| {});
    let chunk = load(&mut vm, "function f(x)\n  return x\nend\nf(1)\nf(2)\n");
    vm.call(chunk, &[]).expect("defines f");
    let Value::Func(f) = vm.get_global("f") else {
        panic!("f")
    };
    vm.set_function_breakpoints([f]);
    let chunk = load(&mut vm, "f(3)\n");
    vm.call(chunk, &[]).expect("runs");
    assert_eq!(events(&log), ["entry 2"]);
}

#[test]
fn an_api_breakpoint_stops_before_the_native_runs() {
    let mut vm = Vm::new();
    vm.set_output(|_| {});
    let log = attach(&mut vm, |_, _, _| {});
    let Value::Func(p) = vm.get_global("print") else {
        panic!("print")
    };
    vm.set_function_breakpoints([p]);
    let chunk = load(&mut vm, "print('a')\nlocal t = tostring(1)\nprint(t)\n");
    vm.call(chunk, &[]).expect("runs");
    assert_eq!(events(&log), ["api print", "api print"]);
}

#[test]
fn errors_are_reported_once_with_whether_they_are_caught() {
    let mut vm = Vm::new();
    let log = attach(&mut vm, |_, _, _| {});
    let chunk = load(
        &mut vm,
        "local ok = pcall(function() error('inner', 0) end)\n\
         local function deep() error('outer', 0) end\n\
         local function mid() deep() end\n\
         mid()\n",
    );
    assert!(vm.call(chunk, &[]).is_err());
    assert_eq!(events(&log), ["error true inner", "error false outer"]);
}

#[test]
fn an_error_in_a_coroutine_is_caught_by_resume() {
    let mut vm = Vm::new();
    let log = attach(&mut vm, |_, _, _| {});
    let chunk = load(
        &mut vm,
        "local co = coroutine.create(function() error('in co', 0) end)\n\
         return coroutine.resume(co)\n",
    );
    vm.call(chunk, &[]).expect("runs");
    assert_eq!(events(&log), ["error true in co"]);
}

#[test]
fn the_interrupt_flag_stops_a_busy_loop() {
    let mut vm = Vm::new();
    let log = attach(&mut vm, |_, _, _| {});
    let flag = vm.interrupt_handle().expect("attached");
    flag.store(true, std::sync::atomic::Ordering::Relaxed);
    let chunk = load(&mut vm, "while true do pcall(function() end) end\n");
    let err = vm.call(chunk, &[]).expect_err("aborted");
    assert!(vm.aborted());
    let msg = String::from_utf8_lossy(&vm.raw_tostring(err.value)).into_owned();
    assert_eq!(msg, "session ended");
    assert_eq!(events(&log), ["interrupted"]);
}

#[test]
fn without_hooks_nothing_is_counted() {
    let mut vm = Vm::new();
    let chunk = load(&mut vm, TWO_FUNCTIONS);
    vm.call(chunk, &[]).expect("runs");
    assert_eq!(vm.instructions(), 0);
}

fn clock() -> Box<dyn Fn() -> u64> {
    let t = std::cell::Cell::new(0u64);
    Box::new(move || {
        t.set(t.get() + 10);
        t.get()
    })
}

#[test]
fn the_profiler_counts_calls_and_instructions_per_function() {
    let mut vm = Vm::new();
    vm.start_profiling(clock());
    let chunk = load(
        &mut vm,
        "local function leaf(x) return x * 2 end\n\
         local function mid(x) local s = 0 for i = 1, 10 do s = s + leaf(i) end return s end\n\
         local t = {}\n\
         for i = 1, 5 do t[i] = mid(i) .. 'x' end\n",
    );
    vm.call(chunk, &[]).expect("runs");
    let p = vm.stop_profiling().expect("profiling");
    let flat = p.flat();
    let by_name = |n: &str| {
        flat.iter()
            .find(|e| p.key_name(e.key) == n)
            .unwrap_or_else(|| {
                panic!(
                    "{n} in {:?}",
                    flat.iter().map(|e| p.key_name(e.key)).collect::<Vec<_>>()
                )
            })
            .clone()
    };
    let leaf = by_name("function (t:1)");
    let mid = by_name("function (t:2)");
    let main = by_name("main chunk (t)");
    assert_eq!(leaf.calls, 50);
    assert_eq!(mid.calls, 5);
    assert!(leaf.self_cost.instructions > 0);
    assert_eq!(leaf.total, leaf.self_cost);
    assert!(mid.total.instructions > leaf.total.instructions);
    assert_eq!(main.total.instructions, vm.instructions());
    assert!(main.total.bytes > 0, "string concatenations allocate");
    assert!(main.total.nanos > 0);
}

#[test]
fn dispatches_are_nodes_of_their_own() {
    let mut vm = Vm::new();
    vm.start_profiling(clock());
    let chunk = load(
        &mut vm,
        "function handler() local x = 0 for i = 1, 5 do x = x + i end end\n",
    );
    vm.call(chunk, &[]).expect("defines");
    let handler = vm.get_global("handler");
    for _ in 0..3 {
        let d = vm.dispatch_enter(wowlua_vm::debug::DispatchKind::Event, || {
            "PLAYER_LOGIN".into()
        });
        vm.call(handler, &[]).expect("runs");
        vm.dispatch_exit(d);
    }
    let p = vm.stop_profiling().expect("profiling");
    let flat = p.flat();
    let event = flat
        .iter()
        .find(|e| p.key_name(e.key) == "event PLAYER_LOGIN")
        .expect("the event");
    assert_eq!(event.calls, 3);
    let handler = flat
        .iter()
        .find(|e| p.key_name(e.key) == "function (t:1)")
        .expect("the handler");
    assert_eq!(event.total.instructions, handler.total.instructions);
    // The handler's node is the event's child.
    let ev_node = p
        .nodes
        .iter()
        .position(|n| p.key_name(n.key) == "event PLAYER_LOGIN")
        .expect("node");
    assert_eq!(p.nodes[ev_node].children.len(), 1);
}

#[test]
fn an_error_unwinds_the_profilers_stack() {
    let mut vm = Vm::new();
    vm.start_profiling(clock());
    let chunk = load(
        &mut vm,
        "local function bad() error('x') end\n\
         local function after() return 1 end\n\
         for i = 1, 3 do pcall(bad) after() end\n",
    );
    vm.call(chunk, &[]).expect("runs");
    let p = vm.stop_profiling().expect("profiling");
    // `after` is the main chunk's child, not `bad`'s.
    let after = p
        .nodes
        .iter()
        .find(|n| p.key_name(n.key) == "function (t:2)")
        .expect("after");
    assert_eq!(
        p.key_name(p.nodes[after.parent as usize].key),
        "main chunk (t)"
    );
    assert_eq!(after.calls, 3);
}
