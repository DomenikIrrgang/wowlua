//! What the core needs of wowlua-engine's `globalsinfo`.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Vm};

use crate::hooks;

pub fn get_num_languages(vm: &mut Vm, c: NativeCall) -> Ret {
    (hooks::get().get_num_languages)(vm, c)
}
