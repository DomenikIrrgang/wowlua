//! The frame tree a front-end shows (spec debugger §18.12.1,
//! `wowlua/frameTree`).

use wowlua_vm::Vm;

/// The default tree starts at the frames without a parent, UIParent
/// among them though the engine made it (not `CreateFrame`): a frame made
/// on UIParent is under it (decision T359).
#[test]
fn the_default_tree_holds_uiparents_frames() {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    let f = vm
        .load_as(
            b"local f = CreateFrame('Frame', 'TreeDemo', UIParent) f:SetSize(10, 10) f:SetPoint('CENTER')",
            "@MyAddon/main.lua",
            "MyAddon",
        )
        .expect("compiles");
    vm.call(f, &[]).expect("runs");
    let tree = wowlua_debug::ui::frame_tree(&mut vm, None, false).expect("a tree");
    let roots = tree.as_array().expect("a list");
    let ui_parent = roots
        .iter()
        .find(|n| n["name"] == "UIParent")
        .expect("UIParent among the roots");
    let children = ui_parent["children"]
        .as_array()
        .expect("UIParent's children");
    assert!(
        children.iter().any(|n| n["name"] == "TreeDemo"),
        "{ui_parent:#}"
    );
    // The client's frames are marked Blizzard's, the addon's not (T496).
    assert_eq!(ui_parent["blizzard"], true);
    let demo = children.iter().find(|n| n["name"] == "TreeDemo").unwrap();
    assert!(demo.get("blizzard").is_none(), "{demo:#}");
}
