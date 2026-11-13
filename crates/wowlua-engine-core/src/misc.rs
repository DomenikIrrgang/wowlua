//! What the core needs of wowlua-engine's `misc`.

use wowlua_vm::Vm;

use crate::hooks;

pub fn file_id_of_path(vm: &mut Vm, path: &str) -> Option<f64> {
    (hooks::get().file_id_of_path)(vm, path)
}
