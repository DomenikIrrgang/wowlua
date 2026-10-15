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
    vm.collect();
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
    vm.collect();
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
        vm.collect();
    }
    run(&mut vm, "print(getter())");
    assert_eq!(out.borrow().trim(), "42");
}
