//! What the core needs of wowlua-engine's `leftovers2`.

use std::rc::Rc;

use wowlua_vm::{NativeCall, Value, Vm};

use crate::engine;

/// What the engine answers for the client function `name` called with
/// `args` (its behaviour; none: nothing), secret answers unwrapped (as
/// `extras::ask`).
pub fn ask(vm: &mut Vm, name: &'static str, args: &[Value]) -> Vec<Value> {
    let Some(f) = engine(vm).behaviour(name) else {
        return vec![];
    };
    let full = vm.str(name);
    let call = NativeCall {
        args: args.iter().copied().collect(),
        taints: args.iter().map(|_| Default::default()).collect(),
        origins: args.iter().map(|_| Default::default()).collect(),
        upvals: Rc::from([full]),
        upval_taints: Rc::from([Default::default()]),
    };
    match f(vm, call) {
        Ok(values) => values.into_iter().map(|v| vm.hidden(v)).collect(),
        Err(_) => vec![],
    }
}
