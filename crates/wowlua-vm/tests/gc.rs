//! The collector: garbage goes, reachable values stay, weak tables clear.

use std::cell::RefCell;
use std::rc::Rc;

use wowlua_vm::Vm;

fn run(vm: &mut Vm, src: &str) {
    let f = vm.load(src.as_bytes(), "=gc").expect("compiles");
    vm.call(f, &[]).expect("runs");
}

#[test]
fn garbage_is_collected_and_live_values_stay() {
    let mut vm = Vm::new();
    run(
        &mut vm,
        "keep = {} for i = 1, 100 do keep[i] = {i} end for i = 1, 5000 do local t = {i, tostring(i) .. 'x'} end",
    );
    let before = vm.heap.live_objects();
    vm.collect().expect("no finalizer errors");
    let after = vm.heap.live_objects();
    assert!(after < before, "{before} -> {after}");
    let out = Rc::new(RefCell::new(String::new()));
    let sink = out.clone();
    vm.set_output(move |b| sink.borrow_mut().push_str(&String::from_utf8_lossy(b)));
    run(
        &mut vm,
        "local s = 0 for i = 1, 100 do s = s + keep[i][1] end print(s)",
    );
    assert_eq!(out.borrow().trim(), "5050");
}

#[test]
fn weak_tables_lose_unreachable_entries() {
    let mut vm = Vm::new();
    let out = Rc::new(RefCell::new(String::new()));
    let sink = out.clone();
    vm.set_output(move |b| sink.borrow_mut().push_str(&String::from_utf8_lossy(b)));
    run(
        &mut vm,
        "weak = setmetatable({}, {__mode = 'k'}) strong = {} \
         weak[strong] = 1 weak[{}] = 2 weak['str'] = 3 \
         vweak = setmetatable({}, {__mode = 'v'}) vweak[1] = {} vweak[2] = strong vweak[3] = 'kept'",
    );
    vm.collect().expect("no finalizer errors");
    run(
        &mut vm,
        "local n = 0 for k in pairs(weak) do n = n + 1 end print(n, weak[strong], weak.str) \
         print(vweak[1], vweak[2] == strong, vweak[3])",
    );
    assert_eq!(out.borrow().as_str(), "2\t1\t3\nnil\ttrue\tkept\n");
}

#[test]
fn closures_keep_their_upvalues() {
    let mut vm = Vm::new();
    let out = Rc::new(RefCell::new(String::new()));
    let sink = out.clone();
    vm.set_output(move |b| sink.borrow_mut().push_str(&String::from_utf8_lossy(b)));
    // The table is reachable only through the closure's closed upvalue.
    run(
        &mut vm,
        "do local t = {value = 42} getter = function() return t.value end end",
    );
    for _ in 0..3 {
        run(&mut vm, "for i = 1, 2000 do local g = {i} end");
        vm.collect().expect("no finalizer errors");
    }
    run(&mut vm, "print(getter())");
    assert_eq!(out.borrow().trim(), "42");
}

#[test]
fn finalizers_run_once() {
    let mut vm = Vm::new();
    let out = Rc::new(RefCell::new(String::new()));
    let sink = out.clone();
    vm.set_output(move |b| sink.borrow_mut().push_str(&String::from_utf8_lossy(b)));
    run(
        &mut vm,
        "finalized = 0 do local p = newproxy(true) getmetatable(p).__gc = function() finalized = finalized + 1 end end",
    );
    vm.collect().expect("no finalizer errors");
    vm.collect().expect("no finalizer errors");
    run(&mut vm, "print(finalized)");
    assert_eq!(out.borrow().trim(), "1");
}

#[test]
fn automatic_collection_keeps_running_state() {
    let mut vm = Vm::new();
    let out = Rc::new(RefCell::new(String::new()));
    let sink = out.clone();
    vm.set_output(move |b| sink.borrow_mut().push_str(&String::from_utf8_lossy(b)));
    // Enough garbage to trigger several collections at safe points, while
    // a coroutine is suspended with locals and closures depend on upvalues.
    run(
        &mut vm,
        "local co = coroutine.create(function(n) local acc = {} for i = 1, n do acc[i] = {i} local got = coroutine.yield(#acc) end return acc end) \
         local counter = 0 local function bump() counter = counter + 1 return counter end \
         coroutine.resume(co, 50) \
         for round = 1, 49 do \
           for j = 1, 2000 do local garbage = {j, tostring(j) .. 'g', function() return j end} end \
           coroutine.resume(co, round) bump() \
         end \
         local ok, acc = coroutine.resume(co) \
         local sum = 0 for _, v in ipairs(acc) do sum = sum + v[1] end \
         print(ok, #acc, sum, counter, collectgarbage('count') > 0)",
    );
    assert_eq!(out.borrow().trim(), "true\t50\t1275\t49\ttrue");
    assert!(vm.gc_cycles >= 2, "collections: {}", vm.gc_cycles);
}
