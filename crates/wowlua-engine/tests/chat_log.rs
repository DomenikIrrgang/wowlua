//! A session's chat (decision T514): what the chat windows show is kept,
//! Blizzard's windows (a Lua mixin's `AddMessage`) hooked at login, the
//! engine's own message frames as they add, each line once.

use wowlua_vm::Vm;

fn vm() -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
    vm
}

fn run(vm: &mut Vm, src: &str) {
    let f = vm.load(src.as_bytes(), "@Test.lua").expect("compiles");
    vm.call(f, &[]).expect("runs");
}

fn lines(vm: &Vm) -> Vec<(String, String, Option<[f64; 3]>)> {
    wowlua_engine::engine(vm)
        .world
        .chat_log
        .iter()
        .map(|l| (l.frame.clone(), l.text.clone(), l.color))
        .collect()
}

#[test]
fn a_lua_chat_window_is_logged_from_the_login_and_an_engine_one_once() {
    let mut vm = vm();
    run(
        &mut vm,
        "-- Blizzard's kind: AddMessage in Lua, drawing its own lines
local lua = CreateFrame('Frame', 'ChatFrame2', UIParent)
lua.AddMessage = function(self, text, r, g, b) self.last = text end
-- the engine's kind
local native = CreateFrame('MessageFrame', 'ChatFrame3', UIParent)
native:SetFont('Fonts\\\\ARIALN.TTF', 14, '')
native:AddMessage('before the login', 1, 1, 1)
mock.fireEvent('PLAYER_LOGIN')
ChatFrame2:AddMessage('Welcome to World of Warcraft!', 1, 1, 0)
native:AddMessage('after the login', 0.5, 0.5, 1)
-- (a frame that is no chat window)
local other = CreateFrame('MessageFrame', 'MyMessages', UIParent)
other:SetFont('Fonts\\\\ARIALN.TTF', 14, '')
other:AddMessage('not chat', 1, 1, 1)",
    );
    assert_eq!(
        lines(&vm),
        [
            (
                "ChatFrame3".to_owned(),
                "before the login".to_owned(),
                Some([1.0, 1.0, 1.0])
            ),
            (
                "ChatFrame2".to_owned(),
                "Welcome to World of Warcraft!".to_owned(),
                Some([1.0, 1.0, 0.0])
            ),
            (
                "ChatFrame3".to_owned(),
                "after the login".to_owned(),
                Some([0.5, 0.5, 1.0])
            ),
        ]
    );
}
